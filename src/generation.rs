use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::manifest::Manifest;
use crate::template::RenderedTemplate;
use crate::write;

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
    let gd = Path::new(&manifest.generations_dir);
    std::fs::create_dir_all(gd)
        .with_context(|| format!("creating generations dir {}", gd.display()))?;

    // Find next generation number
    let number = next_generation(gd)?;
    let gp = gd.join(number.to_string());
    std::fs::create_dir_all(&gp)?;

    // Write secrets
    for spec in &manifest.secrets {
        if let Some(value) = secrets.get(&spec.akeyless_path) {
            let target = gp.join(sanitize_name(&spec.akeyless_path));
            write::write_secret(&target, value, &spec.mode, ignore_passwd)?;
        }
    }

    // Write rendered templates
    let tmpl_dir = gp.join("rendered");
    std::fs::create_dir_all(&tmpl_dir)?;
    for tmpl in templates {
        let target = tmpl_dir.join(&tmpl.name);
        write::write_secret(&target, &tmpl.content, &tmpl.mode, ignore_passwd)?;
    }

    Ok(Generation {
        number,
        path: gp,
    })
}

/// Atomically switch the symlink to point to the new generation.
pub fn switch(manifest: &Manifest, genr: &Generation) -> Result<()> {
    let symlink = Path::new(&manifest.symlink_path);

    // Create symlinks from declared file_path → generation file
    for spec in &manifest.secrets {
        let gf = genr.path.join(sanitize_name(&spec.akeyless_path));
        let target = Path::new(&spec.file_path);

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Remove old symlink/file if it exists
        let _ = std::fs::remove_file(target);
        std::os::unix::fs::symlink(&gf, target)
            .with_context(|| format!("symlinking {} → {}", target.display(), gf.display()))?;
    }

    // Same for templates
    for tmpl in &manifest.templates {
        let gf = genr.path.join("rendered").join(&tmpl.name);
        let target = Path::new(&tmpl.file_path);

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let _ = std::fs::remove_file(target);
        std::os::unix::fs::symlink(&gf, target)
            .with_context(|| format!("symlinking {} → {}", target.display(), gf.display()))?;
    }

    // Update the main symlink
    if let Some(parent) = symlink.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(symlink);
    std::os::unix::fs::symlink(&genr.path, symlink)
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
/// "/pleme/prod/db-password" → "pleme-prod-db-password"
fn sanitize_name(path: &str) -> String {
    path.trim_start_matches('/')
        .replace('/', "-")
}
