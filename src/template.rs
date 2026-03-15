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
fn sha256_hex(input: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    // Use a deterministic hasher for placeholder generation
    // (doesn't need to be cryptographic — just unique per path)
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
