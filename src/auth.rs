use anyhow::{Context, Result};

use crate::config::Config;

/// Authenticate to Akeyless and return a temporary token.
pub async fn authenticate(config: &Config) -> Result<String> {
    let access_id = std::fs::read_to_string(config.access_id_path())
        .with_context(|| format!("reading access-id from {}", config.access_id_path().display()))?
        .trim()
        .to_string();

    let access_key = std::fs::read_to_string(config.access_key_path())
        .with_context(|| format!("reading access-key from {}", config.access_key_path().display()))?
        .trim()
        .to_string();

    let mut api_config = akeyless_api::apis::configuration::Configuration::new();
    api_config.base_path = config.api_url.clone();

    let auth_req = akeyless_api::models::Auth {
        access_id: Some(access_id),
        access_key: Some(access_key),
        access_type: Some("access_key".to_string()),
        ..Default::default()
    };

    let output = akeyless_api::apis::v2_api::auth(&api_config, auth_req)
        .await
        .context("authenticating to Akeyless")?;

    output
        .token
        .ok_or_else(|| anyhow::anyhow!("no token in auth response"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_authenticate_missing_access_id_file() {
        let config = Config {
            auth: crate::config::AuthConfig {
                access_id_file: "/tmp/nonexistent-akeyless-id-12345".to_string(),
                access_key_file: "/tmp/nonexistent-akeyless-key-12345".to_string(),
            },
            api_url: "https://api.akeyless.io".to_string(),
            cache: crate::config::CacheConfig::default(),
        };

        let result = authenticate(&config).await;
        assert!(result.is_err(), "missing access-id file should fail");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("reading access-id"),
            "error should mention reading access-id: {err}"
        );
    }

    #[tokio::test]
    async fn test_authenticate_missing_access_key_file() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-auth-key");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let id_path = dir.join("access-id");
        std::fs::write(&id_path, "test-id").unwrap();

        let config = Config {
            auth: crate::config::AuthConfig {
                access_id_file: id_path.to_string_lossy().to_string(),
                access_key_file: "/tmp/nonexistent-akeyless-key-12345".to_string(),
            },
            api_url: "https://api.akeyless.io".to_string(),
            cache: crate::config::CacheConfig::default(),
        };

        let result = authenticate(&config).await;
        assert!(result.is_err(), "missing access-key file should fail");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("reading access-key"),
            "error should mention reading access-key: {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_authenticate_trims_whitespace() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-auth-trim");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let id_path = dir.join("access-id");
        let key_path = dir.join("access-key");
        std::fs::write(&id_path, "  test-id  \n").unwrap();
        std::fs::write(&key_path, "\ttest-key\n").unwrap();

        let config = Config {
            auth: crate::config::AuthConfig {
                access_id_file: id_path.to_string_lossy().to_string(),
                access_key_file: key_path.to_string_lossy().to_string(),
            },
            api_url: "https://api.fake.invalid".to_string(),
            cache: crate::config::CacheConfig::default(),
        };

        // This will fail at the API call (invalid URL), but should get past
        // reading the files. The error should NOT be about file reading.
        let result = authenticate(&config).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("reading access-id") && !err.contains("reading access-key"),
            "files should be read successfully (error should be about auth, not file reading): {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
