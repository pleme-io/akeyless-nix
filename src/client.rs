use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::traits::SecretProvider;

/// Akeyless API client backed by the generated SDK types.
pub struct AkeylessClient {
    config: akeyless_api::apis::configuration::Configuration,
    token: String,
}

impl AkeylessClient {
    /// Create a new client with the given API URL and pre-authenticated token.
    pub fn new(api_url: &str, token: &str) -> Self {
        let mut config = akeyless_api::apis::configuration::Configuration::new();
        config.base_path = api_url.to_string();
        Self {
            config,
            token: token.to_string(),
        }
    }

    /// Fetch a single secret value by its Akeyless path (typed SDK call).
    pub async fn fetch_secret_value(&self, path: &str) -> Result<String> {
        let req = akeyless_api::models::GetSecretValue {
            names: vec![path.to_string()],
            token: Some(self.token.clone()),
            ..Default::default()
        };

        let response: serde_json::Value =
            akeyless_api::apis::v2_api::get_secret_value(&self.config, req)
                .await
                .with_context(|| format!("fetching secret {path}"))?;

        // Response is { "/path/to/secret": "value" }
        response[path]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("secret {path} not found in response"))
    }
}

#[async_trait]
impl SecretProvider for AkeylessClient {
    async fn get_secret(&self, path: &str) -> Result<String> {
        self.fetch_secret_value(path).await
    }
}
