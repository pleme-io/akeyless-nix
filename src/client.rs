use anyhow::{Context, Result, bail};
use async_trait::async_trait;

use crate::traits::SecretProvider;

/// Akeyless API client for fetching secret values.
pub struct AkeylessClient {
    http: reqwest::Client,
    api_url: String,
    token: String,
}

impl AkeylessClient {
    /// Create a new client with the given API URL and pre-authenticated token.
    pub fn new(api_url: &str, token: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_url: api_url.to_string(),
            token: token.to_string(),
        }
    }

    /// Fetch a single secret value by its Akeyless path (inherent method).
    pub async fn fetch_secret_value(&self, path: &str) -> Result<String> {
        let resp = self
            .http
            .post(format!("{}/get-secret-value", self.api_url))
            .json(&serde_json::json!({
                "names": [path],
                "token": self.token
            }))
            .send()
            .await
            .with_context(|| format!("fetching secret {path}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("failed to get secret {path} ({status}): {body}");
        }

        let body: serde_json::Value = resp.json().await
            .with_context(|| format!("parsing response for {path}"))?;

        // Response is { "/path/to/secret": "value" }
        body[path]
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
