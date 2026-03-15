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
}
