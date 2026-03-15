use std::collections::BTreeMap;

use anyhow::Result;

use crate::client::AkeylessClient;
use crate::manifest::SecretSpec;

/// Fetch all secrets from Akeyless. Returns a map of akeyless_path → value.
pub async fn fetch_all(
    client: &AkeylessClient,
    secrets: &[SecretSpec],
) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();

    for secret in secrets {
        let value = client.get_secret(&secret.akeyless_path).await?;
        result.insert(secret.akeyless_path.clone(), value);
    }

    Ok(result)
}
