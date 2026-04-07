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
pub(crate) fn load() -> Result<Config> {
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

        assert_eq!(config.api_url, "https://override.example.com");
        assert_eq!(config.auth.access_id_file, "~/.config/akeyless/access-id");
        assert!(config.cache.enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_config_discovery_uses_yaml_format() {
        let discovery = ConfigDiscovery::new("akeyless-nix")
            .formats(&[Format::Yaml]);
        let paths = discovery.standard_paths();
        let path_strs: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        assert!(path_strs.iter().any(|p| p.contains("akeyless-nix/akeyless-nix.yaml")));
    }

    #[test]
    fn test_access_id_path_expands_tilde() {
        let config = Config::default();
        let path = config.access_id_path();
        assert!(
            !path.to_string_lossy().contains('~'),
            "access_id_path should expand tilde"
        );
        assert!(
            path.to_string_lossy().ends_with(".config/akeyless/access-id"),
            "should preserve relative part after tilde expansion"
        );
    }

    #[test]
    fn test_access_key_path_expands_tilde() {
        let config = Config::default();
        let path = config.access_key_path();
        assert!(!path.to_string_lossy().contains('~'));
        assert!(path.to_string_lossy().ends_with(".config/akeyless/access-key"));
    }

    #[test]
    fn test_cache_dir_expands_tilde() {
        let config = Config::default();
        let path = config.cache_dir();
        assert!(!path.to_string_lossy().contains('~'));
        assert!(path.to_string_lossy().ends_with(".cache/akeyless-nix"));
    }

    #[test]
    fn test_config_paths_with_absolute_paths() {
        let config = Config {
            auth: AuthConfig {
                access_id_file: "/absolute/path/to/id".to_string(),
                access_key_file: "/absolute/path/to/key".to_string(),
            },
            api_url: "https://api.example.com".to_string(),
            cache: CacheConfig {
                enabled: true,
                dir: "/absolute/cache".to_string(),
                ttl_seconds: 3600,
            },
        };
        assert_eq!(
            config.access_id_path(),
            std::path::PathBuf::from("/absolute/path/to/id")
        );
        assert_eq!(
            config.access_key_path(),
            std::path::PathBuf::from("/absolute/path/to/key")
        );
        assert_eq!(
            config.cache_dir(),
            std::path::PathBuf::from("/absolute/cache")
        );
    }

    #[test]
    fn test_expand_path_absolute_unchanged() {
        let path = expand_path("/absolute/path");
        assert_eq!(path, std::path::PathBuf::from("/absolute/path"));
    }

    #[test]
    fn test_expand_path_relative_unchanged() {
        let path = expand_path("relative/path");
        assert_eq!(path, std::path::PathBuf::from("relative/path"));
    }

    #[test]
    fn test_expand_path_empty_string() {
        let path = expand_path("");
        assert_eq!(path, std::path::PathBuf::from(""));
    }

    #[test]
    fn test_expand_path_tilde_only() {
        let path = expand_path("~");
        assert!(!path.to_string_lossy().contains('~'));
        assert!(!path.to_string_lossy().is_empty());
    }

    #[test]
    fn test_auth_config_default() {
        let auth = AuthConfig::default();
        assert_eq!(auth.access_id_file, "~/.config/akeyless/access-id");
        assert_eq!(auth.access_key_file, "~/.config/akeyless/access-key");
    }

    #[test]
    fn test_cache_config_default() {
        let cache = CacheConfig::default();
        assert!(cache.enabled);
        assert_eq!(cache.dir, "~/.cache/akeyless-nix");
        assert_eq!(cache.ttl_seconds, 3600);
    }

    #[test]
    fn test_config_serde_roundtrip() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.api_url, config.api_url);
        assert_eq!(
            deserialized.auth.access_id_file,
            config.auth.access_id_file
        );
        assert_eq!(
            deserialized.auth.access_key_file,
            config.auth.access_key_file
        );
        assert_eq!(deserialized.cache.enabled, config.cache.enabled);
        assert_eq!(deserialized.cache.dir, config.cache.dir);
        assert_eq!(deserialized.cache.ttl_seconds, config.cache.ttl_seconds);
    }

    #[test]
    fn test_config_from_empty_json() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config.api_url, "https://api.akeyless.io");
        assert!(config.cache.enabled);
        assert_eq!(config.cache.ttl_seconds, 3600);
    }

    #[test]
    fn test_config_empty_yaml_uses_defaults() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-config-empty-yaml");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let yaml_path = dir.join("empty.yaml");
        std::fs::write(&yaml_path, "{}").unwrap();

        let config: Config = ProviderChain::new()
            .with_defaults(&Config::default())
            .with_file(&yaml_path)
            .extract()
            .unwrap();

        assert_eq!(config.api_url, "https://api.akeyless.io");
        assert!(config.cache.enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_config_cache_disabled_via_yaml() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-config-cache-disabled");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let yaml = "cache:\n  enabled: false\n  ttl_seconds: 0\n";
        let yaml_path = dir.join("config.yaml");
        std::fs::write(&yaml_path, yaml).unwrap();

        let config: Config = ProviderChain::new()
            .with_defaults(&Config::default())
            .with_file(&yaml_path)
            .extract()
            .unwrap();

        assert!(!config.cache.enabled);
        assert_eq!(config.cache.ttl_seconds, 0);
        assert_eq!(config.api_url, "https://api.akeyless.io");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_config_env_override_via_provider_chain() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-config-env-chain");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let yaml_path = dir.join("config.yaml");
        std::fs::write(&yaml_path, "api_url: https://file.example.com\n").unwrap();

        unsafe {
            std::env::set_var("AKEYLESS_NIX_API_URL", "https://env.example.com");
        }

        let config: Config = ProviderChain::new()
            .with_defaults(&Config::default())
            .with_file(&yaml_path)
            .with_env("AKEYLESS_NIX_")
            .extract()
            .unwrap();

        assert_eq!(
            config.api_url, "https://env.example.com",
            "env var should override file value"
        );

        unsafe {
            std::env::remove_var("AKEYLESS_NIX_API_URL");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_config_file_overrides_defaults() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-config-file-over-defaults");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let yaml = r#"
auth:
  access_id_file: /custom/id
  access_key_file: /custom/key
api_url: https://custom.api
cache:
  enabled: false
  dir: /custom/cache
  ttl_seconds: 100
"#;
        let yaml_path = dir.join("full.yaml");
        std::fs::write(&yaml_path, yaml).unwrap();

        let config: Config = ProviderChain::new()
            .with_defaults(&Config::default())
            .with_file(&yaml_path)
            .extract()
            .unwrap();

        assert_eq!(config.auth.access_id_file, "/custom/id");
        assert_eq!(config.auth.access_key_file, "/custom/key");
        assert_eq!(config.api_url, "https://custom.api");
        assert!(!config.cache.enabled);
        assert_eq!(config.cache.dir, "/custom/cache");
        assert_eq!(config.cache.ttl_seconds, 100);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_expand_path_tilde_with_subpath() {
        let path = expand_path("~/sub/dir/file.txt");
        assert!(!path.to_string_lossy().contains('~'));
        assert!(path.to_string_lossy().ends_with("sub/dir/file.txt"));
    }
}
