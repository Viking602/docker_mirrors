use crate::config::RegistryConfig;
use reqwest::{Client, Response, header};
use log::{error, info, warn};
use std::error::Error;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    issued_at: String,
}

pub struct ProxyService {
    client: Client,
    config: RegistryConfig,
}

impl ProxyService {
    pub fn new(config: RegistryConfig) -> Self {
        Self {
            client: Client::new(),
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
                    format!("/v2/library{}", path)
                } else {
                    format!("/v2/{}", path)
                }
            } else {
                path.to_string()
            }
        } else {
            path.to_string()
        };

        let url = if let Some(q) = query {
            format!("https://{}{}{}", registry_url, formatted_path, q)
        } else {
            format!("https://{}{}", registry_url, formatted_path)
        };

        info!("Forwarding request to: {}", url);

        let mut request_builder = match method {
            "GET" => self.client.get(&url),
            "POST" => self.client.post(&url),
            "PUT" => self.client.put(&url),
            "DELETE" => self.client.delete(&url),
            "HEAD" => self.client.head(&url),
            "PATCH" => self.client.patch(&url),
            _ => {
                error!("Unsupported HTTP method: {}", method);
                return Err(format!("Unsupported HTTP method: {}", method).into());
            }
        };

        // Add headers
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
            let user_agent = reqwest::header::HeaderValue::from_static("docker-registry-proxy");
            headers_clone.insert(reqwest::header::USER_AGENT, user_agent);
        }

        // Log outgoing headers
        info!("Outgoing headers:");
        for (key, value) in headers_clone.iter() {
            info!("  {}: {}", key, value.to_str().unwrap_or_default());
        }

        // Clone headers_clone before moving it
        let headers_for_auth = headers_clone.clone();

        request_builder = request_builder.headers(headers_clone);

        // Add body if present
        let body_for_auth = body.clone();
        if let Some(ref b) = body {
            request_builder = request_builder.body(b.clone());
        }

        // Log outgoing request details
        info!("Sending request to: {} {}", method, url);

        let response = request_builder.send().await?;
        info!("Received response: {} from {}", response.status(), url);

        // Handle Docker Hub authentication
        if registry_url == "registry-1.docker.io" && response.status() == reqwest::StatusCode::UNAUTHORIZED {
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

                                    // Retry the request with the token
                                    let mut headers_with_auth = headers_for_auth.clone();
                                    let auth_value = format!("Bearer {}", token);
                                    if let Ok(auth_header) = header::HeaderValue::from_str(&auth_value) {
                                        headers_with_auth.insert(header::AUTHORIZATION, auth_header);
                                    }

                                    // Rebuild the request with the token
                                    let mut auth_request_builder = match method {
                                        "GET" => self.client.get(&url),
                                        "POST" => self.client.post(&url),
                                        "PUT" => self.client.put(&url),
                                        "DELETE" => self.client.delete(&url),
                                        "HEAD" => self.client.head(&url),
                                        "PATCH" => self.client.patch(&url),
                                        _ => {
                                            error!("Unsupported HTTP method: {}", method);
                                            return Err(format!("Unsupported HTTP method: {}", method).into());
                                        }
                                    };

                                    auth_request_builder = auth_request_builder.headers(headers_with_auth);

                                    // Add body if present and it's not a HEAD request
                                    if let Some(ref b) = body_for_auth {
                                        if method != "HEAD" {
                                            auth_request_builder = auth_request_builder.body(b.clone());
                                        }
                                    }

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
        }

        Ok(response)
    }
}
