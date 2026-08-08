//! Template rendering engine — the typed surface `src/template.rs` builds on.
//!
//! # Why this lives here instead of in a dependency
//!
//! This was `igata`, a pleme-io crate. On 2026-03-27 commit `7477921`
//! ("rewrite igata as Nix-first Packer-compatible machine image builder")
//! replaced that crate wholesale *in the same repo*: it renamed the package to
//! `pleme-igata`, deleted `src/lib.rs`, and shipped an unrelated machine-image
//! builder. So the repo name is continuous and the CRATE is not — the thing
//! this file needs has no published home and no longer exists at its repo's
//! HEAD. `pleme-igata` 0.1.5 on crates.io is the machine-image builder and is
//! binary-only; depending on it yields `E0433: unresolved crate igata`.
//!
//! That left a bare `git =` dependency as the only coordinate, which makes a
//! crate **structurally unpublishable** — cargo refuses to package a dependency
//! with no version requirement — so it blocked this repo from releasing at all.
//!
//! The recovered surface is ~120 lines over `minijinja`, and akeyless-nix was
//! its only consumer, so it comes home rather than being resurrected as a crate
//! nobody else would import. Ported 1:1 from `igata@7477921~1`
//! (`src/{syntax,traits,defaults}.rs`) so the diff stays reviewable against
//! what it replaces.
//!
//! `TemplateRenderer` stays a **trait**, not a concrete type: swapping MiniJinja
//! for Tera/Handlebars/plain-substitution must remain a one-impl change.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use minijinja::syntax::SyntaxConfig;
use minijinja::Environment;

/// Errors raised while configuring or rendering a template.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The delimiter set was rejected by the template backend.
    #[error("invalid template syntax configuration: {0}")]
    Syntax(String),

    /// The template failed to parse or render.
    #[error("template render failed: {0}")]
    Render(#[from] minijinja::Error),
}

/// Result alias for template operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Delimiter configuration for templates.
///
/// Defaults to `[= =]` for variables, `[% %]` for blocks, `[# #]` for comments.
/// These defaults deliberately avoid Nix `${}` interpolation, shell `$VAR`, and
/// the brace syntax common to config formats — this crate renders templates
/// *into* Nix files, so a `{{ }}` default would collide with the output
/// language rather than with the input.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Syntax {
    /// Variable delimiters, e.g. `["[=", "=]"]`.
    #[serde(default = "Syntax::default_variable")]
    pub variable: (String, String),

    /// Block delimiters, e.g. `["[%", "%]"]`.
    #[serde(default = "Syntax::default_block")]
    pub block: (String, String),

    /// Comment delimiters, e.g. `["[#", "#]"]`.
    #[serde(default = "Syntax::default_comment")]
    pub comment: (String, String),
}

impl Default for Syntax {
    fn default() -> Self {
        Self {
            variable: Self::default_variable(),
            block: Self::default_block(),
            comment: Self::default_comment(),
        }
    }
}

impl Syntax {
    fn default_variable() -> (String, String) {
        ("[=".into(), "=]".into())
    }

    fn default_block() -> (String, String) {
        ("[%".into(), "%]".into())
    }

    fn default_comment() -> (String, String) {
        ("[#".into(), "#]".into())
    }

    /// Convert to a MiniJinja `SyntaxConfig`.
    ///
    /// MiniJinja's builder requires `&'static str` for delimiters, so the
    /// strings are leaked. Callers must go through [`MiniJinjaRenderer`], which
    /// caches the result in a `OnceLock` — calling this directly in a loop
    /// leaks on every iteration.
    pub fn to_config(&self) -> Result<SyntaxConfig> {
        let var_open: &'static str = self.variable.0.clone().leak();
        let var_close: &'static str = self.variable.1.clone().leak();
        let blk_open: &'static str = self.block.0.clone().leak();
        let blk_close: &'static str = self.block.1.clone().leak();
        let cmt_open: &'static str = self.comment.0.clone().leak();
        let cmt_close: &'static str = self.comment.1.clone().leak();

        SyntaxConfig::builder()
            .variable_delimiters(var_open, var_close)
            .block_delimiters(blk_open, blk_close)
            .comment_delimiters(cmt_open, cmt_close)
            .build()
            .map_err(|e| Error::Syntax(e.to_string()))
    }
}

