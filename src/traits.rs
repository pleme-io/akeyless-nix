use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;

/// Trait abstracting Akeyless API interactions.
#[async_trait]
pub trait SecretProvider: Send + Sync {
    /// Authenticate and return a token.
    async fn authenticate(&self) -> Result<String>;
    /// Fetch a secret value by path.
    async fn get_secret(&self, path: &str) -> Result<String>;
}

/// Trait abstracting file system operations.
pub trait FileWriter: Send + Sync {
    fn write_file(&self, path: &Path, content: &str, mode: u32) -> Result<()>;
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    fn remove_file(&self, path: &Path) -> Result<()>;
    fn symlink(&self, src: &Path, dst: &Path) -> Result<()>;
}

/// Trait abstracting secret cache operations.
pub trait CacheStore: Send + Sync {
    /// Persist secrets to cache storage.
    fn store(&self, secrets: &BTreeMap<String, String>) -> Result<()>;
    /// Load secrets from cache storage, returning None if no cache exists.
    fn load(&self) -> Result<Option<BTreeMap<String, String>>>;
}
