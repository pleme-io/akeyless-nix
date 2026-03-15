use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

use crate::traits::FileWriter;

/// Ownership specification for a secret file.
/// When uid/gid are set, they take precedence over owner/group names.
#[derive(Debug, Clone, Default)]
pub struct Ownership {
    /// File owner name (looked up via passwd)
    pub owner: String,
    /// File group name (looked up via passwd)
    pub group: String,
    /// File owner UID (alternative to owner name, takes precedence)
    pub uid: Option<u32>,
    /// File group GID (alternative to group name, takes precedence)
    pub gid: Option<u32>,
}

/// Write a secret value to a file with the specified permissions.
///
/// Convenience wrapper around [`write_secret_with_ownership`] with default
/// (empty) ownership. When `ignore_passwd` is true, all owner/group operations
/// (chown) are skipped. This is useful in CI, dry-run, or containerized
/// contexts where /etc/passwd may not have the target users/groups.
#[cfg(test)]
pub fn write_secret(path: &Path, value: &str, mode: &str, ignore_passwd: bool) -> Result<()> {
    write_secret_with_ownership(path, value, mode, ignore_passwd, &Ownership::default())
}

/// Write a secret value to a file with the specified permissions and ownership.
///
/// When `ignore_passwd` is true, all owner/group operations (chown) are skipped.
/// When uid/gid are set in ownership, they are used directly without passwd lookup.
/// When owner/group names are set (and ignore_passwd is false), they are resolved
/// via the system passwd/group databases.
pub fn write_secret_with_ownership(
    path: &Path,
    value: &str,
    mode: &str,
    ignore_passwd: bool,
    ownership: &Ownership,
) -> Result<()> {
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

    // Set ownership (chown) unless ignore_passwd is set
    if !ignore_passwd {
        set_ownership(path, ownership)?;
    }

    Ok(())
}

/// Set file ownership using uid/gid directly, or by resolving owner/group names.
///
/// - If uid is Some, use it directly; otherwise resolve from owner name
/// - If gid is Some, use it directly; otherwise resolve from group name
/// - Empty owner/group strings are treated as "no change" (-1 in chown)
fn set_ownership(path: &Path, ownership: &Ownership) -> Result<()> {
    use std::ffi::CString;

    // Determine the UID to set
    let uid: i32 = if let Some(u) = ownership.uid {
        i32::try_from(u).unwrap_or(-1)
    } else if ownership.owner.is_empty() {
        -1 // no change
    } else {
        resolve_uid(&ownership.owner)?
    };

    // Determine the GID to set
    let gid: i32 = if let Some(g) = ownership.gid {
        i32::try_from(g).unwrap_or(-1)
    } else if ownership.group.is_empty() {
        -1 // no change
    } else {
        resolve_gid(&ownership.group)?
    };

    // Skip chown if neither uid nor gid needs changing
    if uid == -1 && gid == -1 {
        return Ok(());
    }

    let c_path = CString::new(path.to_string_lossy().as_bytes())
        .with_context(|| format!("converting path {} to CString", path.display()))?;

    // SAFETY: c_path is a valid null-terminated string, uid/gid are valid values
    // (-1 means "don't change"). This is a standard POSIX chown call.
    let ret = unsafe { libc::chown(c_path.as_ptr(), uid as libc::uid_t, gid as libc::gid_t) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        anyhow::bail!(
            "chown({}, uid={uid}, gid={gid}) failed: {err}",
            path.display()
        );
    }

    Ok(())
}

/// Resolve a username to a UID via getpwnam.
fn resolve_uid(owner: &str) -> Result<i32> {
    use std::ffi::CString;

    let c_name = CString::new(owner)
        .with_context(|| format!("invalid owner name: {owner}"))?;

    // SAFETY: c_name is a valid null-terminated string. getpwnam returns a pointer
    // to a static passwd struct (or null if not found). We only read the pw_uid field.
    let pw = unsafe { libc::getpwnam(c_name.as_ptr()) };
    if pw.is_null() {
        anyhow::bail!("user '{owner}' not found in passwd database");
    }
    // SAFETY: pw is non-null, so dereferencing is valid.
    Ok(unsafe { (*pw).pw_uid } as i32)
}

