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
}
