use std::collections::BTreeMap;

use anyhow::Result;

use crate::manifest::TemplateSpec;

/// Rendered template with its content and target path.
pub struct RenderedTemplate {
    pub name: String,
    pub content: String,
    pub file_path: String,
    pub mode: String,
    pub owner: String,
    pub group: String,
}

/// Render all templates by substituting placeholders with secret values.
///
/// Placeholders are in the format `<AKEYLESS:{sha256_of_path}:PLACEHOLDER>`.
/// The secret map is keyed by akeyless_path.
pub fn render_all(
    templates: &[TemplateSpec],
    secrets: &BTreeMap<String, String>,
) -> Result<Vec<RenderedTemplate>> {
    let mut result = Vec::new();

    // Build placeholder → value map
    let placeholder_map: BTreeMap<String, &str> = secrets
        .iter()
        .map(|(path, value)| {
            let hash = sha256_hex(path);
            let placeholder = format!("<AKEYLESS:{hash}:PLACEHOLDER>");
            (placeholder, value.as_str())
        })
        .collect();

    for tmpl in templates {
        let mut rendered = tmpl.content.clone();
        for (placeholder, value) in &placeholder_map {
            rendered = rendered.replace(placeholder, value);
        }

        result.push(RenderedTemplate {
            name: tmpl.name.clone(),
            content: rendered,
            file_path: tmpl.file_path.clone(),
            mode: tmpl.mode.clone(),
            owner: tmpl.owner.clone(),
            group: tmpl.group.clone(),
        });
    }

    Ok(result)
}

/// Simple SHA-256 hex digest for placeholder generation.
pub(crate) fn sha256_hex(input: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    // Use a deterministic hasher for placeholder generation
    // (doesn't need to be cryptographic — just unique per path)
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_replaces_placeholders() {
        let mut secrets = BTreeMap::new();
        secrets.insert("/pleme/token".to_string(), "my-secret-token".to_string());

        let hash = sha256_hex("/pleme/token");
        let placeholder = format!("<AKEYLESS:{hash}:PLACEHOLDER>");

        let templates = vec![TemplateSpec {
            name: "config".to_string(),
            content: format!("token: {placeholder}"),
            file_path: "/tmp/config".to_string(),
            mode: "0600".to_string(),
            owner: String::new(),
            group: String::new(),
        }];

        let rendered = render_all(&templates, &secrets).unwrap();
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0].content, "token: my-secret-token");
    }

    #[test]
    fn test_render_multiple_placeholders() {
        let mut secrets = BTreeMap::new();
        secrets.insert("/a".to_string(), "val-a".to_string());
        secrets.insert("/b".to_string(), "val-b".to_string());

        let ha = sha256_hex("/a");
        let hb = sha256_hex("/b");

        let templates = vec![TemplateSpec {
            name: "multi".to_string(),
            content: format!("a=<AKEYLESS:{ha}:PLACEHOLDER> b=<AKEYLESS:{hb}:PLACEHOLDER>"),
            file_path: "/tmp/multi".to_string(),
            mode: "0600".to_string(),
            owner: String::new(),
            group: String::new(),
        }];

        let rendered = render_all(&templates, &secrets).unwrap();
        assert_eq!(rendered[0].content, "a=val-a b=val-b");
    }
}
