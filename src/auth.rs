use anyhow::{Context, Result, bail};

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

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/auth", config.api_url))
        .json(&serde_json::json!({
            "access-id": access_id,
            "access-key": access_key,
            "access-type": "access_key"
        }))
        .send()
        .await
        .context("sending auth request to Akeyless")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("Akeyless auth failed ({status}): {body}");
    }

    let body: serde_json::Value = resp.json().await.context("parsing auth response")?;
    let token = body["token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no token in auth response"))?
        .to_string();

    Ok(token)
}
