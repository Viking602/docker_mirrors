use crate::config::RegistryConfig;
use reqwest::{Client, Response};
use log::{error, info};
use std::error::Error;
use bytes::Bytes;

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

        request_builder = request_builder.headers(headers_clone);

        // Add body if present
        if let Some(b) = body {
            request_builder = request_builder.body(b);
        }

        // Log outgoing request details
        info!("Sending request to: {} {}", method, url);

        let response = request_builder.send().await?;
        info!("Received response: {} from {}", response.status(), url);

        // Handle Docker Hub authentication
        if registry_url == "registry-1.docker.io" && response.status() == reqwest::StatusCode::UNAUTHORIZED {
            info!("Received unauthorized response from Docker Hub, attempting to authenticate");

            // In a real implementation, we would handle authentication here
            // For now, we'll just log the issue and return the response
            error!("Docker Hub authentication required. Please configure Docker Hub credentials.");
        }

        Ok(response)
    }
}
