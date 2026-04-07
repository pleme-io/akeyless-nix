//! Shared test doubles for use across unit and integration test modules.
//!
//! All types in this module are gated behind `#[cfg(test)]` in `main.rs`
//! so they never appear in production builds.

use std::collections::BTreeMap;
use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;

use crate::traits::{CacheStore, SecretProvider};

/// In-memory mock secret provider backed by a `BTreeMap`.
///
/// Returns `Err` for any path not in the map with a descriptive message.
pub struct MockProvider {
    pub secrets: BTreeMap<String, String>,
}

#[async_trait]
impl SecretProvider for MockProvider {
    async fn get_secret(&self, path: &str) -> Result<String> {
        self.secrets
            .get(path)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("not found: {path}"))
    }
}

/// In-memory mock cache store.
///
/// Stores and loads a `BTreeMap<String, String>` in a `Mutex` for thread
/// safety in async tests.
pub struct MockCache {
    stored: Mutex<Option<BTreeMap<String, String>>>,
}

impl MockCache {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            stored: Mutex::new(None),
        }
    }

    #[must_use]
    pub fn with_data(data: BTreeMap<String, String>) -> Self {
        Self {
            stored: Mutex::new(Some(data)),
        }
    }
}

impl CacheStore for MockCache {
    fn store(&self, secrets: &BTreeMap<String, String>) -> Result<()> {
        *self.stored.lock().unwrap() = Some(secrets.clone());
        Ok(())
    }

    fn load(&self) -> Result<Option<BTreeMap<String, String>>> {
        Ok(self.stored.lock().unwrap().clone())
    }
}

/// A secret provider that always fails, for testing fallback paths.
pub struct FailingProvider;

#[async_trait]
impl SecretProvider for FailingProvider {
    async fn get_secret(&self, _path: &str) -> Result<String> {
        anyhow::bail!("API unreachable")
    }
}

/// A cache store that always fails, for testing error resilience.
pub struct FailingCache;

impl CacheStore for FailingCache {
    fn store(&self, _secrets: &BTreeMap<String, String>) -> Result<()> {
        anyhow::bail!("cache write failed")
    }

    fn load(&self) -> Result<Option<BTreeMap<String, String>>> {
        anyhow::bail!("cache read failed")
    }
}
