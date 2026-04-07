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
/// When owner/group names are set (and `ignore_passwd` is false), they are resolved
/// via the system passwd/group databases.
pub(crate) fn write_secret_with_ownership(
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

/// POSIX "no change" sentinel — `(uid_t)-1` / `(gid_t)-1`.
const NO_CHANGE: libc::uid_t = libc::uid_t::MAX;

/// Set file ownership using uid/gid directly, or by resolving owner/group names.
///
/// - If uid is `Some`, use it directly; otherwise resolve from owner name
/// - If gid is `Some`, use it directly; otherwise resolve from group name
/// - Empty owner/group strings are treated as "no change" (`NO_CHANGE` in chown)
fn set_ownership(path: &Path, ownership: &Ownership) -> Result<()> {
    use std::ffi::CString;

    let uid: libc::uid_t = if let Some(u) = ownership.uid {
        u
    } else if ownership.owner.is_empty() {
        NO_CHANGE
    } else {
        resolve_uid(&ownership.owner)?
    };

    let gid: libc::gid_t = if let Some(g) = ownership.gid {
        g
    } else if ownership.group.is_empty() {
        NO_CHANGE
    } else {
        resolve_gid(&ownership.group)?
    };

    if uid == NO_CHANGE && gid == NO_CHANGE {
        return Ok(());
    }

    let c_path = CString::new(path.to_string_lossy().as_bytes())
        .with_context(|| format!("converting path {} to CString", path.display()))?;

    // SAFETY: c_path is a valid null-terminated string, uid/gid are valid values
    // (NO_CHANGE means "don't change"). This is a standard POSIX chown call.
    let ret = unsafe { libc::chown(c_path.as_ptr(), uid, gid) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        anyhow::bail!(
            "chown({}, uid={uid}, gid={gid}) failed: {err}",
            path.display()
        );
    }

    Ok(())
}

/// Resolve a username to a UID via `getpwnam`.
fn resolve_uid(owner: &str) -> Result<libc::uid_t> {
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
    Ok(unsafe { (*pw).pw_uid })
}

/// Resolve a group name to a GID via `getgrnam`.
fn resolve_gid(group: &str) -> Result<libc::gid_t> {
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
    Ok(unsafe { (*gr).gr_gid })
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
        write_secret_with_ownership(&path, "value", "0600", true, &ownership).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "value");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_with_uid_gid() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-uidgid");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("test-secret-uidgid");
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

    #[test]
    fn test_write_secret_invalid_mode_octal() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-badmode");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("test-secret-badmode");
        let result = write_secret(&path, "value", "9999", true);
        assert!(result.is_err(), "non-octal mode should fail");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("parsing mode"),
            "error should mention parsing: {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_empty_mode_string() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-emptymode");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("test-secret-emptymode");
        let result = write_secret(&path, "value", "", true);
        assert!(result.is_err(), "empty mode string should fail");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_creates_nested_parent_dirs() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-nested");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("a").join("b").join("c").join("secret");
        write_secret(&path, "deep-value", "0644", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "deep-value");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_overwrite_existing() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-overwrite");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("overwrite-secret");
        write_secret(&path, "first", "0600", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");

        write_secret(&path, "second", "0600", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_preserves_newlines_and_special_chars() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-special");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("special-secret");
        let value = "line1\nline2\n\ttab\0null";
        write_secret(&path, value, "0600", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), value);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_empty_value() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-empty-val");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("empty-secret");
        write_secret(&path, "", "0600", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_mode_0400_readonly() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-readonly");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("readonly-secret");
        write_secret(&path, "readonly", "0400", true).unwrap();

        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o400);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_mode_0644() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-0644");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("readable-secret");
        write_secret(&path, "readable", "0644", true).unwrap();

        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o644);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_various_modes() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-modes");
        let _ = std::fs::remove_dir_all(&dir);

        let test_modes = [("0400", 0o400), ("0644", 0o644), ("0755", 0o755), ("0600", 0o600)];
        for (mode_str, expected) in &test_modes {
            let path = dir.join(format!("mode-{mode_str}"));
            write_secret(&path, "val", mode_str, true).unwrap();
            let perms = std::fs::metadata(&path).unwrap().permissions();
            assert_eq!(
                perms.mode() & 0o777,
                *expected,
                "mode {mode_str} should result in {expected:#o}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_uid_nonexistent_user() {
        let result = resolve_uid("nonexistent_user_xyz_12345");
        assert!(
            result.is_err(),
            "nonexistent user should fail"
        );
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "error should say not found: {err}");
    }

    #[test]
    fn test_resolve_gid_nonexistent_group() {
        let result = resolve_gid("nonexistent_group_xyz_12345");
        assert!(
            result.is_err(),
            "nonexistent group should fail"
        );
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "error should say not found: {err}");
    }

    #[test]
    fn test_resolve_uid_root() {
        let result = resolve_uid("root");
        assert!(result.is_ok(), "root user should exist");
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_resolve_gid_root() {
        let result = resolve_gid("root");
        assert!(result.is_ok(), "root group should exist");
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_set_ownership_with_uid_only() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-uid-only");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("uid-only-secret");
        std::fs::write(&path, "test").unwrap();

        let current_uid = unsafe { libc::getuid() };
        let ownership = Ownership {
            owner: String::new(),
            group: String::new(),
            uid: Some(current_uid),
            gid: None,
        };
        let result = set_ownership(&path, &ownership);
        assert!(result.is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_set_ownership_with_gid_only() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-gid-only");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("gid-only-secret");
        std::fs::write(&path, "test").unwrap();

        let current_gid = unsafe { libc::getgid() };
        let ownership = Ownership {
            owner: String::new(),
            group: String::new(),
            uid: None,
            gid: Some(current_gid),
        };
        let result = set_ownership(&path, &ownership);
        assert!(result.is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_with_owner_name_ignore_passwd_false() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-ownername");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("named-owner-secret");
        let ownership = Ownership {
            owner: "root".to_string(),
            group: "root".to_string(),
            uid: None,
            gid: None,
        };
        let result = write_secret_with_ownership(&path, "val", "0600", false, &ownership);
        let current_uid = unsafe { libc::getuid() };
        if current_uid == 0 {
            assert!(result.is_ok());
        } else {
            assert!(result.is_err());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ownership_default() {
        let ownership = Ownership::default();
        assert!(ownership.owner.is_empty());
        assert!(ownership.group.is_empty());
        assert!(ownership.uid.is_none());
        assert!(ownership.gid.is_none());
    }

    #[test]
    fn test_fs_file_writer_symlink_replaces_existing() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-fswriter-sym-replace");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let writer = FsFileWriter;
        let src1 = dir.join("source1");
        let src2 = dir.join("source2");
        let dst = dir.join("link");

        std::fs::write(&src1, "first").unwrap();
        std::fs::write(&src2, "second").unwrap();

        writer.symlink(&src1, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "first");

        writer.symlink(&src2, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "second");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fs_file_writer_create_dir_all_idempotent() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-fswriter-idempotent");
        let _ = std::fs::remove_dir_all(&dir);

        let writer = FsFileWriter;
        writer.create_dir_all(&dir).unwrap();
        writer.create_dir_all(&dir).unwrap();
        assert!(dir.is_dir());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_with_ownership_uid_gid_direct() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-own-uid");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("owned-secret");
        let current_uid = unsafe { libc::getuid() };
        let current_gid = unsafe { libc::getgid() };
        let ownership = Ownership {
            owner: "should-be-ignored".to_string(),
            group: "should-be-ignored".to_string(),
            uid: Some(current_uid),
            gid: Some(current_gid),
        };
        write_secret_with_ownership(&path, "val", "0600", false, &ownership).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "val");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_with_nonexistent_owner_fails() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-bad-owner");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("bad-owner-secret");
        let ownership = Ownership {
            owner: "user_that_does_not_exist_xyz_12345".to_string(),
            group: String::new(),
            uid: None,
            gid: None,
        };
        let result = write_secret_with_ownership(&path, "val", "0600", false, &ownership);
        assert!(result.is_err(), "nonexistent owner should fail");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "error: {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_secret_with_nonexistent_group_fails() {
        let dir = std::env::temp_dir().join("akeyless-nix-test-write-bad-group");
        let _ = std::fs::remove_dir_all(&dir);

        let path = dir.join("bad-group-secret");
        let ownership = Ownership {
            owner: String::new(),
            group: "group_that_does_not_exist_xyz_12345".to_string(),
            uid: None,
            gid: None,
        };
        let result = write_secret_with_ownership(&path, "val", "0600", false, &ownership);
        assert!(result.is_err(), "nonexistent group should fail");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "error: {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
