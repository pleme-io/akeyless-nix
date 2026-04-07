use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::manifest::Manifest;
use crate::template::RenderedTemplate;
use crate::traits::FileWriter;
use crate::write::{self, Ownership};

/// A created generation, with its sequence number and filesystem path.
#[must_use]
pub(crate) struct Generation {
    /// Monotonically increasing generation number.
    pub number: u64,
    /// Absolute path to the generation directory.
    pub path: PathBuf,
}

impl fmt::Display for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "generation {} at {}", self.number, self.path.display())
    }
}

/// Create a new generation directory and write all secrets + templates.
pub(crate) fn create(
    manifest: &Manifest,
    secrets: &BTreeMap<String, String>,
    templates: &[RenderedTemplate],
    ignore_passwd: bool,
) -> Result<Generation> {
    create_with_writer(
        manifest,
        secrets,
        templates,
        ignore_passwd,
        &crate::write::FsFileWriter,
    )
}

/// Create a new generation directory using the provided `FileWriter`.
pub(crate) fn create_with_writer(
    manifest: &Manifest,
    secrets: &BTreeMap<String, String>,
    templates: &[RenderedTemplate],
    ignore_passwd: bool,
    writer: &dyn FileWriter,
) -> Result<Generation> {
    let gd = Path::new(&manifest.generations_dir);
    writer
        .create_dir_all(gd)
        .with_context(|| format!("creating generations dir {}", gd.display()))?;

    // Find next generation number
    let number = next_generation(gd)?;
    let gp = gd.join(number.to_string());
    writer.create_dir_all(&gp)?;

    // Write secrets
    for spec in &manifest.secrets {
        if let Some(value) = secrets.get(&spec.akeyless_path) {
            let target = gp.join(sanitize_name(&spec.akeyless_path));
            write::write_secret_with_ownership(
                &target,
                value,
                &spec.mode,
                ignore_passwd,
                &Ownership::from(spec),
            )?;
        }
    }

    // Write rendered templates
    let tmpl_dir = gp.join("rendered");
    writer.create_dir_all(&tmpl_dir)?;
    for tmpl in templates {
        let target = tmpl_dir.join(&tmpl.name);
        write::write_secret_with_ownership(
            &target,
            &tmpl.content,
            &tmpl.mode,
            ignore_passwd,
            &Ownership::from(tmpl),
        )?;
    }

    Ok(Generation {
        number,
        path: gp,
    })
}

/// Atomically switch the symlink to point to the new generation.
pub(crate) fn switch(
    manifest: &Manifest,
    genr: &Generation,
    rendered_templates: &[RenderedTemplate],
) -> Result<()> {
    switch_with_writer(manifest, genr, rendered_templates, &crate::write::FsFileWriter)
}

/// Switch using the provided `FileWriter`.
///
/// Creates symlinks from each secret's declared `file_path` to the corresponding
/// file in the generation directory, and from each rendered template's `file_path`
/// to the rendered output.
pub(crate) fn switch_with_writer(
    manifest: &Manifest,
    genr: &Generation,
    rendered_templates: &[RenderedTemplate],
    writer: &dyn FileWriter,
) -> Result<()> {
    let symlink = Path::new(&manifest.symlink_path);

    // Create symlinks from declared file_path -> generation file
    for spec in &manifest.secrets {
        let gf = genr.path.join(sanitize_name(&spec.akeyless_path));
        let target = Path::new(&spec.file_path);

        if let Some(parent) = target.parent() {
            writer.create_dir_all(parent)?;
        }

        // Remove old symlink/file if it exists
        writer.remove_file(target)?;
        writer
            .symlink(&gf, target)
            .with_context(|| format!("symlinking {} -> {}", target.display(), gf.display()))?;
    }

    // Create symlinks for rendered templates using file_path from RenderedTemplate
    for tmpl in rendered_templates {
        let gf = genr.path.join("rendered").join(&tmpl.name);
        let target = Path::new(&tmpl.file_path);

        if let Some(parent) = target.parent() {
            writer.create_dir_all(parent)?;
        }

        writer.remove_file(target)?;
        writer
            .symlink(&gf, target)
            .with_context(|| format!("symlinking {} -> {}", target.display(), gf.display()))?;
    }

    // Update the main symlink
    if let Some(parent) = symlink.parent() {
        writer.create_dir_all(parent)?;
    }
    writer.remove_file(symlink)?;
    writer
        .symlink(&genr.path, symlink)
        .with_context(|| format!("switching generation symlink to {}", genr.path.display()))?;

    Ok(())
}

