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
    #[serde(default = "default_ttl")]
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

fn expand_path(path: &str) -> PathBuf {
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
