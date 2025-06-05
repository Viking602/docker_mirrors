use crate::config::RegistryConfig;
use reqwest::{Client, Response, header, Method};
use log::{info, warn};
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
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self { client, config }
    }

    fn parse_www_authenticate_header(&self, header_value: &str) -> Option<(String, HashMap<String, String>)> {
        let parts: Vec<&str> = header_value.splitn(2, ' ').collect();
        if parts.len() != 2 {
            return None;
        }

        let auth_type = parts[0].to_string();
        let params = parts[1]
            .split(',')
            .filter_map(|param| {
                let kv: Vec<&str> = param.splitn(2, '=').collect();
                if kv.len() == 2 {
                    Some((kv[0].trim().to_string(), kv[1].trim().trim_matches('"').to_string()))
                } else {
                    None
                }
            })
            .collect();

        Some((auth_type, params))
    }

    async fn get_docker_hub_token(&self, realm: &str, service: &str, scope: &str) -> Result<String, Box<dyn Error>> {
        let url = format!("{}?service={}&scope={}", realm, service, scope);
        info!("Requesting token from: {}", url);

        let mut request = self.client.get(&url);
        
        if let (Some(username), Some(password)) = (&self.config.docker_hub_credentials.username, &self.config.docker_hub_credentials.password) {
            request = request.basic_auth(username, Some(password));
            info!("Using configured Docker Hub credentials");
        } else {
            warn!("Using anonymous Docker Hub access");
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(format!("Token request failed: {}", response.status()).into());
        }

        let token_response: TokenResponse = response.json().await?;
        Ok(if !token_response.token.is_empty() { token_response.token } else { token_response.access_token })
    }

    fn format_docker_hub_path(&self, registry_key: &str, path: &str) -> String {
        match (registry_key, path) {
            ("v2", "/") => "/v2/".to_string(),
            ("v2", _) => format!("/v2{}", path),
            (_, path) if path.starts_with("/v2") => path.to_string(),
            (_, path) if path.starts_with("/library/") => format!("/v2{}", path),
            (_, path) if path.starts_with("/") && !path.starts_with("/v2/") => {
                if path.matches('/').count() >= 2 {
                    format!("/v2{}", path)
                } else {
                    format!("/v2/library{}", path)
                }
            }
            _ => format!("/v2/{}", path),
        }
    }

    fn prepare_headers(&self, headers: &reqwest::header::HeaderMap, registry_url: &str, is_blob_request: bool) -> reqwest::header::HeaderMap {
        let mut headers_clone = headers.clone();
        
        if let Ok(host) = reqwest::header::HeaderValue::from_str(registry_url) {
            headers_clone.insert(reqwest::header::HOST, host);
        }

        headers_clone.insert(
            reqwest::header::HeaderName::from_static("docker-distribution-api-version"),
            reqwest::header::HeaderValue::from_static("registry/2.0"),
        );

        if registry_url == "registry-1.docker.io" {
            headers_clone.insert(
                reqwest::header::USER_AGENT,
                reqwest::header::HeaderValue::from_static("docker/20.10.12 go/go1.16.12 git-commit/459d0df kernel/5.10.47 os/linux arch/amd64 UpstreamClient(Docker-Client/20.10.12 \\(linux\\))")
            );
            headers_clone.insert(
                reqwest::header::HeaderName::from_static("accept-encoding"),
                reqwest::header::HeaderValue::from_static("gzip")
            );
            headers_clone.insert(header::CONNECTION, reqwest::header::HeaderValue::from_static("keep-alive"));
            headers_clone.insert(header::CACHE_CONTROL, reqwest::header::HeaderValue::from_static("max-age=0"));
        }

        let accept = if is_blob_request {
            "application/octet-stream, application/vnd.docker.image.rootfs.diff.tar.gzip, application/vnd.oci.image.layer.v1.tar+gzip"
        } else {
            "application/json, application/vnd.docker.distribution.manifest.v2+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.oci.image.index.v1+json"
        };
        
        headers_clone.insert(header::ACCEPT, reqwest::header::HeaderValue::from_static(accept));
        headers_clone
    }

    async fn handle_authentication(&self, response: &Response, headers: &mut reqwest::header::HeaderMap) -> Result<(), Box<dyn Error>> {
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(www_auth) = response.headers().get(header::WWW_AUTHENTICATE) {
                if let Ok(www_auth_str) = www_auth.to_str() {
                    if let Some((auth_type, params)) = self.parse_www_authenticate_header(www_auth_str) {
                        if auth_type.to_lowercase() == "bearer" {
                            let realm = params.get("realm").cloned().unwrap_or_default();
                            let service = params.get("service").cloned().unwrap_or_default();
                            let scope = params.get("scope").cloned().unwrap_or_default();

                            let token = self.get_docker_hub_token(&realm, &service, &scope).await?;
                            let auth_value = format!("Bearer {}", token);
                            if let Ok(auth_header) = header::HeaderValue::from_str(&auth_value) {
                                headers.insert(header::AUTHORIZATION, auth_header);
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
        Ok(())
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
        let registry_url = if registry_key == "v2" {
            "registry-1.docker.io"
        } else {
            self.config.get_registry_url(registry_key)
                .ok_or_else(|| format!("Unsupported registry: {}", registry_key))?
        };

        let formatted_path = if registry_url == "registry-1.docker.io" {
            self.format_docker_hub_path(registry_key, path)
        } else {
            path.to_string()
        };

        let url = format!("https://{}{}{}", 
            registry_url, 
            formatted_path,
            query.unwrap_or("")
        );

        info!("Forwarding request to: {}", url);
        let is_blob_request = formatted_path.contains("/blobs/");
        let mut headers = self.prepare_headers(&headers, registry_url, is_blob_request);

        let method_enum = Method::from_str(method)?;
        let mut request = self.client.request(method_enum.clone(), &url).headers(headers.clone());

        if let Some(b) = &body {
            request = request.body(b.clone());
        }

        let response = request.send().await?;
        
        if registry_url == "registry-1.docker.io" {
            if let Err(e) = self.handle_authentication(&response, &mut headers).await {
                warn!("Authentication failed: {}", e);
            } else if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                // Retry request with new authentication
                let mut retry_request = self.client.request(method_enum, &url).headers(headers);
                if let Some(b) = body {
                    retry_request = retry_request.body(b);
                }
                return Ok(retry_request.send().await?);
            }
        }

        Ok(response)
    }
}