/// Remove old generations beyond the keep limit.
pub(crate) fn prune(manifest: &Manifest) -> Result<()> {
    let gd = Path::new(&manifest.generations_dir);
    if !gd.exists() {
        return Ok(());
    }

    let mut gens = list_generation_numbers(gd)?;
    gens.sort_unstable();

    let keep = manifest.keep_generations as usize;
    if gens.len() <= keep {
        return Ok(());
    }

    for gn in &gens[..gens.len() - keep] {
        let path = gd.join(gn.to_string());
        let _ = std::fs::remove_dir_all(&path);
    }

    Ok(())
}

fn next_generation(gd: &Path) -> Result<u64> {
    let max = list_generation_numbers(gd)?
        .into_iter()
        .max()
        .unwrap_or(0);
    Ok(max + 1)
}

/// Scan a directory for numeric subdirectory names, returning them as u64.
fn list_generation_numbers(gd: &Path) -> Result<Vec<u64>> {
    Ok(std::fs::read_dir(gd)?
        .filter_map(std::result::Result::ok)
        .filter_map(|e| e.file_name().to_str()?.parse::<u64>().ok())
        .collect())
}

/// Convert an Akeyless path to a safe filename.
/// "/pleme/prod/db-password" -> "pleme-prod-db-password"
fn sanitize_name(path: &str) -> String {
    path.trim_start_matches('/')
        .replace('/', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_name("/pleme/prod/db-password"), "pleme-prod-db-password");
        assert_eq!(sanitize_name("/a/b/c"), "a-b-c");
        assert_eq!(sanitize_name("no-slash"), "no-slash");
    }

    #[test]
    fn test_generation_display() {
        let genr = Generation {
            number: 42,
            path: PathBuf::from("/tmp/generations/42"),
        };
        assert_eq!(genr.to_string(), "generation 42 at /tmp/generations/42");
    }

    #[test]
    fn test_generation_lifecycle() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-gen");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest = Manifest {
            secrets: vec![],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let secrets = BTreeMap::new();
        let templates = vec![];

        let g1 = create(&manifest, &secrets, &templates, true).unwrap();
        assert_eq!(g1.number, 1);

        let g2 = create(&manifest, &secrets, &templates, true).unwrap();
        assert_eq!(g2.number, 2);

        let g3 = create(&manifest, &secrets, &templates, true).unwrap();
        assert_eq!(g3.number, 3);

        prune(&manifest).unwrap();

        let mut remaining: Vec<u64> = std::fs::read_dir(dir.join("generations")).unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str()?.parse::<u64>().ok())
            .collect();
        remaining.sort();

        assert_eq!(remaining.len(), 2);
        assert!(remaining.contains(&2));
        assert!(remaining.contains(&3));
        assert!(!remaining.contains(&1));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_generation_with_fs_file_writer() {
        use crate::manifest::SecretSpec;
        use crate::write::FsFileWriter;

        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-writer");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let secret_target = dir.join("targets").join("db-password");
        let manifest = Manifest {
            secrets: vec![SecretSpec::for_test(
                "/pleme/prod/db-password",
                &secret_target.to_string_lossy(),
            )],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let mut secrets = BTreeMap::new();
        secrets.insert("/pleme/prod/db-password".into(), "s3cret".into());

        let writer = FsFileWriter;
        let generation = create_with_writer(&manifest, &secrets, &[], true, &writer).unwrap();
        assert_eq!(generation.number, 1);

        let gen_file = generation.path.join("pleme-prod-db-password");
        assert_eq!(std::fs::read_to_string(&gen_file).unwrap(), "s3cret");

        switch_with_writer(&manifest, &generation, &[], &writer).unwrap();

        assert!(secret_target.is_symlink());
        assert_eq!(std::fs::read_to_string(&secret_target).unwrap(), "s3cret");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_full_flow_with_templates() {
        use crate::manifest::{SecretSpec, TemplateSpec};
        use crate::template::RenderedTemplate;
        use crate::write::FsFileWriter;

        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-full");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let secret_target = dir.join("targets").join("token");
        let tmpl_target = dir.join("targets").join("config.yaml");

        let manifest = Manifest {
            secrets: vec![SecretSpec::for_test(
                "/app/token",
                &secret_target.to_string_lossy(),
            )],
            templates: vec![TemplateSpec::for_test(
                "config.yaml",
                "token: PLACEHOLDER",
                &tmpl_target.to_string_lossy(),
            )],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let mut secrets = BTreeMap::new();
        secrets.insert("/app/token".into(), "my-token".into());

        let rendered = vec![RenderedTemplate {
            name: "config.yaml".into(),
            content: "token: my-token".into(),
            file_path: tmpl_target.to_string_lossy().to_string(),
            mode: "0600".into(),
            owner: String::new(),
            group: String::new(),
            uid: None,
            gid: None,
        }];

        let writer = FsFileWriter;
        let generation = create_with_writer(&manifest, &secrets, &rendered, true, &writer).unwrap();
        switch_with_writer(&manifest, &generation, &rendered, &writer).unwrap();

        assert!(secret_target.is_symlink());
        assert_eq!(std::fs::read_to_string(&secret_target).unwrap(), "my-token");

        assert!(tmpl_target.is_symlink());
        assert_eq!(std::fs::read_to_string(&tmpl_target).unwrap(), "token: my-token");

        let current = dir.join("current");
        assert!(current.is_symlink());
        assert_eq!(std::fs::read_link(&current).unwrap(), generation.path);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prune_fewer_than_keep() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-prune-few");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest = Manifest {
            secrets: vec![],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 5,
        };

        let secrets = BTreeMap::new();
        let templates = vec![];

        let g1 = create(&manifest, &secrets, &templates, true).unwrap();
        assert_eq!(g1.number, 1);
        let g2 = create(&manifest, &secrets, &templates, true).unwrap();
        assert_eq!(g2.number, 2);

        prune(&manifest).unwrap();

        let remaining: Vec<u64> = std::fs::read_dir(dir.join("generations"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str()?.parse::<u64>().ok())
            .collect();
        assert_eq!(remaining.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prune_nonexistent_dir() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-prune-nodir");
        let _ = std::fs::remove_dir_all(&dir);

        let manifest = Manifest {
            secrets: vec![],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        prune(&manifest).unwrap();
    }

    #[test]
    fn test_empty_manifest() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-empty-manifest");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest = Manifest {
            secrets: vec![],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let secrets = BTreeMap::new();
        let rendered: Vec<RenderedTemplate> = vec![];

        let generation = create(&manifest, &secrets, &rendered, true).unwrap();
        assert_eq!(generation.number, 1);
        assert!(generation.path.exists());

        switch(&manifest, &generation, &rendered).unwrap();

        let current = dir.join("current");
        assert!(current.is_symlink());
        assert_eq!(std::fs::read_link(&current).unwrap(), generation.path);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sanitize_name_leading_slashes() {
        assert_eq!(sanitize_name("///a/b"), "a-b");
    }

    #[test]
    fn test_sanitize_name_empty() {
        assert_eq!(sanitize_name(""), "");
    }

    #[test]
    fn test_sanitize_name_slash_only() {
        assert_eq!(sanitize_name("/"), "");
    }

    #[test]
    fn test_sanitize_name_preserves_dots_and_underscores() {
        assert_eq!(sanitize_name("/app.prod/db_pass"), "app.prod-db_pass");
    }

    #[test]
    fn test_sanitize_name_edge_cases() {
        assert_eq!(sanitize_name("///"), "");
        assert_eq!(sanitize_name("/a"), "a");
        assert_eq!(sanitize_name("plain"), "plain");
        assert_eq!(sanitize_name("/a/b.c/d-e"), "a-b.c-d-e");
    }

    #[test]
    fn test_next_generation_empty_dir() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-nextgen-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let number = next_generation(&dir).unwrap();
        assert_eq!(number, 1, "first generation in empty dir should be 1");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_next_generation_with_existing() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-next-existing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::create_dir_all(dir.join("5")).unwrap();
        std::fs::create_dir_all(dir.join("3")).unwrap();
        std::fs::create_dir_all(dir.join("10")).unwrap();

        let n = next_generation(&dir).unwrap();
        assert_eq!(n, 11);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_next_generation_skips_non_numeric_entries() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-nextgen-nonnumeric");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::create_dir_all(dir.join("5")).unwrap();
        std::fs::create_dir_all(dir.join("not-a-number")).unwrap();
        std::fs::create_dir_all(dir.join("abc")).unwrap();
        std::fs::write(dir.join("file.txt"), "data").unwrap();

        let number = next_generation(&dir).unwrap();
        assert_eq!(number, 6, "should use max numeric + 1, ignoring non-numeric entries");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prune_keeps_exactly_keep_generations() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-prune-exact");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest = Manifest {
            secrets: vec![],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let secrets = BTreeMap::new();
        let templates = vec![];

        for _ in 0..5 {
            create(&manifest, &secrets, &templates, true).unwrap();
        }

        prune(&manifest).unwrap();

        let remaining: Vec<u64> = std::fs::read_dir(dir.join("generations"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str()?.parse::<u64>().ok())
            .collect();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.contains(&4));
        assert!(remaining.contains(&5));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prune_ignores_non_numeric_entries() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-prune-nonnumeric");
        let _ = std::fs::remove_dir_all(&dir);

        let gen_dir = dir.join("generations");
        std::fs::create_dir_all(&gen_dir).unwrap();
        std::fs::create_dir_all(gen_dir.join("1")).unwrap();
        std::fs::create_dir_all(gen_dir.join("2")).unwrap();
        std::fs::create_dir_all(gen_dir.join("3")).unwrap();
        std::fs::create_dir_all(gen_dir.join("metadata")).unwrap();

        let manifest = Manifest {
            secrets: vec![],
            templates: vec![],
            generations_dir: gen_dir.to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        prune(&manifest).unwrap();

        assert!(!gen_dir.join("1").exists(), "gen 1 should be pruned");
        assert!(gen_dir.join("2").exists(), "gen 2 should remain");
        assert!(gen_dir.join("3").exists(), "gen 3 should remain");
        assert!(gen_dir.join("metadata").exists(), "non-numeric should be untouched");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prune_with_keep_one() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-prune-keepone");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest = Manifest {
            secrets: vec![],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 1,
        };

        let secrets = BTreeMap::new();
        let templates = vec![];

        create(&manifest, &secrets, &templates, true).unwrap();
        create(&manifest, &secrets, &templates, true).unwrap();
        create(&manifest, &secrets, &templates, true).unwrap();

        prune(&manifest).unwrap();

        let remaining: Vec<u64> = std::fs::read_dir(dir.join("generations"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str()?.parse::<u64>().ok())
            .collect();
        assert_eq!(remaining.len(), 1);
        assert!(remaining.contains(&3));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_switch_updates_symlink_on_new_generation() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-switch-update");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest = Manifest {
            secrets: vec![],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 5,
        };

        let secrets = BTreeMap::new();
        let templates: Vec<RenderedTemplate> = vec![];

        let g1 = create(&manifest, &secrets, &templates, true).unwrap();
        switch(&manifest, &g1, &templates).unwrap();
        assert_eq!(std::fs::read_link(dir.join("current")).unwrap(), g1.path);

        let g2 = create(&manifest, &secrets, &templates, true).unwrap();
        switch(&manifest, &g2, &templates).unwrap();
        assert_eq!(
            std::fs::read_link(dir.join("current")).unwrap(),
            g2.path,
            "symlink should point to newest generation after switch"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_switch_replaces_existing_symlinks() {
        use crate::manifest::SecretSpec;
        use crate::write::FsFileWriter;

        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-switch-replace");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let secret_target = dir.join("targets").join("secret");
        let manifest = Manifest {
            secrets: vec![SecretSpec::for_test(
                "/app/secret",
                &secret_target.to_string_lossy(),
            )],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let writer = FsFileWriter;

        let mut secrets1 = BTreeMap::new();
        secrets1.insert("/app/secret".into(), "value-1".into());
        let g1 = create_with_writer(&manifest, &secrets1, &[], true, &writer).unwrap();
        switch_with_writer(&manifest, &g1, &[], &writer).unwrap();
        assert_eq!(std::fs::read_to_string(&secret_target).unwrap(), "value-1");

        let mut secrets2 = BTreeMap::new();
        secrets2.insert("/app/secret".into(), "value-2".into());
        let g2 = create_with_writer(&manifest, &secrets2, &[], true, &writer).unwrap();
        switch_with_writer(&manifest, &g2, &[], &writer).unwrap();
        assert_eq!(std::fs::read_to_string(&secret_target).unwrap(), "value-2");

        let current = dir.join("current");
        assert_eq!(std::fs::read_link(&current).unwrap(), g2.path);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_generation_numbers_monotonic() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-monotonic");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest = Manifest {
            secrets: vec![],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 10,
        };

        let secrets = BTreeMap::new();
        let templates = vec![];

        let mut prev = 0u64;
        for _ in 0..5 {
            let generation = create(&manifest, &secrets, &templates, true).unwrap();
            assert!(generation.number > prev, "generation numbers must be monotonically increasing");
            prev = generation.number;
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_create_with_missing_secret_in_map() {
        use crate::manifest::SecretSpec;

        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-missing-secret");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest = Manifest {
            secrets: vec![SecretSpec::for_test(
                "/missing/secret",
                &dir.join("target").to_string_lossy(),
            )],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let secrets = BTreeMap::new();
        let generation = create(&manifest, &secrets, &[], true).unwrap();
        let gen_file = generation.path.join("missing-secret");
        assert!(
            !gen_file.exists(),
            "secret file should not be created when value is not in the map"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_rendered_template_dir_created() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-rendered-dir");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest = Manifest {
            secrets: vec![],
            templates: vec![],
            generations_dir: dir.join("generations").to_string_lossy().to_string(),
            symlink_path: dir.join("current").to_string_lossy().to_string(),
            keep_generations: 2,
        };

        let generation = create(&manifest, &BTreeMap::new(), &[], true).unwrap();
        assert!(generation.path.join("rendered").exists(), "rendered/ dir should always be created");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
