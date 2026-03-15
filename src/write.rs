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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn test_write_secret_creates_file() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("test-secret");
        write_secret(&path, "secret-value", "0600", false).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "secret-value");

        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
