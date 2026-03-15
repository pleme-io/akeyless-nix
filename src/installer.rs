//! Installer service — orchestrates the full secret installation flow.
//!
//! Composes all trait objects (SecretProvider, FileWriter, CacheStore) into
//! a single entry point. This is the core architecture: everything flows
//! through the Installer, which delegates to traits at every I/O boundary.
//!
//! ```text
//! Installer::new(provider, writer, cache)
//!   .install(manifest, ignore_passwd)
//!     1. fetch all secrets via provider
//!     2. render templates (pure, no I/O)
//!     3. write generation via writer
//!     4. switch symlinks via writer
//!     5. prune old generations
//!     6. cache secrets via cache
//! ```

use std::collections::BTreeMap;

use anyhow::Result;

use crate::fetch;
use crate::generation;
use crate::manifest::Manifest;
use crate::template;
use crate::traits::{CacheStore, SecretProvider};

/// Result of a successful installation.
pub struct InstallResult {
    pub secrets_count: usize,
    pub templates_count: usize,
    pub generation_number: u64,
}

/// The Installer orchestrates secret fetching, rendering, writing, and caching.
///
/// All I/O is delegated to trait objects, making the entire flow testable
/// with mocks.
pub struct Installer<'a> {
    provider: &'a dyn SecretProvider,
    cache: Option<&'a dyn CacheStore>,
}

impl<'a> Installer<'a> {
    /// Create a new Installer with the given dependencies.
    pub fn new(provider: &'a dyn SecretProvider, cache: Option<&'a dyn CacheStore>) -> Self {
        Self { provider, cache }
    }

    /// Run the full installation flow.
    pub async fn install(
        &self,
        manifest: &Manifest,
        ignore_passwd: bool,
    ) -> Result<InstallResult> {
        // 1. Fetch all secrets via provider
        let secrets = self.fetch_with_fallback(manifest).await?;

        // 2. Render templates (pure computation, no I/O)
        let rendered = template::render_all(&manifest.templates, &secrets)?;

        // 3. Write generation + switch symlinks + prune
        let gen_info = generation::create(manifest, &secrets, &rendered, ignore_passwd)?;
        generation::switch(manifest, &gen_info, &rendered)?;
        generation::prune(manifest)?;

        // 4. Cache secrets for offline fallback
        if let Some(cache) = self.cache {
            let _ = cache.store(&secrets); // non-fatal
        }

        Ok(InstallResult {
            secrets_count: secrets.len(),
            templates_count: rendered.len(),
            generation_number: gen_info.number,
        })
    }

    /// Check all secrets exist in the provider (dry-run validation).
    pub async fn check(&self, manifest: &Manifest) -> Result<Vec<(String, bool)>> {
        let mut results = Vec::new();
        for secret in &manifest.secrets {
            let ok = self.provider.get_secret(&secret.akeyless_path).await.is_ok();
            results.push((secret.akeyless_path.clone(), ok));
        }
        Ok(results)
    }

