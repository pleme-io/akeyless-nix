//! Installer service — orchestrates the full secret installation flow.
//!
//! Composes all trait objects (`SecretProvider`, `TemplateEngine`, `CacheStore`) into
//! a single entry point. This is the core architecture: everything flows
//! through the Installer, which delegates to traits at every I/O boundary.
//!
//! ```text
//! Installer::new(provider, engine, cache)
//!   .install(manifest, ignore_passwd)
//!     1. fetch all secrets via provider
//!     2. render templates via engine (pure, no I/O)
//!     3. write generation via filesystem
//!     4. switch symlinks atomically
//!     5. prune old generations
//!     6. cache secrets via cache
//! ```

use std::collections::BTreeMap;

use anyhow::Result;

use crate::fetch;
use crate::generation;
use crate::manifest::Manifest;
use crate::template;
use crate::traits::{CacheStore, SecretProvider, TemplateEngine};

/// Result of a successful installation.
#[must_use]
pub(crate) struct InstallResult {
    /// Number of secrets written in this generation.
    pub(crate) secrets_count: usize,
    /// Number of rendered templates written in this generation.
    pub(crate) templates_count: usize,
    /// The generation number assigned to this install.
    pub(crate) generation_number: u64,
}

/// The Installer orchestrates secret fetching, rendering, writing, and caching.
///
/// All I/O is delegated to trait objects, making the entire flow testable
/// with mocks.
pub struct Installer<'a> {
    provider: &'a dyn SecretProvider,
    engine: &'a dyn TemplateEngine,
    cache: Option<&'a dyn CacheStore>,
}

impl<'a> Installer<'a> {
    /// Create a new Installer with the given dependencies.
    #[must_use]
    pub fn new(
        provider: &'a dyn SecretProvider,
        engine: &'a dyn TemplateEngine,
        cache: Option<&'a dyn CacheStore>,
    ) -> Self {
        Self {
            provider,
            engine,
            cache,
        }
    }

