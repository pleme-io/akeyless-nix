use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use shikumi::{ConfigDiscovery, Format, ProviderChain};
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    #[serde(default = "default_auth")]
    pub auth: AuthConfig,
    #[serde(default = "default_api_url")]
    pub api_url: String,
    #[serde(default)]
    pub cache: CacheConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AuthConfig {
    #[serde(default = "default_access_id_file")]
    pub access_id_file: String,
    #[serde(default = "default_access_key_file")]
    pub access_key_file: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CacheConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_cache_dir")]
    pub dir: String,
    /// Cache time-to-live in seconds. Cached secrets older than this are
    /// considered stale during fallback. Reserved for future TTL-based
    /// cache expiry implementation.
    #[serde(default = "default_ttl")]
    #[allow(dead_code)]
    pub ttl_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auth: default_auth(),
            api_url: default_api_url(),
            cache: CacheConfig::default(),
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            access_id_file: default_access_id_file(),
            access_key_file: default_access_key_file(),
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dir: default_cache_dir(),
            ttl_seconds: default_ttl(),
        }
    }
}

fn default_auth() -> AuthConfig {
    AuthConfig::default()
}
fn default_api_url() -> String {
    "https://api.akeyless.io".to_string()
}
fn default_access_id_file() -> String {
    "~/.config/akeyless/access-id".to_string()
}
fn default_access_key_file() -> String {
    "~/.config/akeyless/access-key".to_string()
}
fn default_cache_dir() -> String {
    "~/.cache/akeyless-nix".to_string()
}
fn default_ttl() -> u64 {
    3600
}
fn default_true() -> bool {
    true
}

/// Expand a path string, replacing `~` with the user's home directory.
pub(crate) fn expand_path(path: &str) -> PathBuf {
    let expanded = shellexpand::tilde(path);
    PathBuf::from(expanded.as_ref())
}

/// Load config using shikumi's config discovery and provider chain.
///
/// Discovery order (via `ConfigDiscovery`):
/// 1. `$AKEYLESS_NIX_CONFIG` environment variable override
/// 2. `$XDG_CONFIG_HOME/akeyless-nix/akeyless-nix.yaml`
/// 3. `$HOME/.config/akeyless-nix/akeyless-nix.yaml`
/// 4. Legacy: `$HOME/.akeyless-nix`, `$HOME/.akeyless-nix.toml`
///
/// Provider chain layering (via `ProviderChain`):
/// `Config::default()` → config file → `AKEYLESS_NIX_` env vars
///
/// Falls back to defaults if no config file exists at any location.
pub fn load() -> Result<Config> {
    let defaults = Config::default();

    let discovery = ConfigDiscovery::new("akeyless-nix")
        .env_override("AKEYLESS_NIX_CONFIG")
        .formats(&[Format::Yaml]);

    let mut chain = ProviderChain::new().with_defaults(&defaults);

    if let Ok(path) = discovery.discover() {
        chain = chain.with_file(&path);
    }
    // If no config file found, we proceed with defaults + env overrides only.

    chain = chain.with_env("AKEYLESS_NIX_");

    chain
        .extract()
        .context("loading akeyless-nix configuration")
}

impl Config {
    pub fn access_id_path(&self) -> PathBuf {
        expand_path(&self.auth.access_id_file)
    }

    pub fn access_key_path(&self) -> PathBuf {
        expand_path(&self.auth.access_key_file)
    }

    pub fn cache_dir(&self) -> PathBuf {
        expand_path(&self.cache.dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        // When no config file exists, defaults should work
        let config = Config::default();

        assert_eq!(config.api_url, "https://api.akeyless.io");
        assert!(config.cache.enabled);
        assert_eq!(config.cache.ttl_seconds, 3600);
        assert_eq!(
            config.auth.access_id_file,
            "~/.config/akeyless/access-id"
        );
        assert_eq!(
            config.auth.access_key_file,
            "~/.config/akeyless/access-key"
        );
    }

    #[test]
    fn test_expand_path() {
        let path = expand_path("~/test");
        assert!(!path.to_string_lossy().contains('~'));
    }

    #[test]
    fn test_provider_chain_loads_yaml_file() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-config-chain");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let yaml_content = r#"
auth:
  access_id_file: /tmp/test-access-id
  access_key_file: /tmp/test-access-key
api_url: https://custom-api.example.com
cache:
  enabled: false
  dir: /tmp/test-cache
  ttl_seconds: 7200
"#;
        let yaml_path = dir.join("test-config.yaml");
        std::fs::write(&yaml_path, yaml_content).unwrap();

        // Test ProviderChain directly (no env var contamination)
        let config: Config = ProviderChain::new()
            .with_defaults(&Config::default())
            .with_file(&yaml_path)
            .extract()
            .unwrap();

        assert_eq!(config.auth.access_id_file, "/tmp/test-access-id");
        assert_eq!(config.auth.access_key_file, "/tmp/test-access-key");
        assert_eq!(config.api_url, "https://custom-api.example.com");
        assert!(!config.cache.enabled);
        assert_eq!(config.cache.dir, "/tmp/test-cache");
        assert_eq!(config.cache.ttl_seconds, 7200);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_provider_chain_defaults_without_file() {
        // ProviderChain with defaults only should produce correct defaults
        let config: Config = ProviderChain::new()
            .with_defaults(&Config::default())
            .extract()
            .unwrap();

        assert_eq!(config.auth.access_id_file, "~/.config/akeyless/access-id");
        assert_eq!(
            config.auth.access_key_file,
            "~/.config/akeyless/access-key"
        );
        assert_eq!(config.api_url, "https://api.akeyless.io");
        assert!(config.cache.enabled);
        assert_eq!(config.cache.ttl_seconds, 3600);
    }

    #[test]
    fn test_provider_chain_partial_override() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-config-partial");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let yaml_content = "api_url: https://override.example.com\n";
        let yaml_path = dir.join("partial-config.yaml");
        std::fs::write(&yaml_path, yaml_content).unwrap();

        let config: Config = ProviderChain::new()
            .with_defaults(&Config::default())
            .with_file(&yaml_path)
            .extract()
            .unwrap();

        // Overridden value
        assert_eq!(config.api_url, "https://override.example.com");
        // Default values preserved
        assert_eq!(config.auth.access_id_file, "~/.config/akeyless/access-id");
        assert!(config.cache.enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_config_discovery_uses_yaml_format() {
        // Verify ConfigDiscovery scans for yaml files
        let discovery = ConfigDiscovery::new("akeyless-nix")
            .formats(&[Format::Yaml]);
        let paths = discovery.standard_paths();
        let path_strs: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        assert!(path_strs.iter().any(|p| p.contains("akeyless-nix/akeyless-nix.yaml")));
    }
}
