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
        let registry_url = match self.config.get_registry_url(registry_key) {
            Some(url) => url,
            None => {
                error!("Unsupported registry: {}", registry_key);
                return Err(format!("Unsupported registry: {}", registry_key).into());
            }
        };

        let url = if let Some(q) = query {
            format!("https://{}{}{}", registry_url, path, q)
        } else {
            format!("https://{}{}", registry_url, path)
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
        request_builder = request_builder.headers(headers);

        // Add body if present
        if let Some(b) = body {
            request_builder = request_builder.body(b);
        }

        let response = request_builder.send().await?;
        Ok(response)
    }
}
