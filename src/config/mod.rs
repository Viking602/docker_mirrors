use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerHubCredentials {
    pub username: Option<String>,
    pub password: Option<String>,
}

impl Default for DockerHubCredentials {
    fn default() -> Self {
        Self {
            username: env::var("DOCKER_HUB_USERNAME").ok(),
            password: env::var("DOCKER_HUB_PASSWORD").ok(),
        }
    }
}

impl DockerHubCredentials {
    pub fn is_configured(&self) -> bool {
        self.username.is_some() && self.password.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    pub registries: HashMap<String, String>,
    pub docker_hub_credentials: DockerHubCredentials,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        let mut registries = HashMap::new();
        registries.insert("docker".to_string(), "registry-1.docker.io".to_string());
        registries.insert("quay".to_string(), "quay.io".to_string());
        registries.insert("gcr".to_string(), "gcr.io".to_string());
        registries.insert("k8s-gcr".to_string(), "k8s.gcr.io".to_string());
        registries.insert("k8s".to_string(), "registry.k8s.io".to_string());
        registries.insert("ghcr".to_string(), "ghcr.io".to_string());
        registries.insert("cloudsmith".to_string(), "docker.cloudsmith.io".to_string());
        registries.insert("nvcr".to_string(), "nvcr.io".to_string());
        registries.insert("gitlab".to_string(), "registry.gitlab.com".to_string());

        Self { 
            registries,
            docker_hub_credentials: DockerHubCredentials::default(),
        }
    }
}

impl RegistryConfig {
    pub fn get_registry_url(&self, registry_key: &str) -> Option<&String> {
        self.registries.get(registry_key)
    }
}
