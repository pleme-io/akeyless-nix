use std::collections::BTreeMap;

use anyhow::{Context, Result};

use crate::config::Config;

/// Store fetched secrets in local cache for offline fallback.
pub fn store(config: &Config, secrets: &BTreeMap<String, String>) -> Result<()> {
    let cache_dir = config.cache_dir();
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("creating cache dir {}", cache_dir.display()))?;

    let cache_file = cache_dir.join("secrets.json");
    let content = serde_json::to_string_pretty(secrets)
        .context("serializing secrets for cache")?;

    std::fs::write(&cache_file, content)
        .with_context(|| format!("writing cache {}", cache_file.display()))?;

    // Restrict permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cache_file, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

/// Load secrets from local cache (for offline fallback).
#[allow(dead_code)]
pub fn load(config: &Config) -> Result<Option<BTreeMap<String, String>>> {
    let cache_file = config.cache_dir().join("secrets.json");
    if !cache_file.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&cache_file)
        .with_context(|| format!("reading cache {}", cache_file.display()))?;
    let secrets: BTreeMap<String, String> = serde_json::from_str(&content)
        .with_context(|| format!("parsing cache {}", cache_file.display()))?;

    Ok(Some(secrets))
}