    /// Run the full installation flow.
    pub async fn install(
        &self,
        manifest: &Manifest,
        ignore_passwd: bool,
    ) -> Result<InstallResult> {
        // 1. Fetch all secrets via provider
        let secrets = self.fetch_with_fallback(manifest).await?;

        // 2. Render templates via engine (pure computation, no I/O)
        let rendered = template::render_all(self.engine, &manifest.templates, &secrets)?;

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
                if let Some(cache) = self.cache
                    && let Ok(Some(cached)) = cache.load()
                {
                    eprintln!(
                        "akeyless-nix: WARNING — API fetch failed ({e}), using cached secrets"
                    );
                    return Ok(cached);
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
    use crate::template::{sha256_hex, IgataEngine, PlaceholderEngine};
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

    /// Mock engine that records calls and returns content unchanged.
    struct RecordingEngine {
        calls: Mutex<Vec<String>>,
    }

    impl RecordingEngine {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }
        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl TemplateEngine for RecordingEngine {
        fn render(&self, template: &str, _secrets: &BTreeMap<String, String>) -> Result<String> {
            self.calls.lock().unwrap().push(template.to_string());
            Ok(template.to_string())
        }
    }

    fn test_manifest(dir: &std::path::Path) -> Manifest {
        Manifest {
            secrets: vec![SecretSpec::for_test(
                "/test/secret",
                &dir.join("secret-file").to_string_lossy(),
            )],
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
        let engine = PlaceholderEngine;
        let cache = MockCache::empty();
        let installer = Installer::new(&provider, &engine, Some(&cache));
        let manifest = test_manifest(&dir);

        let result = installer.install(&manifest, true).await.unwrap();

        assert_eq!(result.secrets_count, 1);
        assert_eq!(result.templates_count, 0);
        assert_eq!(result.generation_number, 1);

        let content = std::fs::read_to_string(dir.join("secret-file")).unwrap();
        assert_eq!(content, "my-value");

        let cached = cache.load().unwrap().unwrap();
        assert_eq!(cached["/test/secret"], "my-value");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_with_placeholder_template() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-tmpl");
        let _ = std::fs::remove_dir_all(&dir);

        let mut secrets = BTreeMap::new();
        secrets.insert("/db/password".into(), "s3cret".into());

        let hash = sha256_hex("/db/password");
        let placeholder = format!("<AKEYLESS:{hash}:PLACEHOLDER>");

        let manifest = Manifest {
            secrets: vec![SecretSpec::for_test(
                "/db/password",
                &dir.join("db-pass").to_string_lossy(),
            )],
            templates: vec![TemplateSpec::for_test(
                "db-config",
                &format!("connection: postgresql://user:{placeholder}@host/db"),
                &dir.join("db-config").to_string_lossy(),
            )],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let provider = MockProvider { secrets };
        let engine = PlaceholderEngine;
        let installer = Installer::new(&provider, &engine, None);

        let result = installer.install(&manifest, true).await.unwrap();
        assert_eq!(result.secrets_count, 1);
        assert_eq!(result.templates_count, 1);

        let rendered = std::fs::read_to_string(dir.join("db-config")).unwrap();
        assert_eq!(rendered, "connection: postgresql://user:s3cret@host/db");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_with_igata_template() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-igata");
        let _ = std::fs::remove_dir_all(&dir);

        let mut secrets = BTreeMap::new();
        secrets.insert("/db/password".into(), "s3cret".into());

        let manifest = Manifest {
            secrets: vec![SecretSpec::for_test(
                "/db/password",
                &dir.join("db-pass").to_string_lossy(),
            )],
            templates: vec![TemplateSpec::for_test(
                "db-config",
                "connection: postgresql://user:[= db_password =]@host/db",
                &dir.join("db-config").to_string_lossy(),
            )],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let provider = MockProvider { secrets };
        let engine = IgataEngine::new();
        let installer = Installer::new(&provider, &engine, None);

        let result = installer.install(&manifest, true).await.unwrap();
        assert_eq!(result.secrets_count, 1);
        assert_eq!(result.templates_count, 1);

        let rendered = std::fs::read_to_string(dir.join("db-config")).unwrap();
        assert_eq!(rendered, "connection: postgresql://user:s3cret@host/db");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_cache_fallback() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-fallback");
        let _ = std::fs::remove_dir_all(&dir);

        let mut cached_secrets = BTreeMap::new();
        cached_secrets.insert("/test/secret".into(), "cached-value".into());

        let provider = FailingProvider;
        let engine = PlaceholderEngine;
        let cache = MockCache::with_data(cached_secrets);
        let installer = Installer::new(&provider, &engine, Some(&cache));
        let manifest = test_manifest(&dir);

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
        let engine = PlaceholderEngine;
        let installer = Installer::new(&provider, &engine, None);
        let manifest = test_manifest(&dir);

        let result = installer.install(&manifest, true).await;
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_check_reports_per_secret() {
        let mut secrets = BTreeMap::new();
        secrets.insert("/exists".into(), "val".into());

        let provider = MockProvider { secrets };
        let engine = PlaceholderEngine;
        let installer = Installer::new(&provider, &engine, None);

        let manifest = Manifest {
            secrets: vec![
                SecretSpec::for_test("/exists", "/tmp/e"),
                SecretSpec::for_test("/missing", "/tmp/m"),
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

    #[tokio::test]
    async fn test_recording_engine_tracks_calls() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-recording");
        let _ = std::fs::remove_dir_all(&dir);

        let mut secrets = BTreeMap::new();
        secrets.insert("/app/key".into(), "val".into());

        let manifest = Manifest {
            secrets: vec![SecretSpec::for_test(
                "/app/key",
                &dir.join("key").to_string_lossy(),
            )],
            templates: vec![
                TemplateSpec::for_test("a", "template-a", &dir.join("a").to_string_lossy()),
                TemplateSpec::for_test("b", "template-b", &dir.join("b").to_string_lossy()),
            ],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let provider = MockProvider { secrets };
        let engine = RecordingEngine::new();
        let installer = Installer::new(&provider, &engine, None);

        installer.install(&manifest, true).await.unwrap();

        assert_eq!(engine.call_count(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_empty_manifest() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-empty");
        let _ = std::fs::remove_dir_all(&dir);

        let provider = MockProvider {
            secrets: BTreeMap::new(),
        };
        let engine = PlaceholderEngine;
        let installer = Installer::new(&provider, &engine, None);

        let manifest = Manifest {
            secrets: vec![],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let result = installer.install(&manifest, true).await.unwrap();
        assert_eq!(result.secrets_count, 0);
        assert_eq!(result.templates_count, 0);
        assert_eq!(result.generation_number, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_multi_secret_multi_template() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-full-combo");
        let _ = std::fs::remove_dir_all(&dir);

        let mut secrets = BTreeMap::new();
        secrets.insert("/app/db-pass".into(), "pg-secret".into());
        secrets.insert("/app/api-key".into(), "key-42".into());

        let hash = sha256_hex("/app/api-key");
        let placeholder = format!("<AKEYLESS:{hash}:PLACEHOLDER>");

        let manifest = Manifest {
            secrets: vec![
                SecretSpec::for_test("/app/db-pass", &dir.join("db-pass").to_string_lossy()),
                SecretSpec::for_test("/app/api-key", &dir.join("api-key").to_string_lossy()),
            ],
            templates: vec![TemplateSpec::for_test(
                "app-env",
                &format!("DB_PASS=inline\nAPI_KEY={placeholder}\n"),
                &dir.join("app-env").to_string_lossy(),
            )],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let provider = MockProvider { secrets };
        let engine = PlaceholderEngine;
        let cache = MockCache::empty();
        let installer = Installer::new(&provider, &engine, Some(&cache));

        let result = installer.install(&manifest, true).await.unwrap();
        assert_eq!(result.secrets_count, 2);
        assert_eq!(result.templates_count, 1);

        assert_eq!(
            std::fs::read_to_string(dir.join("db-pass")).unwrap(),
            "pg-secret"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("api-key")).unwrap(),
            "key-42"
        );

        let env_content = std::fs::read_to_string(dir.join("app-env")).unwrap();
        assert!(env_content.contains("API_KEY=key-42"));
        assert!(!env_content.contains("AKEYLESS"));

        let cached = cache.load().unwrap().unwrap();
        assert_eq!(cached.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_cache_fallback_with_empty_cache() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-emptycache-fallback");
        let _ = std::fs::remove_dir_all(&dir);

        let provider = FailingProvider;
        let engine = PlaceholderEngine;
        let cache = MockCache::empty();
        let installer = Installer::new(&provider, &engine, Some(&cache));
        let manifest = test_manifest(&dir);

        let result = installer.install(&manifest, true).await;
        assert!(
            result.is_err(),
            "empty cache cannot provide fallback, so install should fail"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_check_empty_manifest() {
        let provider = MockProvider {
            secrets: BTreeMap::new(),
        };
        let engine = PlaceholderEngine;
        let installer = Installer::new(&provider, &engine, None);

        let manifest = Manifest {
            secrets: vec![],
            templates: vec![],
            generations_dir: "/tmp/g".into(),
            symlink_path: "/tmp/s".into(),
            keep_generations: 2,
        };

        let results = installer.check(&manifest).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_check_all_missing() {
        let provider = MockProvider {
            secrets: BTreeMap::new(),
        };
        let engine = PlaceholderEngine;
        let installer = Installer::new(&provider, &engine, None);

        let manifest = Manifest {
            secrets: vec![
                SecretSpec::for_test("/a", "/tmp/a"),
                SecretSpec::for_test("/b", "/tmp/b"),
            ],
            templates: vec![],
            generations_dir: "/tmp/g".into(),
            symlink_path: "/tmp/s".into(),
            keep_generations: 2,
        };

        let results = installer.check(&manifest).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(!results[0].1);
        assert!(!results[1].1);
    }

    #[tokio::test]
    async fn test_check_all_present() {
        let mut secrets = BTreeMap::new();
        secrets.insert("/a".into(), "va".into());
        secrets.insert("/b".into(), "vb".into());

        let provider = MockProvider { secrets };
        let engine = PlaceholderEngine;
        let installer = Installer::new(&provider, &engine, None);

        let manifest = Manifest {
            secrets: vec![
                SecretSpec::for_test("/a", "/tmp/a"),
                SecretSpec::for_test("/b", "/tmp/b"),
            ],
            templates: vec![],
            generations_dir: "/tmp/g".into(),
            symlink_path: "/tmp/s".into(),
            keep_generations: 2,
        };

        let results = installer.check(&manifest).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].1);
        assert!(results[1].1);
    }

    struct FailingCache;

    impl CacheStore for FailingCache {
        fn store(&self, _secrets: &BTreeMap<String, String>) -> Result<()> {
            anyhow::bail!("cache write failed")
        }
        fn load(&self) -> Result<Option<BTreeMap<String, String>>> {
            anyhow::bail!("cache read failed")
        }
    }

    #[tokio::test]
    async fn test_install_cache_store_failure_is_non_fatal() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-cache-fail");
        let _ = std::fs::remove_dir_all(&dir);

        let mut secrets = BTreeMap::new();
        secrets.insert("/test/secret".into(), "value".into());

        let provider = MockProvider { secrets };
        let engine = PlaceholderEngine;
        let cache = FailingCache;
        let installer = Installer::new(&provider, &engine, Some(&cache));
        let manifest = test_manifest(&dir);

        let result = installer.install(&manifest, true).await;
        assert!(
            result.is_ok(),
            "cache store failure should be non-fatal: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().secrets_count, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_with_no_cache_succeeds() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-no-cache");
        let _ = std::fs::remove_dir_all(&dir);

        let mut secrets = BTreeMap::new();
        secrets.insert("/test/secret".into(), "my-value".into());

        let provider = MockProvider { secrets };
        let engine = PlaceholderEngine;
        let installer = Installer::new(&provider, &engine, None);
        let manifest = test_manifest(&dir);

        let result = installer.install(&manifest, true).await.unwrap();
        assert_eq!(result.secrets_count, 1);
        assert_eq!(result.generation_number, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_successive_generations_increment() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-gen-incr");
        let _ = std::fs::remove_dir_all(&dir);

        let mut secrets = BTreeMap::new();
        secrets.insert("/test/secret".into(), "v1".into());

        let provider = MockProvider {
            secrets: secrets.clone(),
        };
        let engine = PlaceholderEngine;
        let installer = Installer::new(&provider, &engine, None);
        let manifest = test_manifest(&dir);

        let r1 = installer.install(&manifest, true).await.unwrap();
        assert_eq!(r1.generation_number, 1);

        let r2 = installer.install(&manifest, true).await.unwrap();
        assert_eq!(r2.generation_number, 2);

        let r3 = installer.install(&manifest, true).await.unwrap();
        assert_eq!(r3.generation_number, 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_provider_failure_with_failing_cache_load() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-both-fail");
        let _ = std::fs::remove_dir_all(&dir);

        let provider = FailingProvider;
        let engine = PlaceholderEngine;
        let cache = FailingCache;
        let installer = Installer::new(&provider, &engine, Some(&cache));
        let manifest = test_manifest(&dir);

        let result = installer.install(&manifest, true).await;
        assert!(
            result.is_err(),
            "both provider and cache failing should error"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_with_failing_engine() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-engine-fail");
        let _ = std::fs::remove_dir_all(&dir);

        struct FailEngine;
        impl TemplateEngine for FailEngine {
            fn render(
                &self,
                _template: &str,
                _secrets: &BTreeMap<String, String>,
            ) -> Result<String> {
                anyhow::bail!("engine failure")
            }
        }

        let mut secrets = BTreeMap::new();
        secrets.insert("/test/secret".into(), "val".into());

        let manifest = Manifest {
            secrets: vec![SecretSpec::for_test(
                "/test/secret",
                &dir.join("secret-file").to_string_lossy(),
            )],
            templates: vec![TemplateSpec::for_test("t", "content", &dir.join("t").to_string_lossy())],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let provider = MockProvider { secrets };
        let engine = FailEngine;
        let installer = Installer::new(&provider, &engine, None);

        let result = installer.install(&manifest, true).await;
        assert!(result.is_err(), "engine failure should propagate");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_prunes_old_generations() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-prune");
        let _ = std::fs::remove_dir_all(&dir);

        let mut secrets = BTreeMap::new();
        secrets.insert("/test/secret".into(), "val".into());

        let manifest = Manifest {
            secrets: vec![SecretSpec::for_test(
                "/test/secret",
                &dir.join("secret-file").to_string_lossy(),
            )],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        for _ in 0..4 {
            let provider = MockProvider {
                secrets: secrets.clone(),
            };
            let engine = PlaceholderEngine;
            let installer = Installer::new(&provider, &engine, None);
            installer.install(&manifest, true).await.unwrap();
        }

        let mut remaining: Vec<u64> = std::fs::read_dir(dir.join("generations"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str()?.parse::<u64>().ok())
            .collect();
        remaining.sort();
        assert_eq!(remaining.len(), 2, "should keep only 2 generations");
        assert_eq!(remaining, vec![3, 4]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_check_returns_paths_in_manifest_order() {
        let mut secrets = BTreeMap::new();
        secrets.insert("/z".into(), "vz".into());
        secrets.insert("/a".into(), "va".into());

        let provider = MockProvider { secrets };
        let engine = PlaceholderEngine;
        let installer = Installer::new(&provider, &engine, None);

        let manifest = Manifest {
            secrets: vec![
                SecretSpec::for_test("/z", "/tmp/z"),
                SecretSpec::for_test("/a", "/tmp/a"),
                SecretSpec::for_test("/missing", "/tmp/m"),
            ],
            templates: vec![],
            generations_dir: "/tmp/g".into(),
            symlink_path: "/tmp/s".into(),
            keep_generations: 2,
        };

        let results = installer.check(&manifest).await.unwrap();
        assert_eq!(results[0].0, "/z");
        assert!(results[0].1);
        assert_eq!(results[1].0, "/a");
        assert!(results[1].1);
        assert_eq!(results[2].0, "/missing");
        assert!(!results[2].1);
    }

    #[tokio::test]
    async fn test_install_with_igata_template_multiline() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-igata-multi");
        let _ = std::fs::remove_dir_all(&dir);

        let mut secrets = BTreeMap::new();
        secrets.insert("/app/host".into(), "example.com".into());
        secrets.insert("/app/port".into(), "8080".into());

        let manifest = Manifest {
            secrets: vec![
                SecretSpec::for_test("/app/host", &dir.join("host").to_string_lossy()),
                SecretSpec::for_test("/app/port", &dir.join("port").to_string_lossy()),
            ],
            templates: vec![TemplateSpec::for_test(
                "config",
                "host=[= app_host =]\nport=[= app_port =]",
                &dir.join("config").to_string_lossy(),
            )],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let provider = MockProvider { secrets };
        let engine = IgataEngine::new();
        let installer = Installer::new(&provider, &engine, None);

        let result = installer.install(&manifest, true).await.unwrap();
        assert_eq!(result.secrets_count, 2);
        assert_eq!(result.templates_count, 1);

        let rendered = std::fs::read_to_string(dir.join("config")).unwrap();
        assert_eq!(rendered, "host=example.com\nport=8080");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_install_cache_is_updated_after_successful_fetch() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-installer-cache-update");
        let _ = std::fs::remove_dir_all(&dir);

        let mut secrets = BTreeMap::new();
        secrets.insert("/test/secret".into(), "fresh-value".into());

        let provider = MockProvider { secrets };
        let engine = PlaceholderEngine;
        let cache = MockCache::empty();
        let installer = Installer::new(&provider, &engine, Some(&cache));
        let manifest = test_manifest(&dir);

        installer.install(&manifest, true).await.unwrap();

        let cached = cache.load().unwrap().unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached["/test/secret"], "fresh-value");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
