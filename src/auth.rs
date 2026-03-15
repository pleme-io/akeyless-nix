use anyhow::{Context, Result};

use crate::config::Config;

/// Authenticate to Akeyless and return a temporary token.
pub async fn authenticate(config: &Config) -> Result<String> {
    let access_id = std::fs::read_to_string(config.access_id_path())
        .with_context(|| format!("reading access-id from {}", config.access_id_path().display()))?
        .trim()
        .to_string();

    let access_key = std::fs::read_to_string(config.access_key_path())
        .with_context(|| format!("reading access-key from {}", config.access_key_path().display()))?
        .trim()
        .to_string();

    let mut api_config = akeyless_api::apis::configuration::Configuration::new();
    api_config.base_path = config.api_url.clone();

    let auth_req = akeyless_api::models::Auth {
        access_id: Some(access_id),
        access_key: Some(access_key),
        access_type: Some("access_key".to_string()),
        ..Default::default()
    };

    let output = akeyless_api::apis::v2_api::auth(&api_config, auth_req)
        .await
        .context("authenticating to Akeyless")?;

    output
        .token
        .ok_or_else(|| anyhow::anyhow!("no token in auth response"))
}
