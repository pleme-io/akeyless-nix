use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub auth: AuthConfig,
    #[serde(default = "default_api_url")]
    pub api_url: String,
    #[serde(default)]
    pub cache: CacheConfig,
}

#[derive(Debug, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "default_access_id_file")]
    pub access_id_file: String,
    #[serde(default = "default_access_key_file")]
    pub access_key_file: String,
}

#[derive(Debug, Deserialize)]
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

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dir: default_cache_dir(),
            ttl_seconds: default_ttl(),
        }
    }
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

pub(crate) fn expand_path(path: &str) -> PathBuf {
    let expanded = shellexpand::tilde(path);
    PathBuf::from(expanded.as_ref())
}

/// Load config from ~/.config/akeyless-nix/akeyless-nix.yaml
/// Falls back to defaults if file doesn't exist.
pub fn load() -> Result<Config> {
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("akeyless-nix");
    let config_path = config_dir.join("akeyless-nix.yaml");

    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("reading config {}", config_path.display()))?;
        serde_yaml::from_str(&content)
            .with_context(|| format!("parsing config {}", config_path.display()))
    } else {
        // Use defaults — credentials from standard Akeyless paths
        Ok(Config {
            auth: AuthConfig {
                access_id_file: default_access_id_file(),
                access_key_file: default_access_key_file(),
            },
            api_url: default_api_url(),
            cache: CacheConfig::default(),
        })
    }
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
        let config = Config {
            auth: AuthConfig {
                access_id_file: "~/.config/akeyless/access-id".to_string(),
                access_key_file: "~/.config/akeyless/access-key".to_string(),
            },
            api_url: "https://api.akeyless.io".to_string(),
            cache: CacheConfig::default(),
        };

        assert_eq!(config.api_url, "https://api.akeyless.io");
        assert!(config.cache.enabled);
        assert_eq!(config.cache.ttl_seconds, 3600);
    }

    #[test]
    fn test_expand_path() {
        let path = expand_path("~/test");
        assert!(!path.to_string_lossy().contains('~'));
    }

    #[test]
    fn test_load_from_yaml_file() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-config-yaml");
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

        let content = std::fs::read_to_string(&yaml_path).unwrap();
        let config: Config = serde_yaml::from_str(&content).unwrap();

        assert_eq!(config.auth.access_id_file, "/tmp/test-access-id");
        assert_eq!(config.auth.access_key_file, "/tmp/test-access-key");
        assert_eq!(config.api_url, "https://custom-api.example.com");
        assert!(!config.cache.enabled);
        assert_eq!(config.cache.dir, "/tmp/test-cache");
        assert_eq!(config.cache.ttl_seconds, 7200);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_yaml_with_defaults() {
        // Minimal YAML — all optional fields should use defaults
        let yaml_content = r#"
auth: {}
"#;
        let config: Config = serde_yaml::from_str(yaml_content).unwrap();

        assert_eq!(config.auth.access_id_file, "~/.config/akeyless/access-id");
        assert_eq!(config.auth.access_key_file, "~/.config/akeyless/access-key");
        assert_eq!(config.api_url, "https://api.akeyless.io");
        assert!(config.cache.enabled);
        assert_eq!(config.cache.ttl_seconds, 3600);
    }
}
