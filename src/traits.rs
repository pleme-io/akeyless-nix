use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;

/// Trait abstracting Akeyless API interactions.
///
/// Authentication is a separate concern handled by `auth::authenticate()` before
/// a provider is constructed. This trait only covers secret retrieval.
#[async_trait]
pub trait SecretProvider: Send + Sync {
    /// Fetch a secret value by path.
    async fn get_secret(&self, path: &str) -> Result<String>;
}

/// Trait abstracting file system operations for generation management.
///
/// Secret file writing (with ownership and permissions) is handled separately
/// by `write::write_secret_with_ownership`. This trait covers directory and
/// symlink operations used during generation creation and switching.
pub trait FileWriter: Send + Sync {
    /// Create a directory and all parent directories.
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    /// Remove a file (non-fatal if it does not exist).
    fn remove_file(&self, path: &Path) -> Result<()>;
    /// Create a symbolic link from `src` to `dst`.
    fn symlink(&self, src: &Path, dst: &Path) -> Result<()>;
}

/// Trait abstracting secret cache operations.
pub trait CacheStore: Send + Sync {
    /// Persist secrets to cache storage.
    fn store(&self, secrets: &BTreeMap<String, String>) -> Result<()>;
    /// Load secrets from cache storage, returning None if no cache exists.
    fn load(&self) -> Result<Option<BTreeMap<String, String>>>;
}