    /// Fetch secrets, falling back to cache if the provider fails.
    async fn fetch_with_fallback(
        &self,
        manifest: &Manifest,
    ) -> Result<BTreeMap<String, String>> {
        match fetch::fetch_all(self.provider, &manifest.secrets).await {
            Ok(secrets) => Ok(secrets),
            Err(e) => {
                // Try cache fallback
                if let Some(cache) = self.cache {
                    if let Ok(Some(cached)) = cache.load() {
                        eprintln!(
                            "akeyless-nix: WARNING — API fetch failed ({e}), using cached secrets"
                        );
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{SecretSpec, TemplateSpec};
    use crate::template::sha256_hex;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct MockProvider {
        secrets: BTreeMap<String, String>,
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

    struct MockCache {
        stored: Mutex<Option<BTreeMap<String, String>>>,
    }

    impl MockCache {
        fn empty() -> Self {
            Self {
                stored: Mutex::new(None),
            }
        }
        fn with_data(data: BTreeMap<String, String>) -> Self {
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

    struct FailingProvider;

    #[async_trait]
    impl SecretProvider for FailingProvider {
        async fn get_secret(&self, _path: &str) -> Result<String> {
            anyhow::bail!("API unreachable")
        }
    }

    fn test_manifest(dir: &std::path::Path) -> Manifest {
        Manifest {
            secrets: vec![SecretSpec {
                akeyless_path: "/test/secret".into(),
                file_path: dir.join("secret-file").to_string_lossy().to_string(),
                mode: "0600".into(),
                owner: String::new(),
                group: String::new(),
                uid: None,
                gid: None,
                restart_units: vec![],
                reload_units: vec![],
            }],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        }
    }

    #[tokio::test]
    async fn test_install_full_flow() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer");
        let _ = std::fs::remove_dir_all(&dir);

        let mut secrets = BTreeMap::new();
        secrets.insert("/test/secret".into(), "my-value".into());

        let provider = MockProvider { secrets };
        let cache = MockCache::empty();
        let installer = Installer::new(&provider, Some(&cache));
        let manifest = test_manifest(&dir);

        let result = installer.install(&manifest, true).await.unwrap();

        assert_eq!(result.secrets_count, 1);
        assert_eq!(result.templates_count, 0);
        assert_eq!(result.generation_number, 1);

        // Verify file was written
        let content = std::fs::read_to_string(dir.join("secret-file")).unwrap();
        assert_eq!(content, "my-value");

        // Verify cache was populated
        let cached = cache.load().unwrap().unwrap();
        assert_eq!(cached["/test/secret"], "my-value");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_with_template() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-tmpl");
        let _ = std::fs::remove_dir_all(&dir);

        let mut secrets = BTreeMap::new();
        secrets.insert("/db/password".into(), "s3cret".into());

        let hash = sha256_hex("/db/password");
        let placeholder = format!("<AKEYLESS:{hash}:PLACEHOLDER>");

        let manifest = Manifest {
            secrets: vec![SecretSpec {
                akeyless_path: "/db/password".into(),
                file_path: dir.join("db-pass").to_string_lossy().to_string(),
                mode: "0600".into(),
                owner: String::new(),
                group: String::new(),
                uid: None,
                gid: None,
                restart_units: vec![],
                reload_units: vec![],
            }],
            templates: vec![TemplateSpec {
                name: "db-config".into(),
                content: format!("connection: postgresql://user:{placeholder}@host/db"),
                file_path: dir.join("db-config").to_string_lossy().to_string(),
                mode: "0600".into(),
                owner: String::new(),
                group: String::new(),
                uid: None,
                gid: None,
            }],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let provider = MockProvider { secrets };
        let installer = Installer::new(&provider, None);

        let result = installer.install(&manifest, true).await.unwrap();
        assert_eq!(result.secrets_count, 1);
        assert_eq!(result.templates_count, 1);

        // Template should have placeholder replaced
        let rendered = std::fs::read_to_string(dir.join("db-config")).unwrap();
        assert_eq!(rendered, "connection: postgresql://user:s3cret@host/db");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_cache_fallback() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-fallback");
        let _ = std::fs::remove_dir_all(&dir);

        // Pre-populate cache with the secret
        let mut cached_secrets = BTreeMap::new();
        cached_secrets.insert("/test/secret".into(), "cached-value".into());

        let provider = FailingProvider;
        let cache = MockCache::with_data(cached_secrets);
        let installer = Installer::new(&provider, Some(&cache));
        let manifest = test_manifest(&dir);

        // Should succeed using cache fallback
        let result = installer.install(&manifest, true).await.unwrap();
        assert_eq!(result.secrets_count, 1);

        let content = std::fs::read_to_string(dir.join("secret-file")).unwrap();
        assert_eq!(content, "cached-value");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_no_cache_fails() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-nocache");
        let _ = std::fs::remove_dir_all(&dir);

        let provider = FailingProvider;
        let installer = Installer::new(&provider, None);
        let manifest = test_manifest(&dir);

        // Should fail — no cache, provider is offline
        let result = installer.install(&manifest, true).await;
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_check_reports_per_secret() {
        let mut secrets = BTreeMap::new();
        secrets.insert("/exists".into(), "val".into());

        let provider = MockProvider { secrets };
        let installer = Installer::new(&provider, None);

        let manifest = Manifest {
            secrets: vec![
                SecretSpec {
                    akeyless_path: "/exists".into(),
                    file_path: "/tmp/e".into(),
                    mode: "0400".into(),
                    owner: String::new(),
                    group: String::new(),
                    uid: None,
                    gid: None,
                    restart_units: vec![],
                    reload_units: vec![],
                },
                SecretSpec {
                    akeyless_path: "/missing".into(),
                    file_path: "/tmp/m".into(),
                    mode: "0400".into(),
                    owner: String::new(),
                    group: String::new(),
                    uid: None,
                    gid: None,
                    restart_units: vec![],
                    reload_units: vec![],
                },
            ],
            templates: vec![],
            generations_dir: "/tmp/g".into(),
            symlink_path: "/tmp/s".into(),
            keep_generations: 2,
        };

        let results = installer.check(&manifest).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].1);  // /exists → true
        assert!(!results[1].1); // /missing → false
    }
}
