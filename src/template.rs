use std::collections::BTreeMap;

use anyhow::Result;
use igata::traits::TemplateRenderer;

use crate::manifest::TemplateSpec;
use crate::traits::TemplateEngine;

/// Rendered template with its content and target path.
pub struct RenderedTemplate {
    pub name: String,
    pub content: String,
    pub file_path: String,
    pub mode: String,
    pub owner: String,
    pub group: String,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

/// Render all templates using the provided engine.
pub fn render_all(
    engine: &dyn TemplateEngine,
    templates: &[TemplateSpec],
    secrets: &BTreeMap<String, String>,
) -> Result<Vec<RenderedTemplate>> {
    let mut result = Vec::new();

    for tmpl in templates {
        let rendered_content = engine.render(&tmpl.content, secrets)?;

        result.push(RenderedTemplate {
            name: tmpl.name.clone(),
            content: rendered_content,
            file_path: tmpl.file_path.clone(),
            mode: tmpl.mode.clone(),
            owner: tmpl.owner.clone(),
            group: tmpl.group.clone(),
            uid: tmpl.uid,
            gid: tmpl.gid,
        });
    }

    Ok(result)
}

// ── Placeholder-based engine (legacy, backward compatible) ─────────────

/// Legacy template engine using `<AKEYLESS:{hash}:PLACEHOLDER>` tokens.
///
/// This is the original approach: the Nix module generates placeholder
/// tokens (SHA-256 hash of the akeyless path) which are substituted
/// at activation time.
pub struct PlaceholderEngine;

impl TemplateEngine for PlaceholderEngine {
    fn render(&self, template: &str, secrets: &BTreeMap<String, String>) -> Result<String> {
        let mut result = template.to_string();
        for (path, value) in secrets {
            let hash = sha256_hex(path);
            let placeholder = format!("<AKEYLESS:{hash}:PLACEHOLDER>");
            result = result.replace(&placeholder, value);
        }
        Ok(result)
    }
}

// ── Igata-based engine (MiniJinja, `[= var =]` syntax) ────────────────

/// MiniJinja-backed template engine using igata.
///
/// Template variables are derived from akeyless paths by sanitizing:
/// `/pleme/github/token` → `pleme_github_token`
///
/// Templates use igata's default Nix-safe syntax:
/// ```text
/// token = [= pleme_github_token =]
/// [% if pleme_enable_tls == "true" %]tls = on[% endif %]
/// ```
pub struct IgataEngine {
    renderer: igata::MiniJinjaRenderer,
}

impl IgataEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            renderer: igata::MiniJinjaRenderer::default(),
        }
    }

    /// Create with custom syntax.
    #[allow(dead_code)]
    pub fn with_syntax(syntax: igata::Syntax) -> Result<Self> {
        Ok(Self {
            renderer: igata::MiniJinjaRenderer::new(syntax),
        })
    }
}

impl Default for IgataEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateEngine for IgataEngine {
    fn render(&self, template: &str, secrets: &BTreeMap<String, String>) -> Result<String> {
        let variables: BTreeMap<String, String> = secrets
            .iter()
            .map(|(path, value)| (sanitize_var_name(path), value.clone()))
            .collect();
        self.renderer
            .render(template, &variables)
            .map_err(|e| anyhow::anyhow!("igata template error: {e}"))
    }
}

/// Sanitize an akeyless path to a valid template variable name.
///
/// `/pleme/github/token` → `pleme_github_token`
/// `/pleme/prod/db-password` → `pleme_prod_db_password`
/// `/app.prod/token` → `app_prod_token`
pub fn sanitize_var_name(path: &str) -> String {
    path.trim_start_matches('/')
        .replace('/', "_")
        .replace('-', "_")
        .replace('.', "_")
}

