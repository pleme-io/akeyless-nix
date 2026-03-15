use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::manifest::Manifest;
use crate::template::RenderedTemplate;
use crate::traits::FileWriter;
use crate::write::{self, Ownership};

pub struct Generation {
    pub number: u64,
    pub path: PathBuf,
}

/// Create a new generation directory and write all secrets + templates.
pub fn create(
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
pub fn create_with_writer(
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
            let ownership = Ownership {
                owner: spec.owner.clone(),
                group: spec.group.clone(),
                uid: spec.uid,
                gid: spec.gid,
            };
            write::write_secret_with_ownership(
                &target,
                value,
                &spec.mode,
                ignore_passwd,
                &ownership,
            )?;
        }
    }

    // Write rendered templates
    let tmpl_dir = gp.join("rendered");
    writer.create_dir_all(&tmpl_dir)?;
    for tmpl in templates {
        let target = tmpl_dir.join(&tmpl.name);
        let ownership = Ownership {
            owner: tmpl.owner.clone(),
            group: tmpl.group.clone(),
            uid: tmpl.uid,
            gid: tmpl.gid,
        };
        write::write_secret_with_ownership(
            &target,
            &tmpl.content,
            &tmpl.mode,
            ignore_passwd,
            &ownership,
        )?;
    }

    Ok(Generation {
        number,
        path: gp,
    })
}

/// Atomically switch the symlink to point to the new generation.
pub fn switch(manifest: &Manifest, genr: &Generation) -> Result<()> {
    switch_with_writer(manifest, genr, &crate::write::FsFileWriter)
}

/// Switch using the provided `FileWriter`.
pub fn switch_with_writer(
    manifest: &Manifest,
    genr: &Generation,
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

    // Same for templates
    for tmpl in &manifest.templates {
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
pub fn prune(manifest: &Manifest) -> Result<()> {
    let gd = Path::new(&manifest.generations_dir);
    if !gd.exists() {
        return Ok(());
    }

    let mut gens: Vec<u64> = std::fs::read_dir(gd)?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str()?.parse::<u64>().ok())
        .collect();
    gens.sort();

    let keep = manifest.keep_generations as usize;
    if gens.len() <= keep {
        return Ok(());
    }

    let to_remove = &gens[..gens.len() - keep];
    for gn in to_remove {
        let path = gd.join(gn.to_string());
        let _ = std::fs::remove_dir_all(&path);
    }

    Ok(())
}

fn next_generation(gd: &Path) -> Result<u64> {
    let max = std::fs::read_dir(gd)?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str()?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    Ok(max + 1)
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

        // Create 3 generations
        let secrets = BTreeMap::new();
        let templates = vec![];

        let g1 = create(&manifest, &secrets, &templates, true).unwrap();
        assert_eq!(g1.number, 1);

        let g2 = create(&manifest, &secrets, &templates, true).unwrap();
        assert_eq!(g2.number, 2);

        let g3 = create(&manifest, &secrets, &templates, true).unwrap();
        assert_eq!(g3.number, 3);

        // Prune should keep only 2
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
            secrets: vec![SecretSpec {
                akeyless_path: "/pleme/prod/db-password".into(),
                file_path: secret_target.to_string_lossy().to_string(),
                mode: "0600".into(),
                owner: String::new(),
                group: String::new(),
                uid: None,
                gid: None,
                restart_units: vec![],
                reload_units: vec![],
            }],
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

        // Verify file was written in generation dir
        let gen_file = generation.path.join("pleme-prod-db-password");
        assert_eq!(std::fs::read_to_string(&gen_file).unwrap(), "s3cret");

        // Switch symlinks
        switch_with_writer(&manifest, &generation, &writer).unwrap();

        // Verify symlink was created to target
        assert!(secret_target.is_symlink());
        assert_eq!(std::fs::read_to_string(&secret_target).unwrap(), "s3cret");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_full_flow_with_templates() {
        use crate::manifest::SecretSpec;
        use crate::template::RenderedTemplate;
        use crate::write::FsFileWriter;

        let dir = std::env::temp_dir().join("akeyless-nix-test-gen-full");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let secret_target = dir.join("targets").join("token");
        let tmpl_target = dir.join("targets").join("config.yaml");

        let manifest = Manifest {
            secrets: vec![SecretSpec {
                akeyless_path: "/app/token".into(),
                file_path: secret_target.to_string_lossy().to_string(),
                mode: "0400".into(),
                owner: String::new(),
                group: String::new(),
                uid: None,
                gid: None,
                restart_units: vec![],
                reload_units: vec![],
            }],
            templates: vec![crate::manifest::TemplateSpec {
                name: "config.yaml".into(),
                content: "token: PLACEHOLDER".into(),
                file_path: tmpl_target.to_string_lossy().to_string(),
                mode: "0600".into(),
                owner: String::new(),
                group: String::new(),
                uid: None,
                gid: None,
            }],
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
        switch_with_writer(&manifest, &generation, &writer).unwrap();

        // Verify secret symlink
        assert!(secret_target.is_symlink());
        assert_eq!(std::fs::read_to_string(&secret_target).unwrap(), "my-token");

        // Verify template symlink
        assert!(tmpl_target.is_symlink());
        assert_eq!(std::fs::read_to_string(&tmpl_target).unwrap(), "token: my-token");

        // Verify generation symlink
        let current = dir.join("current");
        assert!(current.is_symlink());
        assert_eq!(std::fs::read_link(&current).unwrap(), generation.path);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
