use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

/// Write a secret value to a file with the specified permissions.
pub fn write_secret(path: &Path, value: &str, mode: &str, _ignore_passwd: bool) -> Result<()> {
    // Create parent directories
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dirs for {}", path.display()))?;
    }

    // Write content
    std::fs::write(path, value)
        .with_context(|| format!("writing {}", path.display()))?;

    // Set permissions
    let mode_int = u32::from_str_radix(mode, 8)
        .with_context(|| format!("parsing mode '{mode}' as octal"))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode_int))
        .with_context(|| format!("setting permissions on {}", path.display()))?;

    Ok(())
}