/// SHA-256 hash for placeholder generation.
///
/// Must match `builtins.hashString "sha256"` in the Nix module so that
/// placeholders generated at eval time are found by the Rust binary at
/// activation time.
pub(crate) fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:064x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PlaceholderEngine tests ────────────────────────────────────────

    #[test]
    fn placeholder_replaces_tokens() {
        let engine = PlaceholderEngine;
        let mut secrets = BTreeMap::new();
        secrets.insert("/pleme/token".to_string(), "my-secret-token".to_string());

        let hash = sha256_hex("/pleme/token");
        let content = format!("token: <AKEYLESS:{hash}:PLACEHOLDER>");

        let result = engine.render(&content, &secrets).unwrap();
        assert_eq!(result, "token: my-secret-token");
    }

    #[test]
    fn placeholder_passthrough() {
        let engine = PlaceholderEngine;
        let result = engine.render("plain text", &BTreeMap::new()).unwrap();
        assert_eq!(result, "plain text");
    }

    #[test]
    fn placeholder_multiple() {
        let engine = PlaceholderEngine;
        let mut secrets = BTreeMap::new();
        secrets.insert("/a".to_string(), "val-a".to_string());
        secrets.insert("/b".to_string(), "val-b".to_string());

        let ha = sha256_hex("/a");
        let hb = sha256_hex("/b");
        let content = format!("a=<AKEYLESS:{ha}:PLACEHOLDER> b=<AKEYLESS:{hb}:PLACEHOLDER>");

        let result = engine.render(&content, &secrets).unwrap();
        assert_eq!(result, "a=val-a b=val-b");
    }

    #[test]
    fn placeholder_special_chars() {
        let engine = PlaceholderEngine;
        let mut secrets = BTreeMap::new();
        secrets.insert("/db/conn".to_string(), "p@ss$w0rd!&<>\"'\\".to_string());

        let hash = sha256_hex("/db/conn");
        let content = format!("password: <AKEYLESS:{hash}:PLACEHOLDER>");

        let result = engine.render(&content, &secrets).unwrap();
        assert_eq!(result, "password: p@ss$w0rd!&<>\"'\\");
    }

    #[test]
    fn placeholder_newlines_in_value() {
        let engine = PlaceholderEngine;
        let mut secrets = BTreeMap::new();
        secrets.insert(
            "/cert/tls".to_string(),
            "-----BEGIN CERTIFICATE-----\nMIIBxTCCAW\n-----END CERTIFICATE-----\n".to_string(),
        );

        let hash = sha256_hex("/cert/tls");
        let content = format!("cert: <AKEYLESS:{hash}:PLACEHOLDER>");

        let result = engine.render(&content, &secrets).unwrap();
        assert!(result.contains("BEGIN CERTIFICATE"));
        assert!(!result.contains("AKEYLESS"));
    }

    #[test]
    fn sha256_hex_deterministic() {
        let h1 = sha256_hex("/pleme/token");
        let h2 = sha256_hex("/pleme/token");
        assert_eq!(h1, h2);
        assert_ne!(h1, sha256_hex("/pleme/other"));
    }

    #[test]
    fn sha256_hex_is_64_chars() {
        let h = sha256_hex("/pleme/token");
        assert_eq!(h.len(), 64, "SHA-256 hex digest must be 64 characters, got {}", h.len());
    }

    #[test]
    fn sha256_hex_matches_nix_builtins_hash_string() {
        // Verified via: nix-instantiate --eval -E 'builtins.hashString "sha256" "/pleme/token"'
        let expected = "9600f4f62653c71f3daed10e123128ba7403a07835c4aa5591cd1a2b833aa6ce";
        let actual = sha256_hex("/pleme/token");
        assert_eq!(
            actual, expected,
            "sha256_hex must match Nix's builtins.hashString \"sha256\""
        );
    }

    // ── IgataEngine tests ──────────────────────────────────────────────

    #[test]
    fn igata_renders_variables() {
        let engine = IgataEngine::new();
        let mut secrets = BTreeMap::new();
        secrets.insert("/pleme/github/token".to_string(), "ghp_abc123".to_string());

        let result = engine
            .render("token = [= pleme_github_token =]", &secrets)
            .unwrap();
        assert_eq!(result, "token = ghp_abc123");
    }

    #[test]
    fn igata_renders_multiple() {
        let engine = IgataEngine::new();
        let mut secrets = BTreeMap::new();
        secrets.insert("/pleme/db/pass".to_string(), "secret".to_string());
        secrets.insert("/pleme/db/host".to_string(), "db.example.com".to_string());

        let result = engine
            .render("host=[= pleme_db_host =] pass=[= pleme_db_pass =]", &secrets)
            .unwrap();
        assert_eq!(result, "host=db.example.com pass=secret");
    }

    #[test]
    fn igata_conditional() {
        let engine = IgataEngine::new();
        let mut secrets = BTreeMap::new();
        secrets.insert("/tls/enabled".to_string(), "true".to_string());
        secrets.insert("/tls/cert".to_string(), "/etc/ssl/cert.pem".to_string());

        let template = "[% if tls_enabled == \"true\" %]cert = [= tls_cert =]\n[% endif %]";
        let result = engine.render(template, &secrets).unwrap();
        assert!(result.contains("cert = /etc/ssl/cert.pem"));
    }

    #[test]
    fn igata_passthrough_no_vars() {
        let engine = IgataEngine::new();
        let result = engine.render("static content", &BTreeMap::new()).unwrap();
        assert_eq!(result, "static content");
    }

    #[test]
    fn igata_nix_dollar_brace_not_interpreted() {
        let engine = IgataEngine::new();
        let mut secrets = BTreeMap::new();
        secrets.insert("/val".to_string(), "ok".to_string());

        let result = engine
            .render("nix=${foo} igata=[= val =]", &secrets)
            .unwrap();
        assert_eq!(result, "nix=${foo} igata=ok");
    }

    // ── sanitize_var_name tests ────────────────────────────────────────

    #[test]
    fn sanitize_simple_path() {
        assert_eq!(sanitize_var_name("/pleme/github/token"), "pleme_github_token");
    }

    #[test]
    fn sanitize_dashes() {
        assert_eq!(
            sanitize_var_name("/pleme/prod/db-password"),
            "pleme_prod_db_password"
        );
    }

    #[test]
    fn sanitize_no_leading_slash() {
        assert_eq!(sanitize_var_name("simple"), "simple");
    }

    #[test]
    fn sanitize_double_slashes() {
        // trim_start_matches removes all leading slashes
        assert_eq!(sanitize_var_name("//pleme/token"), "pleme_token");
    }

    #[test]
    fn sanitize_trailing_slash() {
        assert_eq!(sanitize_var_name("/pleme/token/"), "pleme_token_");
    }

    #[test]
    fn sanitize_dots_replaced() {
        // Dots become underscores — MiniJinja treats dots as attribute access
        assert_eq!(sanitize_var_name("/app.prod/token"), "app_prod_token");
    }

    #[test]
    fn sanitize_already_underscored() {
        assert_eq!(sanitize_var_name("/api_key"), "api_key");
    }

    #[test]
    fn sanitize_empty_string() {
        assert_eq!(sanitize_var_name(""), "");
    }

    // ── render_all tests ───────────────────────────────────────────────

    #[test]
    fn render_all_with_placeholder_engine() {
        let engine = PlaceholderEngine;
        let mut secrets = BTreeMap::new();
        secrets.insert("/pleme/token".to_string(), "secret123".to_string());

        let hash = sha256_hex("/pleme/token");
        let templates = vec![TemplateSpec::for_test(
            "config",
            &format!("token: <AKEYLESS:{hash}:PLACEHOLDER>"),
            "/tmp/config",
        )];

        let rendered = render_all(&engine, &templates, &secrets).unwrap();
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0].content, "token: secret123");
        assert_eq!(rendered[0].name, "config");
    }

    #[test]
    fn render_all_with_igata_engine() {
        let engine = IgataEngine::new();
        let mut secrets = BTreeMap::new();
        secrets.insert("/pleme/token".to_string(), "secret123".to_string());

        let templates = vec![TemplateSpec::for_test(
            "config",
            "token: [= pleme_token =]",
            "/tmp/config",
        )];

        let rendered = render_all(&engine, &templates, &secrets).unwrap();
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0].content, "token: secret123");
    }

    #[test]
    fn render_all_empty() {
        let engine = PlaceholderEngine;
        let secrets = BTreeMap::new();
        let rendered = render_all(&engine, &[], &secrets).unwrap();
        assert!(rendered.is_empty());
    }

    // ── Mock engine test ───────────────────────────────────────────────

    #[test]
    fn mock_engine_for_testing() {
        /// A test-only engine that wraps values in markers.
        struct MockEngine;
        impl TemplateEngine for MockEngine {
            fn render(
                &self,
                template: &str,
                _secrets: &BTreeMap<String, String>,
            ) -> Result<String> {
                Ok(format!("<<MOCK>>{template}<</MOCK>>"))
            }
        }

        let engine = MockEngine;
        let result = engine.render("hello", &BTreeMap::new()).unwrap();
        assert_eq!(result, "<<MOCK>>hello<</MOCK>>");
    }
}
