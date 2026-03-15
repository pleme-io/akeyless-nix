use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub secrets: Vec<SecretSpec>,
    #[serde(default)]
    pub templates: Vec<TemplateSpec>,
    pub generations_dir: String,
    pub symlink_path: String,
    #[serde(default = "default_keep")]
    pub keep_generations: u32,
}

fn default_keep() -> u32 {
    2
}

#[derive(Debug, Deserialize)]
pub struct SecretSpec {
    /// Path in Akeyless vault (e.g., "/pleme/prod/db-password")
    pub akeyless_path: String,
    /// Local file path to write the secret value
    pub file_path: String,
    /// File permission mode (octal string, e.g., "0600")
    #[serde(default = "default_mode")]
    pub mode: String,
    /// File owner (username or empty for current user)
    #[serde(default)]
    pub owner: String,
    /// File group (group name or empty for current group)
    #[serde(default)]
    pub group: String,
}

fn default_mode() -> String {
    "0400".to_string()
}

#[derive(Debug, Deserialize)]
pub struct TemplateSpec {
    /// Template name (for symlink naming)
    pub name: String,
    /// Template content with placeholders
    pub content: String,
    /// Local file path to write the rendered template
    pub file_path: String,
    /// File permission mode
    #[serde(default = "default_mode")]
    pub mode: String,
    /// File owner
    #[serde(default)]
    pub owner: String,
    /// File group
    #[serde(default)]
    pub group: String,
}

pub fn load(path: &Path) -> Result<Manifest> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading manifest {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("parsing manifest {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_manifest() {
        let json = r#"{
            "secrets": [
                {
                    "akeyless_path": "/pleme/test/hello",
                    "file_path": "/tmp/hello",
                    "mode": "0600"
                }
            ],
            "templates": [],
            "generations_dir": "/tmp/gens",
            "symlink_path": "/tmp/current",
            "keep_generations": 2
        }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.secrets.len(), 1);
        assert_eq!(manifest.secrets[0].akeyless_path, "/pleme/test/hello");
        assert_eq!(manifest.secrets[0].mode, "0600");
        assert_eq!(manifest.keep_generations, 2);
    }

    #[test]
    fn test_default_mode() {
        let json = r#"{
            "secrets": [{"akeyless_path": "/test", "file_path": "/tmp/test"}],
            "templates": [],
            "generations_dir": "/tmp/g",
            "symlink_path": "/tmp/s"
        }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.secrets[0].mode, "0400");
        assert_eq!(manifest.keep_generations, 2);
    }
}
