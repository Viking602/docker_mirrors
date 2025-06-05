use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerHubCredentials {
    pub username: Option<String>,
    pub password: Option<String>,
}

impl DockerHubCredentials {
    pub fn from_env() -> Self {
        Self {
            username: env::var("DOCKER_HUB_USERNAME").ok(),
            password: env::var("DOCKER_HUB_PASSWORD").ok(),
        }
    }

}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    pub registries: HashMap<String, String>,
    pub docker_hub_credentials: DockerHubCredentials,
}

impl RegistryConfig {
    pub fn new() -> Self {
        let mut registries = HashMap::new();
        
        // 默认注册表配置
        let default_registries = [
            ("docker", "registry-1.docker.io"),
            ("quay", "quay.io"),
            ("gcr", "gcr.io"),
            ("k8s-gcr", "k8s.gcr.io"),
            ("k8s", "registry.k8s.io"),
            ("ghcr", "ghcr.io"),
            ("cloudsmith", "docker.cloudsmith.io"),
            ("nvcr", "nvcr.io"),
            ("gitlab", "registry.gitlab.com"),
        ];

        for (key, value) in default_registries {
            registries.insert(key.to_string(), value.to_string());
        }

        Self { 
            registries,
            docker_hub_credentials: DockerHubCredentials::from_env(),
        }
    }

    pub fn get_registry_url(&self, registry_key: &str) -> Option<&String> {
        self.registries.get(registry_key)
    }

}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self::new()
    }
}