/// Resolve a group name to a GID via getgrnam.
fn resolve_gid(group: &str) -> Result<i32> {
    use std::ffi::CString;

    let c_name = CString::new(group)
        .with_context(|| format!("invalid group name: {group}"))?;

    // SAFETY: c_name is a valid null-terminated string. getgrnam returns a pointer
    // to a static group struct (or null if not found). We only read the gr_gid field.
    let gr = unsafe { libc::getgrnam(c_name.as_ptr()) };
    if gr.is_null() {
        anyhow::bail!("group '{group}' not found in group database");
    }
    // SAFETY: gr is non-null, so dereferencing is valid.
    Ok(unsafe { (*gr).gr_gid } as i32)
}

/// Concrete file-system backed [`FileWriter`] implementation.
///
/// All operations are real filesystem calls (mkdir, unlink, symlink).
pub struct FsFileWriter;

impl FileWriter for FsFileWriter {
    fn create_dir_all(&self, path: &Path) -> Result<()> {
        std::fs::create_dir_all(path)
            .with_context(|| format!("creating directory {}", path.display()))
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        // Non-fatal: ignore if file doesn't exist
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    fn symlink(&self, src: &Path, dst: &Path) -> Result<()> {
        let _ = std::fs::remove_file(dst);
        std::os::unix::fs::symlink(src, dst)
            .with_context(|| format!("symlinking {} -> {}", dst.display(), src.display()))
    }
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

    #[test]
    fn test_write_secret_ignore_passwd_skips_chown() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-ignore");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("test-secret-ignore");
        let ownership = Ownership {
            owner: "nonexistent_user_12345".to_string(),
            group: "nonexistent_group_12345".to_string(),
            uid: None,
            gid: None,
        };
        // With ignore_passwd=true, this should succeed even with bogus owner/group
        write_secret_with_ownership(&path, "value", "0600", true, &ownership).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "value");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_with_uid_gid() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-uidgid");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("test-secret-uidgid");
        // Use current user's uid/gid so chown succeeds without root
        let current_uid = unsafe { libc::getuid() };
        let current_gid = unsafe { libc::getgid() };
        let ownership = Ownership {
            owner: String::new(),
            group: String::new(),
            uid: Some(current_uid),
            gid: Some(current_gid),
        };
        write_secret_with_ownership(&path, "value", "0400", false, &ownership).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "value");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_empty_ownership_no_chown() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-empty-own");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("test-secret-empty-own");
        let ownership = Ownership::default();
        // Empty owner/group with no uid/gid should skip chown entirely
        write_secret_with_ownership(&path, "value", "0600", false, &ownership).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "value");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fs_file_writer_create_dir_all() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-fswriter-dir");
        let _ = std::fs::remove_dir_all(&dir);

        let writer = FsFileWriter;
        let nested = dir.join("a").join("b").join("c");
        writer.create_dir_all(&nested).unwrap();
        assert!(nested.is_dir());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fs_file_writer_remove_file() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-fswriter-rm");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let writer = FsFileWriter;
        let path = dir.join("to-remove");
        std::fs::write(&path, "bye").unwrap();
        assert!(path.exists());

        writer.remove_file(&path).unwrap();
        assert!(!path.exists());

        // Removing a non-existent file should not error
        writer.remove_file(&path).unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fs_file_writer_symlink() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-fswriter-sym");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let writer = FsFileWriter;
        let src = dir.join("source");
        let dst = dir.join("link");

        std::fs::write(&src, "content").unwrap();
        writer.symlink(&src, &dst).unwrap();

        assert!(dst.is_symlink());
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "content");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
