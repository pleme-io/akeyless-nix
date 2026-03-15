// Platform-specific secret filesystem handling.
// Darwin: user-space directories (no RAM disk needed for HM)
// Linux: ramfs mount for NixOS system secrets

#[cfg(target_os = "macos")]
pub mod darwin;

#[cfg(target_os = "linux")]
pub mod linux;
