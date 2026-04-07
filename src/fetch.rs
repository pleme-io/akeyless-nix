use std::collections::BTreeMap;

use anyhow::Result;

use crate::manifest::SecretSpec;
use crate::traits::SecretProvider;

/// Fetch all secrets via a `SecretProvider`. Returns a map of akeyless_path -> value.
pub async fn fetch_all(
    provider: &dyn SecretProvider,
    secrets: &[SecretSpec],
) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();

    for secret in secrets {
        let value = provider.get_secret(&secret.akeyless_path).await?;
        result.insert(secret.akeyless_path.clone(), value);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct MockProvider {
        secrets: BTreeMap<String, String>,
    }

    #[async_trait]
    impl SecretProvider for MockProvider {
        async fn get_secret(&self, path: &str) -> anyhow::Result<String> {
            self.secrets
                .get(path)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("not found: {path}"))
        }
    }

    #[tokio::test]
    async fn test_fetch_all_with_mock() {
        let mut mock_secrets = BTreeMap::new();
        mock_secrets.insert("/a".into(), "val-a".into());
        mock_secrets.insert("/b".into(), "val-b".into());

        let provider = MockProvider {
            secrets: mock_secrets,
        };
        let specs = vec![
            SecretSpec::for_test("/a", "/tmp/a"),
            SecretSpec::for_test("/b", "/tmp/b"),
        ];

        let result = fetch_all(&provider, &specs).await.unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result["/a"], "val-a");
        assert_eq!(result["/b"], "val-b");
    }

    #[tokio::test]
    async fn test_fetch_missing_secret() {
        let provider = MockProvider {
            secrets: BTreeMap::new(),
        };
        let specs = vec![SecretSpec::for_test("/missing", "/tmp/m")];

        let result = fetch_all(&provider, &specs).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_empty_specs() {
        let provider = MockProvider {
            secrets: BTreeMap::new(),
        };
        let result = fetch_all(&provider, &[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_fetch_partial_failure() {
        let mut mock_secrets = BTreeMap::new();
        mock_secrets.insert("/exists".into(), "value".into());

        let provider = MockProvider {
            secrets: mock_secrets,
        };
        let specs = vec![
            SecretSpec::for_test("/exists", "/tmp/e"),
            SecretSpec::for_test("/missing", "/tmp/m"),
        ];

        // Should fail because /missing is not in the mock
        let result = fetch_all(&provider, &specs).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_duplicate_paths_deduplicates() {
        let mut mock_secrets = BTreeMap::new();
        mock_secrets.insert("/dup".into(), "dup-val".into());

        let provider = MockProvider {
            secrets: mock_secrets,
        };
        let specs = vec![
            SecretSpec::for_test("/dup", "/tmp/a"),
            SecretSpec::for_test("/dup", "/tmp/b"),
        ];

        let result = fetch_all(&provider, &specs).await.unwrap();
        // BTreeMap deduplicates by key, so only 1 entry
        assert_eq!(result.len(), 1);
        assert_eq!(result["/dup"], "dup-val");
    }

    #[tokio::test]
    async fn test_fetch_error_message_contains_path() {
        let provider = MockProvider {
            secrets: BTreeMap::new(),
        };
        let specs = vec![SecretSpec::for_test("/specific/path/for/test", "/tmp/x")];

        let result = fetch_all(&provider, &specs).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("/specific/path/for/test"),
            "error should contain the missing path: {err}"
        );
    }

    #[tokio::test]
    async fn test_fetch_preserves_special_characters_in_values() {
        let mut mock_secrets = BTreeMap::new();
        mock_secrets.insert(
            "/cert".into(),
            "-----BEGIN CERT-----\ndata\n-----END CERT-----\n".into(),
        );
        mock_secrets.insert("/pass".into(), "p@$$w0rd\t\n".into());

        let provider = MockProvider {
            secrets: mock_secrets,
        };
        let specs = vec![
            SecretSpec::for_test("/cert", "/tmp/cert"),
            SecretSpec::for_test("/pass", "/tmp/pass"),
        ];

        let result = fetch_all(&provider, &specs).await.unwrap();
        assert!(result["/cert"].contains("BEGIN CERT"));
        assert!(result["/pass"].contains("p@$$w0rd"));
    }

    #[tokio::test]
    async fn test_fetch_single_secret() {
        let mut mock_secrets = BTreeMap::new();
        mock_secrets.insert("/only".into(), "solo".into());

        let provider = MockProvider {
            secrets: mock_secrets,
        };
        let specs = vec![SecretSpec::for_test("/only", "/tmp/only")];

        let result = fetch_all(&provider, &specs).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result["/only"], "solo");
    }
}
