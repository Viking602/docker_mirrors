use crate::config::RegistryConfig;
use reqwest::{Client, Response, header, RequestBuilder, Method};
use log::{error, info, warn};
use std::error::Error;
use bytes::Bytes;
use serde::Deserialize;
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
    #[serde(default)]
    access_token: String,
}

pub struct ProxyService {
    client: Client,
    config: RegistryConfig,
}

impl ProxyService {
    pub fn new(config: RegistryConfig) -> Self {
        // Create a client with custom settings
        let client = Client::builder()
            // Follow redirects automatically, but limit to 10 to prevent infinite loops
            .redirect(reqwest::redirect::Policy::limited(10))
            // Increase timeout for large blob downloads
            .timeout(std::time::Duration::from_secs(300)) // 5 minute timeout
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            config,
        }
    }

    // Parse WWW-Authenticate header to extract auth parameters
    fn parse_www_authenticate_header(&self, header_value: &str) -> Option<(String, HashMap<String, String>)> {
        // Example: Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/ubuntu:pull"
        let parts: Vec<&str> = header_value.splitn(2, ' ').collect();
        if parts.len() != 2 {
            return None;
        }

        let auth_type = parts[0].to_string();
        let params_str = parts[1];

        let mut params = HashMap::new();
        for param in params_str.split(',') {
            let kv: Vec<&str> = param.splitn(2, '=').collect();
            if kv.len() == 2 {
                let key = kv[0].trim().to_string();
                let value = kv[1].trim().trim_matches('"').to_string();
                params.insert(key, value);
            }
        }

        Some((auth_type, params))
    }

    // Get token from Docker Hub auth service
    async fn get_docker_hub_token(&self, realm: &str, service: &str, scope: &str) -> Result<String, Box<dyn Error>> {
        let mut url = format!("{}?service={}", realm, service);
        if !scope.is_empty() {
            url = format!("{}&scope={}", url, scope);
        }

        info!("Requesting token from: {}", url);

        let mut request_builder = self.client.get(&url);

        // Add basic auth if credentials are configured
        if self.config.docker_hub_credentials.is_configured() {
            if let (Some(username), Some(password)) = (&self.config.docker_hub_credentials.username, &self.config.docker_hub_credentials.password) {
                request_builder = request_builder.basic_auth(username, Some(password));
                info!("Using configured Docker Hub credentials for authentication");
            }
        } else {
            warn!("Docker Hub credentials not configured. Anonymous token request may have rate limits.");
        }

        let response = request_builder.send().await?;

        if !response.status().is_success() {
            return Err(format!("Failed to get token: {}", response.status()).into());
        }

        let token_response: TokenResponse = response.json().await?;

        // Use token or access_token depending on which one is available
        let token = if !token_response.token.is_empty() {
            token_response.token
        } else {
            token_response.access_token
        };

        Ok(token)
    }

    // Helper method to format path for Docker Hub
    fn format_docker_hub_path(&self, registry_key: &str, path: &str) -> String {
        if registry_key == "v2" {
            // For v2 API requests, the path is already relative to /v2
            // So we need to ensure it starts with /v2
            if path == "/" {
                // Special case for /v2/ API endpoint
                "/v2/".to_string()
            } else {
                format!("/v2{}", path)
            }
        } else if !path.starts_with("/v2") {
            // For regular Docker Hub requests, ensure the path starts with /v2
            if path.starts_with("/library/") {
                format!("/v2{}", path)
            } else if path.starts_with("/") && !path.starts_with("/v2/") {
                // Check if the path contains a namespace (e.g., /username/repo)
                if path.matches('/').count() >= 2 {
                    format!("/v2{}", path)
                } else {
                    format!("/v2/library{}", path)
                }
            } else {
                format!("/v2/{}", path)
            }
        } else {
            path.to_string()
        }
    }

    // Helper method to create a request builder based on method, URL, headers, and body
    fn create_request_builder(
        &self,
        method: &str,
        url: &str,
        headers: reqwest::header::HeaderMap,
        body: Option<&Bytes>,
    ) -> Result<RequestBuilder, Box<dyn Error>> {
        // Convert string method to reqwest::Method
        let method_enum = match Method::from_str(method) {
            Ok(m) => m,
            Err(_) => {
                error!("Unsupported HTTP method: {}", method);
                return Err(format!("Unsupported HTTP method: {}", method).into());
            }
        };

        // Check if it's a HEAD request before creating the request builder
        let is_head_request = method_enum == Method::HEAD;

        // Create request builder
        let mut request_builder = self.client.request(method_enum, url);

        // Add headers
        request_builder = request_builder.headers(headers);

        // Add body if present and it's not a HEAD request
        if let Some(b) = body {
            if !is_head_request {
                request_builder = request_builder.body(b.clone());
            }
        }

        Ok(request_builder)
    }

    // Helper method to prepare headers for a registry request
    fn prepare_headers(&self, headers: &reqwest::header::HeaderMap, registry_url: &str, is_blob_request: bool) -> reqwest::header::HeaderMap {
        let mut headers_clone = headers.clone();

        // Set the Host header to the registry URL
        if let Ok(host) = reqwest::header::HeaderValue::from_str(registry_url) {
            headers_clone.insert(reqwest::header::HOST, host);
        }

        // Add Docker registry API version header
        let api_version = reqwest::header::HeaderValue::from_static("registry/2.0");
        headers_clone.insert(
            reqwest::header::HeaderName::from_static("docker-distribution-api-version"),
            api_version,
        );

        // For Docker Hub, add User-Agent header to avoid rate limiting
        if registry_url == "registry-1.docker.io" {
            // Use a more specific User-Agent that mimics Docker client
            let user_agent = reqwest::header::HeaderValue::from_static("docker/20.10.12 go/go1.16.12 git-commit/459d0df kernel/5.10.47 os/linux arch/amd64 UpstreamClient(Docker-Client/20.10.12 \\(linux\\))");
            headers_clone.insert(reqwest::header::USER_AGENT, user_agent);

            // Add additional headers that Docker Hub might expect
            headers_clone.insert(
                reqwest::header::HeaderName::from_static("accept-encoding"),
                reqwest::header::HeaderValue::from_static("gzip")
            );

            // Add Connection: keep-alive header
            headers_clone.insert(
                header::CONNECTION,
                reqwest::header::HeaderValue::from_static("keep-alive")
            );

            // Add Cache-Control header
            headers_clone.insert(
                header::CACHE_CONTROL,
                reqwest::header::HeaderValue::from_static("max-age=0")
            );
        }

        // Set appropriate Accept header
        if is_blob_request {
            // For blob requests, accept octet-stream and other formats
            let accept = reqwest::header::HeaderValue::from_static("application/octet-stream, application/vnd.docker.image.rootfs.diff.tar.gzip, application/vnd.oci.image.layer.v1.tar+gzip");
            headers_clone.insert(header::ACCEPT, accept);
        } else if !headers_clone.contains_key(header::ACCEPT) {
            // For other requests, set a default Accept header if not present
            // Include all the formats that Docker client accepts
            let accept = reqwest::header::HeaderValue::from_static("application/json, application/vnd.docker.distribution.manifest.v2+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.oci.image.index.v1+json");
            headers_clone.insert(header::ACCEPT, accept);
        }

        // Log outgoing headers
        info!("Outgoing headers:");
        for (key, value) in headers_clone.iter() {
            info!("  {}: {}", key, value.to_str().unwrap_or_default());
        }

        headers_clone
    }

    // Helper method to handle blob requests without recursion
    async fn handle_blob_request(
        &self,
        initial_url: &str,
        method: &str,
        headers: reqwest::header::HeaderMap,
        body: Option<&Bytes>,
        token: Option<&str>
    ) -> Result<Response, Box<dyn Error>> {
        let mut current_url = initial_url.to_string();
        let mut current_headers = headers.clone();
        let mut redirect_count = 0;
        const MAX_REDIRECTS: usize = 10;

        // Add authorization header if token is provided
        if let Some(token_str) = token {
            let auth_value = format!("Bearer {}", token_str);
            if let Ok(auth_header) = header::HeaderValue::from_str(&auth_value) {
                current_headers.insert(header::AUTHORIZATION, auth_header);
            }
        }

        // Ensure Accept header is set for blob requests
        if !current_headers.contains_key(header::ACCEPT) {
            let accept = reqwest::header::HeaderValue::from_static("application/octet-stream");
            current_headers.insert(header::ACCEPT, accept);
        }

        // Add Range header to request partial content if not present
        // This can help with large blobs and timeouts
        if !current_headers.contains_key(header::RANGE) {
            let range = reqwest::header::HeaderValue::from_static("bytes=0-");
            current_headers.insert(header::RANGE, range);
        }

        // Add Cache-Control header to prevent caching of blob requests
        let cache_control = reqwest::header::HeaderValue::from_static("no-cache");
        current_headers.insert(header::CACHE_CONTROL, cache_control);

        // Add Connection: keep-alive header
        let connection = reqwest::header::HeaderValue::from_static("keep-alive");
        current_headers.insert(header::CONNECTION, connection);

        loop {
            // Create request builder
            let request_builder = self.create_request_builder(method, &current_url, current_headers.clone(), body)?;

            info!("Sending blob request to: {} {}", method, current_url);

            // Send request with longer timeout for blobs
            let response = request_builder
                .timeout(std::time::Duration::from_secs(300)) // 5 minute timeout for blob requests
                .send()
                .await?;

            info!("Received blob response: {} from {}", response.status(), current_url);

            // Log response headers for debugging
            info!("Blob response headers:");
            for (key, value) in response.headers() {
                info!("  {}: {}", key, value.to_str().unwrap_or_default());
            }

            // If we get a redirect, update the URL and try again
            if response.status().is_redirection() {
                if let Some(location) = response.headers().get(header::LOCATION) {
                    if let Ok(redirect_url) = location.to_str() {
                        info!("Following blob redirect to: {}", redirect_url);

                        // Increment redirect count and check if we've reached the limit
                        redirect_count += 1;
                        if redirect_count > MAX_REDIRECTS {
                            return Err(format!("Too many redirects ({})", redirect_count).into());
                        }

                        // Update URL for next iteration
                        current_url = redirect_url.to_string();
                        continue;
                    }
                }
            }

            // If we get a 401 Unauthorized, try to authenticate
            if response.status() == reqwest::StatusCode::UNAUTHORIZED && current_url.contains("registry-1.docker.io") {
                info!("Received 401 Unauthorized for blob request, attempting to authenticate");

                // Check if WWW-Authenticate header is present
                if let Some(www_auth) = response.headers().get(header::WWW_AUTHENTICATE) {
                    if let Ok(www_auth_str) = www_auth.to_str() {
                        info!("WWW-Authenticate header: {}", www_auth_str);

                        // Parse WWW-Authenticate header
                        if let Some((auth_type, params)) = self.parse_www_authenticate_header(www_auth_str) {
                            if auth_type.to_lowercase() == "bearer" {
                                let realm = params.get("realm").cloned().unwrap_or_default();
                                let service = params.get("service").cloned().unwrap_or_default();
                                let scope = params.get("scope").cloned().unwrap_or_default();

                                info!("Auth params - realm: {}, service: {}, scope: {}", realm, service, scope);

                                // Get token from Docker Hub auth service
                                match self.get_docker_hub_token(&realm, &service, &scope).await {
                                    Ok(new_token) => {
                                        info!("Successfully obtained new token, retrying request with authentication");

                                        // Add authorization header
                                        let auth_value = format!("Bearer {}", new_token);
                                        if let Ok(auth_header) = header::HeaderValue::from_str(&auth_value) {
                                            current_headers.insert(header::AUTHORIZATION, auth_header);
                                        }

                                        // Continue to next iteration to retry with new token
                                        continue;
                                    },
                                    Err(e) => {
                                        error!("Failed to get Docker Hub token: {}", e);
                                        // Continue with the original response
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // If we get a 403 Forbidden, it might be due to Docker Hub's rate limiting
            // Log more details and return a more informative error
            if response.status() == reqwest::StatusCode::FORBIDDEN {
                error!("Received 403 Forbidden for blob request. This might be due to Docker Hub rate limiting.");
                if let Some(retry_after) = response.headers().get(header::RETRY_AFTER) {
                    error!("Retry-After: {}", retry_after.to_str().unwrap_or_default());
                }

                // Check for Docker Hub specific headers
                for (key, value) in response.headers() {
                    if key.as_str().to_lowercase().contains("ratelimit") {
                        error!("Rate limit header - {}: {}", key, value.to_str().unwrap_or_default());
                    }
                }

                // If this is a Docker Hub request, try with a different approach
                if current_url.contains("registry-1.docker.io") {
                    // Extract the blob digest from the URL
                    if let Some(digest) = current_url.split("/blobs/").nth(1) {
                        // Try multiple CDN fallbacks
                        let cdn_fallbacks = [
                            // Primary Cloudflare CDN
                            format!("https://production.cloudflare.docker.com/registry-v2/docker/registry/v2/blobs/sha256/{}/{}/data", 
                                &digest[7..9], digest.replace(":", "/")),
                            // Alternative CDN format
                            format!("https://registry.hub.docker.com/v2/library/{}/blobs/{}", 
                                current_url.split("/v2/").nth(1).unwrap_or("").split("/blobs/").nth(0).unwrap_or("library/redis"),
                                digest),
                            // Another alternative format
                            format!("https://registry-cdn.docker.io/v2/library/{}/blobs/{}", 
                                current_url.split("/v2/").nth(1).unwrap_or("").split("/blobs/").nth(0).unwrap_or("library/redis"),
                                digest),
                        ];

                        for (i, cdn_url) in cdn_fallbacks.iter().enumerate() {
                            info!("Trying alternative CDN URL #{} for blob: {}", i+1, cdn_url);

                            // Create a new request to the CDN
                            let mut cdn_headers = reqwest::header::HeaderMap::new();

                            // Use different User-Agent for each attempt
                            let user_agents = [
                                "docker-registry-proxy",
                                "docker/20.10.12 go/go1.16.12",
                                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
                            ];

                            cdn_headers.insert(
                                header::USER_AGENT, 
                                reqwest::header::HeaderValue::from_static(user_agents[i % user_agents.len()])
                            );

                            // Add Accept header
                            cdn_headers.insert(
                                header::ACCEPT,
                                reqwest::header::HeaderValue::from_static("application/octet-stream, application/vnd.docker.image.rootfs.diff.tar.gzip")
                            );

                            // Try with exponential backoff
                            let mut retry_delay = 1;
                            let max_retries = 3;

                            for retry in 0..max_retries {
                                if retry > 0 {
                                    info!("Retry #{} for CDN URL #{} after {}s delay", retry, i+1, retry_delay);
                                    tokio::time::sleep(std::time::Duration::from_secs(retry_delay)).await;
                                    retry_delay *= 2; // Exponential backoff
                                }

                                let cdn_request = self.client.get(cdn_url)
                                    .headers(cdn_headers.clone())
                                    .timeout(std::time::Duration::from_secs(300));

                                match cdn_request.send().await {
                                    Ok(cdn_response) => {
                                        info!("CDN response: {} from {}", cdn_response.status(), cdn_url);

                                        // Log response headers for debugging
                                        info!("CDN response headers:");
                                        for (key, value) in cdn_response.headers() {
                                            info!("  {}: {}", key, value.to_str().unwrap_or_default());
                                        }

                                        if cdn_response.status().is_success() {
                                            return Ok(cdn_response);
                                        }

                                        // If we got a redirect, try to follow it manually
                                        if cdn_response.status().is_redirection() {
                                            if let Some(location) = cdn_response.headers().get(header::LOCATION) {
                                                if let Ok(redirect_url) = location.to_str() {
                                                    info!("Following CDN redirect to: {}", redirect_url);

                                                    let redirect_request = self.client.get(redirect_url)
                                                        .headers(cdn_headers.clone())
                                                        .timeout(std::time::Duration::from_secs(300));

                                                    match redirect_request.send().await {
                                                        Ok(redirect_response) => {
                                                            info!("Redirect response: {} from {}", redirect_response.status(), redirect_url);
                                                            if redirect_response.status().is_success() {
                                                                return Ok(redirect_response);
                                                            }
                                                        },
                                                        Err(e) => {
                                                            warn!("Failed to follow redirect: {}", e);
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        // No need to retry if we got a response, move to next CDN
                                        break;
                                    },
                                    Err(e) => {
                                        warn!("Failed to access CDN (attempt {}/{}): {}", retry+1, max_retries, e);
                                        // Continue with retry loop
                                    }
                                }
                            }
                        }

                        // Try direct download with different headers as last resort
                        info!("All CDN fallbacks failed, trying direct download with different headers");

                        let mut direct_headers = reqwest::header::HeaderMap::new();
                        direct_headers.insert(header::USER_AGENT, reqwest::header::HeaderValue::from_static("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"));
                        direct_headers.insert(header::ACCEPT, reqwest::header::HeaderValue::from_static("*/*"));
                        direct_headers.insert(header::ACCEPT_ENCODING, reqwest::header::HeaderValue::from_static("gzip, deflate, br"));

                        // Try to get a fresh token
                        let repo_path = current_url
                            .split("/v2/")
                            .nth(1)
                            .unwrap_or("library/redis")
                            .split("/blobs/")
                            .next()
                            .unwrap_or("library/redis");

                        let scope = format!("repository:{}:pull", repo_path);
                        let realm = "https://auth.docker.io/token";
                        let service = "registry.docker.io";

                        match self.get_docker_hub_token(realm, service, &scope).await {
                            Ok(token) => {
                                let auth_value = format!("Bearer {}", token);
                                if let Ok(auth_header) = header::HeaderValue::from_str(&auth_value) {
                                    direct_headers.insert(header::AUTHORIZATION, auth_header);
                                }
                            },
                            Err(e) => {
                                warn!("Failed to get fresh token for direct download: {}", e);
                            }
                        }

                        let direct_request = self.client.get(&current_url)
                            .headers(direct_headers)
                            .timeout(std::time::Duration::from_secs(300));

                        match direct_request.send().await {
                            Ok(direct_response) => {
                                info!("Direct response: {} from {}", direct_response.status(), current_url);
                                if direct_response.status().is_success() {
                                    return Ok(direct_response);
                                }
                            },
                            Err(e) => {
                                warn!("Failed direct download: {}", e);
                            }
                        }
                    }
                }
            }

            // If we're here, we either got a non-redirect response or couldn't extract the redirect URL
            return Ok(response);
        }
    }

    pub async fn forward_request(
        &self,
        registry_key: &str,
        path: &str,
        query: Option<&str>,
        headers: reqwest::header::HeaderMap,
        body: Option<Bytes>,
        method: &str,
    ) -> Result<Response, Box<dyn Error>> {
        // Special case for v2 API requests - forward to Docker Hub
        let registry_url = if registry_key == "v2" {
            info!("Detected v2 API request, forwarding to Docker Hub");
            "registry-1.docker.io"
        } else {
            match self.config.get_registry_url(registry_key) {
                Some(url) => url,
                None => {
                    error!("Unsupported registry: {}", registry_key);
                    return Err(format!("Unsupported registry: {}", registry_key).into());
                }
            }
        };

        // For Docker Hub, ensure the path is correctly formatted for the Docker Registry API V2
        let formatted_path = if registry_url == "registry-1.docker.io" {
            self.format_docker_hub_path(registry_key, path)
        } else {
            path.to_string()
        };

        let url = if let Some(q) = query {
            format!("https://{}{}{}", registry_url, formatted_path, q)
        } else {
            format!("https://{}{}", registry_url, formatted_path)
        };

        info!("Forwarding request to: {}", url);

        // Check if this is a blob request
        let is_blob_request = formatted_path.contains("/blobs/");

        // Prepare headers with blob request flag
        let headers_clone = self.prepare_headers(&headers, registry_url, is_blob_request);

        // Clone headers for potential auth retry
        let headers_for_auth = headers_clone.clone();

        // For non-blob requests or initial requests, use the standard flow
        if !is_blob_request {
            // Create request builder
            let request_builder = self.create_request_builder(method, &url, headers_clone, body.as_ref())?;

            // Log outgoing request details
            info!("Sending request to: {} {}", method, url);

            let response = request_builder.send().await?;
            info!("Received response: {} from {}", response.status(), url);

            // Handle Docker Hub authentication and errors
            if registry_url == "registry-1.docker.io" {
                if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                    info!("Received unauthorized response from Docker Hub, attempting to authenticate");

                    // Check if WWW-Authenticate header is present
                    if let Some(www_auth) = response.headers().get(header::WWW_AUTHENTICATE) {
                        if let Ok(www_auth_str) = www_auth.to_str() {
                            info!("WWW-Authenticate header: {}", www_auth_str);

                            // Parse WWW-Authenticate header
                            if let Some((auth_type, params)) = self.parse_www_authenticate_header(www_auth_str) {
                                if auth_type.to_lowercase() == "bearer" {
                                    let realm = params.get("realm").cloned().unwrap_or_default();
                                    let service = params.get("service").cloned().unwrap_or_default();
                                    let scope = params.get("scope").cloned().unwrap_or_default();

                                    info!("Auth params - realm: {}, service: {}, scope: {}", realm, service, scope);

                                    // Get token from Docker Hub auth service
                                    match self.get_docker_hub_token(&realm, &service, &scope).await {
                                        Ok(token) => {
                                            info!("Successfully obtained token, retrying request with authentication");

                                            // For blob requests, use the specialized handler
                                            if is_blob_request {
                                                return self.handle_blob_request(
                                                    &url,
                                                    method,
                                                    headers_for_auth,
                                                    body.as_ref(),
                                                    Some(&token)
                                                ).await;
                                            }

                                            // Add authorization header
                                            let mut headers_with_auth = headers_for_auth.clone();
                                            let auth_value = format!("Bearer {}", token);
                                            if let Ok(auth_header) = header::HeaderValue::from_str(&auth_value) {
                                                headers_with_auth.insert(header::AUTHORIZATION, auth_header);
                                            }

                                            // Create authenticated request
                                            let auth_request_builder = self.create_request_builder(method, &url, headers_with_auth, body.as_ref())?;

                                            info!("Sending authenticated request to: {} {}", method, url);

                                            match auth_request_builder.send().await {
                                                Ok(auth_response) => {
                                                    info!("Received authenticated response: {} from {}", auth_response.status(), url);
                                                    return Ok(auth_response);
                                                },
                                                Err(e) => {
                                                    error!("Failed to send authenticated request: {}", e);
                                                    // Fall back to returning the original response
                                                }
                                            }
                                        },
                                        Err(e) => {
                                            error!("Failed to get Docker Hub token: {}", e);
                                            // Continue with the original response
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // If we get here, authentication failed or wasn't possible
                    error!("Docker Hub authentication required. Please configure Docker Hub credentials.");
                } else if response.status() == reqwest::StatusCode::FORBIDDEN {
                    // For 403 Forbidden responses, log details and try alternative approaches
                    error!("Received 403 Forbidden response from Docker Hub. This might be due to rate limiting.");

                    // Log rate limit headers if present
                    for (key, value) in response.headers() {
                        if key.as_str().to_lowercase().contains("ratelimit") {
                            error!("Rate limit header - {}: {}", key, value.to_str().unwrap_or_default());
                        }
                    }

                    // For manifest requests, we could try to use a different approach
                    if formatted_path.contains("/manifests/") && !is_blob_request {
                        info!("Manifest request detected, trying alternative approach");

                        // Extract repository and reference from path
                        // Path format is typically /v2/library/redis/manifests/latest
                        let parts: Vec<&str> = formatted_path.split('/').collect();
                        if parts.len() >= 5 {
                            let repo = if parts[2] == "library" {
                                parts[3].to_string()
                            } else {
                                format!("{}/{}", parts[2], parts[3])
                            };
                            let reference = parts[5];

                            // Try to access the manifest directly from Docker Hub API
                            let api_url = format!("https://hub.docker.com/v2/repositories/{}/tags/{}", repo, reference);
                            info!("Trying Docker Hub API: {}", api_url);

                            let api_request = self.client.get(&api_url)
                                .header(header::USER_AGENT, "docker-registry-proxy")
                                .timeout(std::time::Duration::from_secs(30));

                            match api_request.send().await {
                                Ok(api_response) => {
                                    if api_response.status().is_success() {
                                        info!("Successfully retrieved manifest from Docker Hub API");
                                        return Ok(api_response);
                                    } else {
                                        warn!("Failed to retrieve manifest from Docker Hub API: {}", api_response.status());
                                    }
                                },
                                Err(e) => {
                                    warn!("Failed to access Docker Hub API: {}", e);
                                }
                            }
                        }
                    }
                }
            }

            return Ok(response);
        } else {
            // For blob requests, first authenticate if needed
            if registry_url == "registry-1.docker.io" {
                // Extract repository name from path for scope
                // Path format is typically /v2/library/redis/blobs/sha256:...
                // We need to extract "library/redis" for the scope
                let repo_path = formatted_path
                    .split("/blobs/")
                    .next()
                    .unwrap_or("/v2/library")
                    .trim_start_matches("/v2/");

                // Ensure we have a valid repository path
                let repo_path = if repo_path.is_empty() {
                    "library/redis" // Default to library/redis if extraction fails
                } else {
                    repo_path
                };

                info!("Extracted repository path: {}", repo_path);
                let scope = format!("repository:{}:pull", repo_path);
                let realm = "https://auth.docker.io/token";
                let service = "registry.docker.io";

                info!("Blob request detected, pre-authenticating with scope: {}", scope);

                // Get token from Docker Hub auth service
                match self.get_docker_hub_token(realm, service, &scope).await {
                    Ok(token) => {
                        // Use specialized blob handler with authentication
                        return self.handle_blob_request(
                            &url,
                            method,
                            headers_for_auth,
                            body.as_ref(),
                            Some(&token)
                        ).await;
                    },
                    Err(e) => {
                        warn!("Failed to pre-authenticate for blob request: {}", e);
                        // Try without authentication
                        self.handle_blob_request(
                            &url,
                            method,
                            headers_clone,
                            body.as_ref(),
                            None
                        ).await
                    }
                }
            } else {
                // For non-Docker Hub registries, just use the blob handler without authentication
                self.handle_blob_request(
                    &url,
                    method,
                    headers_clone,
                    body.as_ref(),
                    None
                ).await
            }
        }
    }
}
