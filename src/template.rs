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
pub(crate) fn render_all(
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
    pub fn with_syntax(syntax: igata::Syntax) -> Self {
        Self {
            renderer: igata::MiniJinjaRenderer::new(syntax),
        }
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
pub(crate) fn sanitize_var_name(path: &str) -> String {
    path.trim_start_matches('/')
        .replace(['/', '-', '.'], "_")
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
        assert_eq!(sanitize_var_name("//pleme/token"), "pleme_token");
    }

    #[test]
    fn sanitize_trailing_slash() {
        assert_eq!(sanitize_var_name("/pleme/token/"), "pleme_token_");
    }

    #[test]
    fn sanitize_dots_replaced() {
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

    // ── IgataEngine::with_syntax test ─────────────────────────────────

    #[test]
    fn igata_with_custom_syntax() {
        let syntax = igata::Syntax {
            variable: ("{{".to_string(), "}}".to_string()),
            block: ("{%".to_string(), "%}".to_string()),
            comment: ("{#".to_string(), "#}".to_string()),
        };
        let engine = IgataEngine::with_syntax(syntax);
        let mut secrets = BTreeMap::new();
        secrets.insert("/app/token".to_string(), "secret123".to_string());

        let result = engine
            .render("key={{ app_token }}", &secrets)
            .unwrap();
        assert_eq!(result, "key=secret123");
    }

    #[test]
    fn igata_default_is_same_as_new() {
        let eng1 = IgataEngine::new();
        let eng2 = IgataEngine::default();
        let mut secrets = BTreeMap::new();
        secrets.insert("/x".to_string(), "val".to_string());

        let r1 = eng1.render("[= x =]", &secrets).unwrap();
        let r2 = eng2.render("[= x =]", &secrets).unwrap();
        assert_eq!(r1, r2);
        assert_eq!(r1, "val");
    }

    // ── Placeholder edge cases ────────────────────────────────────────

    #[test]
    fn placeholder_unreplaced_tokens_pass_through() {
        let engine = PlaceholderEngine;
        let secrets = BTreeMap::new();
        let content = "token: <AKEYLESS:deadbeef:PLACEHOLDER>";
        let result = engine.render(content, &secrets).unwrap();
        assert_eq!(
            result, content,
            "unreplaced placeholder should pass through unchanged"
        );
    }

    #[test]
    fn placeholder_same_secret_twice_in_template() {
        let engine = PlaceholderEngine;
        let mut secrets = BTreeMap::new();
        secrets.insert("/token".to_string(), "abc".to_string());

        let hash = sha256_hex("/token");
        let content = format!(
            "first=<AKEYLESS:{hash}:PLACEHOLDER> second=<AKEYLESS:{hash}:PLACEHOLDER>"
        );
        let result = engine.render(&content, &secrets).unwrap();
        assert_eq!(result, "first=abc second=abc");
    }

    #[test]
    fn placeholder_empty_secret_value() {
        let engine = PlaceholderEngine;
        let mut secrets = BTreeMap::new();
        secrets.insert("/empty".to_string(), String::new());

        let hash = sha256_hex("/empty");
        let content = format!("val=<AKEYLESS:{hash}:PLACEHOLDER>");
        let result = engine.render(&content, &secrets).unwrap();
        assert_eq!(result, "val=");
    }

    #[test]
    fn placeholder_value_containing_placeholder_syntax() {
        let engine = PlaceholderEngine;
        let mut secrets = BTreeMap::new();
        secrets.insert(
            "/meta".to_string(),
            "<AKEYLESS:fake:PLACEHOLDER>".to_string(),
        );

        let hash = sha256_hex("/meta");
        let content = format!("v=<AKEYLESS:{hash}:PLACEHOLDER>");

        let result = engine.render(&content, &secrets).unwrap();
        assert_eq!(result, "v=<AKEYLESS:fake:PLACEHOLDER>");
    }

    // ── IgataEngine error/edge tests ─────────────────────────────────

    #[test]
    fn igata_undefined_variable_renders_empty() {
        let engine = IgataEngine::new();
        let result = engine.render("[= undefined_var =]", &BTreeMap::new());
        assert!(result.is_ok(), "igata renders undefined vars as empty");
    }

    #[test]
    fn igata_empty_template() {
        let engine = IgataEngine::new();
        let result = engine.render("", &BTreeMap::new()).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn igata_special_chars_in_value() {
        let engine = IgataEngine::new();
        let mut secrets = BTreeMap::new();
        secrets.insert("/key".to_string(), "p@ss$w0rd!&<>\"'\\".to_string());

        let result = engine
            .render("pass=[= key =]", &secrets)
            .unwrap();
        assert_eq!(result, "pass=p@ss$w0rd!&<>\"'\\");
    }

    #[test]
    fn igata_newline_in_value() {
        let engine = IgataEngine::new();
        let mut secrets = BTreeMap::new();
        secrets.insert("/cert".to_string(), "line1\nline2\nline3".to_string());

        let result = engine.render("cert=[= cert =]", &secrets).unwrap();
        assert_eq!(result, "cert=line1\nline2\nline3");
    }

    // ── sanitize_var_name edge cases ──────────────────────────────────

    #[test]
    fn sanitize_only_slashes() {
        assert_eq!(sanitize_var_name("///"), "");
    }

    #[test]
    fn sanitize_only_slash() {
        assert_eq!(sanitize_var_name("/"), "");
    }

    #[test]
    fn sanitize_mixed_special_chars() {
        assert_eq!(
            sanitize_var_name("/a.b-c/d-e.f"),
            "a_b_c_d_e_f"
        );
    }

    #[test]
    fn sanitize_multiple_consecutive_slashes() {
        assert_eq!(sanitize_var_name("///a///b///"), "a___b___");
    }

    #[test]
    fn sanitize_mixed_separators() {
        assert_eq!(
            sanitize_var_name("/my-app.v2/db-host"),
            "my_app_v2_db_host"
        );
    }

    // ── render_all metadata preservation ──────────────────────────────

    #[test]
    fn render_all_preserves_template_metadata() {
        let engine = PlaceholderEngine;
        let secrets = BTreeMap::new();

        let templates = vec![TemplateSpec {
            name: "myconfig".into(),
            content: "plain text".into(),
            file_path: "/opt/app/config".into(),
            mode: "0644".into(),
            owner: "app".into(),
            group: "app".into(),
            uid: Some(1000),
            gid: Some(1000),
        }];

        let rendered = render_all(&engine, &templates, &secrets).unwrap();
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0].name, "myconfig");
        assert_eq!(rendered[0].file_path, "/opt/app/config");
        assert_eq!(rendered[0].mode, "0644");
        assert_eq!(rendered[0].owner, "app");
        assert_eq!(rendered[0].group, "app");
        assert_eq!(rendered[0].uid, Some(1000));
        assert_eq!(rendered[0].gid, Some(1000));
        assert_eq!(rendered[0].content, "plain text");
    }

    #[test]
    fn render_all_multiple_templates() {
        let engine = IgataEngine::new();
        let mut secrets = BTreeMap::new();
        secrets.insert("/a".to_string(), "va".to_string());
        secrets.insert("/b".to_string(), "vb".to_string());

        let templates = vec![
            TemplateSpec::for_test("t1", "a=[= a =]", "/tmp/t1"),
            TemplateSpec::for_test("t2", "b=[= b =]", "/tmp/t2"),
            TemplateSpec::for_test("t3", "static", "/tmp/t3"),
        ];

        let rendered = render_all(&engine, &templates, &secrets).unwrap();
        assert_eq!(rendered.len(), 3);
        assert_eq!(rendered[0].content, "a=va");
        assert_eq!(rendered[1].content, "b=vb");
        assert_eq!(rendered[2].content, "static");
    }

    #[test]
    fn render_all_engine_error_propagates() {
        struct FailingEngine;
        impl TemplateEngine for FailingEngine {
            fn render(
                &self,
                _template: &str,
                _secrets: &BTreeMap<String, String>,
            ) -> Result<String> {
                anyhow::bail!("render failed")
            }
        }

        let engine = FailingEngine;
        let templates = vec![TemplateSpec::for_test("t", "content", "/tmp/t")];
        let result = render_all(&engine, &templates, &BTreeMap::new());
        assert!(result.is_err());
    }

    // ── sha256_hex edge cases ─────────────────────────────────────────

    #[test]
    fn sha256_hex_empty_string() {
        let h = sha256_hex("");
        assert_eq!(h.len(), 64);
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_hex_unicode() {
        let h = sha256_hex("こんにちは");
        assert_eq!(h.len(), 64);
        assert_ne!(h, sha256_hex("hello"));
    }

    #[test]
    fn sha256_hex_only_hex_chars() {
        let h = sha256_hex("anything");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ── IgataEngine malformed template ────────────────────────────────

    #[test]
    fn igata_malformed_block_returns_error() {
        let engine = IgataEngine::new();
        let result = engine.render("[% if %]", &BTreeMap::new());
        assert!(result.is_err(), "malformed block should return error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("igata template error"),
            "error should mention igata: {err}"
        );
    }

    #[test]
    fn igata_multiline_template() {
        let engine = IgataEngine::new();
        let mut secrets = BTreeMap::new();
        secrets.insert("/db/host".to_string(), "localhost".to_string());
        secrets.insert("/db/port".to_string(), "5432".to_string());

        let template = "host=[= db_host =]\nport=[= db_port =]";
        let result = engine.render(template, &secrets).unwrap();
        assert_eq!(result, "host=localhost\nport=5432");
    }

    // ── Placeholder with igata syntax (no cross-contamination) ────────

    #[test]
    fn placeholder_ignores_igata_syntax() {
        let engine = PlaceholderEngine;
        let secrets = BTreeMap::new();
        let content = "val=[= some_var =]";
        let result = engine.render(content, &secrets).unwrap();
        assert_eq!(result, content, "PlaceholderEngine should not touch igata syntax");
    }

    #[test]
    fn igata_ignores_akeyless_placeholder_syntax() {
        let engine = IgataEngine::new();
        let secrets = BTreeMap::new();
        let content = "token: <AKEYLESS:abc123:PLACEHOLDER>";
        let result = engine.render(content, &secrets).unwrap();
        assert_eq!(result, content, "IgataEngine should not touch placeholder syntax");
    }

    // ── sanitize_var_name realistic paths ─────────────────────────────

    #[test]
    fn sanitize_realistic_akeyless_paths() {
        assert_eq!(sanitize_var_name("/pleme/prod/db-password"), "pleme_prod_db_password");
        assert_eq!(sanitize_var_name("/org/team/service-v2.1/api-key"), "org_team_service_v2_1_api_key");
        assert_eq!(sanitize_var_name("/ci-cd/github/token"), "ci_cd_github_token");
    }

    // ── render_all stops on first error ───────────────────────────────

    #[test]
    fn render_all_stops_on_first_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        struct CountingFailEngine {
            count: Arc<AtomicUsize>,
        }
        impl TemplateEngine for CountingFailEngine {
            fn render(&self, _: &str, _: &BTreeMap<String, String>) -> Result<String> {
                self.count.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("fail")
            }
        }

        let counter = Arc::new(AtomicUsize::new(0));
        let engine = CountingFailEngine {
            count: counter.clone(),
        };
        let templates = vec![
            TemplateSpec::for_test("a", "aa", "/tmp/a"),
            TemplateSpec::for_test("b", "bb", "/tmp/b"),
            TemplateSpec::for_test("c", "cc", "/tmp/c"),
        ];
        let result = render_all(&engine, &templates, &BTreeMap::new());
        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 1, "should stop on first error");
    }
}
