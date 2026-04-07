use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::config::Config;
use crate::traits::CacheStore;

/// File-system backed cache store.
pub struct FsCache {
    cache_dir: PathBuf,
}

impl FsCache {
    /// Create a new `FsCache` using the cache directory from the given config.
    pub fn new(config: &Config) -> Self {
        Self {
            cache_dir: config.cache_dir(),
        }
    }
}

impl CacheStore for FsCache {
    fn store(&self, secrets: &BTreeMap<String, String>) -> Result<()> {
        std::fs::create_dir_all(&self.cache_dir)
            .with_context(|| format!("creating cache dir {}", self.cache_dir.display()))?;

        let cache_file = self.cache_dir.join("secrets.json");
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

    fn load(&self) -> Result<Option<BTreeMap<String, String>>> {
        let cache_file = self.cache_dir.join("secrets.json");
        if !cache_file.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&cache_file)
            .with_context(|| format!("reading cache {}", cache_file.display()))?;
        let secrets: BTreeMap<String, String> = serde_json::from_str(&content)
            .with_context(|| format!("parsing cache {}", cache_file.display()))?;

        Ok(Some(secrets))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fs_cache_store_and_load() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-cache-trait");
        let _ = std::fs::remove_dir_all(&dir);

        let cache = FsCache {
            cache_dir: dir.clone(),
        };

        let mut secrets = BTreeMap::new();
        secrets.insert("/a".into(), "val-a".into());
        secrets.insert("/b".into(), "val-b".into());

        cache.store(&secrets).unwrap();

        let loaded = cache.load().unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded["/a"], "val-a");
        assert_eq!(loaded["/b"], "val-b");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fs_cache_load_missing() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-cache-miss");
        let _ = std::fs::remove_dir_all(&dir);

        let cache = FsCache {
            cache_dir: dir.clone(),
        };

        let loaded = cache.load().unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_fs_cache_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join("akeyless-nix-test-cache-perms");
        let _ = std::fs::remove_dir_all(&dir);

        let cache = FsCache {
            cache_dir: dir.clone(),
        };

        let mut secrets = BTreeMap::new();
        secrets.insert("/secret".into(), "value".into());

        cache.store(&secrets).unwrap();

        let cache_file = dir.join("secrets.json");
        let perms = std::fs::metadata(&cache_file).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fs_cache_new_from_config() {
        let config = Config {
            auth: crate::config::AuthConfig::default(),
            api_url: "https://api.akeyless.io".to_string(),
            cache: crate::config::CacheConfig {
                enabled: true,
                dir: "/tmp/test-cache-new".to_string(),
                ttl_seconds: 3600,
            },
        };
        let cache = FsCache::new(&config);
        assert_eq!(cache.cache_dir, PathBuf::from("/tmp/test-cache-new"));
    }

    #[test]
    fn test_fs_cache_new_expands_tilde() {
        let config = Config {
            auth: crate::config::AuthConfig::default(),
            api_url: "https://api.akeyless.io".to_string(),
            cache: crate::config::CacheConfig {
                enabled: true,
                dir: "~/.cache/test-akeyless".to_string(),
                ttl_seconds: 3600,
            },
        };
        let cache = FsCache::new(&config);
        assert!(
            !cache.cache_dir.to_string_lossy().contains('~'),
            "FsCache::new should use config.cache_dir() which expands tilde"
        );
    }

    #[test]
    fn test_fs_cache_store_empty_map() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-cache-empty-map");
        let _ = std::fs::remove_dir_all(&dir);

        let cache = FsCache {
            cache_dir: dir.clone(),
        };

        let secrets = BTreeMap::new();
        cache.store(&secrets).unwrap();

        let loaded = cache.load().unwrap();
        assert!(loaded.is_some());
        assert!(loaded.unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fs_cache_overwrite_existing() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-cache-overwrite");
        let _ = std::fs::remove_dir_all(&dir);

        let cache = FsCache {
            cache_dir: dir.clone(),
        };

        let mut secrets1 = BTreeMap::new();
        secrets1.insert("/old".into(), "old-value".into());
        cache.store(&secrets1).unwrap();

        let mut secrets2 = BTreeMap::new();
        secrets2.insert("/new".into(), "new-value".into());
        cache.store(&secrets2).unwrap();

        let loaded = cache.load().unwrap().unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(!loaded.contains_key("/old"));
        assert_eq!(loaded["/new"], "new-value");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fs_cache_load_corrupted_json() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-cache-corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let cache_file = dir.join("secrets.json");
        std::fs::write(&cache_file, "not valid json {{{").unwrap();

        let cache = FsCache {
            cache_dir: dir.clone(),
        };

        let result = cache.load();
        assert!(result.is_err(), "corrupted JSON should produce an error");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fs_cache_store_creates_nested_dirs() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-cache-nested/a/b/c");
        let _ = std::fs::remove_dir_all(
            std::env::temp_dir().join("akeyless-nix-test-cache-nested"),
        );

        let cache = FsCache {
            cache_dir: dir.clone(),
        };

        let mut secrets = BTreeMap::new();
        secrets.insert("/key".into(), "val".into());
        cache.store(&secrets).unwrap();

        let loaded = cache.load().unwrap().unwrap();
        assert_eq!(loaded["/key"], "val");

        let _ = std::fs::remove_dir_all(
            std::env::temp_dir().join("akeyless-nix-test-cache-nested"),
        );
    }

    #[test]
    fn test_fs_cache_special_characters_in_values() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-cache-special");
        let _ = std::fs::remove_dir_all(&dir);

        let cache = FsCache {
            cache_dir: dir.clone(),
        };

        let mut secrets = BTreeMap::new();
        secrets.insert("/cert".into(), "-----BEGIN RSA-----\nMIIBxTCC\n-----END RSA-----\n".into());
        secrets.insert("/pass".into(), "p@ss$w0rd!&<>\"'\\\t\n".into());
        secrets.insert("/unicode".into(), "こんにちは🔑".into());
        cache.store(&secrets).unwrap();

        let loaded = cache.load().unwrap().unwrap();
        assert_eq!(loaded["/cert"], "-----BEGIN RSA-----\nMIIBxTCC\n-----END RSA-----\n");
        assert_eq!(loaded["/pass"], "p@ss$w0rd!&<>\"'\\\t\n");
        assert_eq!(loaded["/unicode"], "こんにちは🔑");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fs_cache_large_secret_values() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-cache-large");
        let _ = std::fs::remove_dir_all(&dir);

        let cache = FsCache {
            cache_dir: dir.clone(),
        };

        let mut secrets = BTreeMap::new();
        let large_value = "x".repeat(1_000_000);
        secrets.insert("/big".into(), large_value.clone());
        cache.store(&secrets).unwrap();

        let loaded = cache.load().unwrap().unwrap();
        assert_eq!(loaded["/big"], large_value);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fs_cache_special_chars_in_keys() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-cache-special-keys");
        let _ = std::fs::remove_dir_all(&dir);

        let cache = FsCache {
            cache_dir: dir.clone(),
        };

        let mut secrets = BTreeMap::new();
        secrets.insert("/path/with spaces".into(), "val1".into());
        secrets.insert("/path/with\"quotes".into(), "val2".into());
        secrets.insert("/path/with\nnewline".into(), "val3".into());
        cache.store(&secrets).unwrap();

        let loaded = cache.load().unwrap().unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded["/path/with spaces"], "val1");
        assert_eq!(loaded["/path/with\"quotes"], "val2");
        assert_eq!(loaded["/path/with\nnewline"], "val3");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