/// Renders a template string with resolved variables.
///
/// Implement this to swap the template backend without touching any caller.
pub trait TemplateRenderer: std::fmt::Debug + Send + Sync {
    /// Render `template`, substituting `variables`.
    ///
    /// # Errors
    /// Returns [`Error`] if the syntax config is invalid or the template fails
    /// to parse/render.
    fn render(&self, template: &str, variables: &BTreeMap<String, String>) -> Result<String>;
}

/// MiniJinja-backed renderer using Nix-safe delimiters.
#[derive(Debug)]
pub struct MiniJinjaRenderer {
    syntax: Syntax,
    cached_config: OnceLock<SyntaxConfig>,
}

impl MiniJinjaRenderer {
    /// Build a renderer over the given delimiter set.
    #[must_use]
    pub fn new(syntax: Syntax) -> Self {
        Self {
            syntax,
            cached_config: OnceLock::new(),
        }
    }

    fn syntax_config(&self) -> Result<&SyntaxConfig> {
        if let Some(config) = self.cached_config.get() {
            return Ok(config);
        }
        let config = self.syntax.to_config()?;
        // If another thread raced us, that's fine — we just discard ours.
        let _ = self.cached_config.set(config);
        Ok(self.cached_config.get().expect("just set"))
    }

    /// Eagerly validate the syntax by building and caching the config, so a bad
    /// delimiter set surfaces at construction rather than at first render.
    ///
    /// Unused today for the same reason `TemplateEngine::with_syntax` is: every
    /// caller currently takes the Nix-safe defaults, which are validated by the
    /// unit test below. It stays because it is the entry point a custom-syntax
    /// caller needs, and re-deriving it later would be strictly harder than
    /// keeping the ported original.
    ///
    /// # Errors
    /// Returns [`Error::Syntax`] if the delimiters are rejected.
    #[allow(dead_code)]
    pub fn validate(&self) -> Result<()> {
        self.syntax_config()?;
        Ok(())
    }
}

impl Default for MiniJinjaRenderer {
    fn default() -> Self {
        Self::new(Syntax::default())
    }
}

impl TemplateRenderer for MiniJinjaRenderer {
    fn render(&self, template: &str, variables: &BTreeMap<String, String>) -> Result<String> {
        let config = self.syntax_config()?;
        let mut env = Environment::new();
        env.set_syntax(config.clone());
        let tmpl = env.template_from_str(template)?;
        Ok(tmpl.render(variables)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn renders_a_variable_with_the_nix_safe_delimiters() {
        let r = MiniJinjaRenderer::default();
        let out = r.render("token=[= tok =]", &vars(&[("tok", "abc")])).unwrap();
        assert_eq!(out, "token=abc");
    }

    #[test]
    fn leaves_nix_interpolation_untouched() {
        // The whole point of the `[= =]` default: this crate renders INTO Nix,
        // so `${...}` must survive verbatim rather than being consumed.
        let r = MiniJinjaRenderer::default();
        let out = r
            .render("nix=${foo} tpl=[= v =]", &vars(&[("v", "ok")]))
            .unwrap();
        assert_eq!(out, "nix=${foo} tpl=ok");
    }

    #[test]
    fn a_block_conditional_evaluates() {
        let r = MiniJinjaRenderer::default();
        let out = r
            .render("[% if on %]yes[% else %]no[% endif %]", &vars(&[("on", "1")]))
            .unwrap();
        assert_eq!(out, "yes");
    }

    #[test]
    fn a_template_with_no_variables_passes_through() {
        let r = MiniJinjaRenderer::default();
        assert_eq!(r.render("plain text", &vars(&[])).unwrap(), "plain text");
    }

    #[test]
    fn an_unparseable_template_is_an_error_not_a_panic() {
        let r = MiniJinjaRenderer::default();
        assert!(r.render("[% if unclosed %]", &vars(&[])).is_err());
    }

    #[test]
    fn validate_accepts_the_default_delimiters() {
        assert!(MiniJinjaRenderer::default().validate().is_ok());
    }
}
