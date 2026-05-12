//! Bubblewrap path detection and availability checking for Linux.
//!
//! This module detects whether bubblewrap (`bwrap`) is available on the
//! system and selects between system and fallback strategies.
//!
//! # Strategy
//!
//! 1. Look for `bwrap` on PATH, excluding the current working directory
//!    to prevent malicious binary injection.
//! 2. Check that the found bwrap supports `--argv0` (needed for clean
//!    re-exec inside the sandbox).
//! 3. Check that user namespaces are available (WSL1 does not support them).
//! 4. If bwrap is unavailable, the system falls back to Landlock.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Name of the bubblewrap binary.
const BWRAP_BINARY: &str = "bwrap";

/// Minimum bwrap version that supports `--argv0`.
/// We check by trying `bwrap --argv0` rather than parsing version strings.
const BWRAP_MIN_VERSION: &str = "0.6.0";

/// Check if bubblewrap is available and functional on this system.
pub fn is_bwrap_available() -> bool {
    static BWRAP_AVAILABLE: OnceLock<bool> = OnceLock::new();

    *BWRAP_AVAILABLE.get_or_init(|| {
        // bwrap is not available on macOS or Windows
        if !cfg!(target_os = "linux") {
            return false;
        }

        let bwrap_path = match find_system_bwrap() {
            Some(path) => path,
            None => return false,
        };

        // Check that bwrap can actually run a trivial command
        let result = Command::new(&bwrap_path)
            .args([
                "--ro-bind",
                "/",
                "/",
                "--dev",
                "/dev",
                "--",
                "/usr/bin/true",
            ])
            .output();

        match result {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    })
}

/// Check if we are running under WSL1 (does not support user namespaces).
pub fn is_wsl1() -> bool {
    // WSL1 does not have a /proc/sys/kernel/osrelease that contains "Microsoft"
    // in the same way. The most reliable check is to try creating a user namespace.
    // However, a simpler heuristic is checking /proc/version for "Microsoft"
    // and /proc/sys/kernel/osrelease for a version < 4.19 (WSL1 kernel version).
    let version = match std::fs::read_to_string("/proc/version") {
        Ok(v) => v.to_lowercase(),
        Err(_) => return false,
    };

    version.contains("microsoft") && !version.contains("wsl2")
}

/// Find the system `bwrap` binary on PATH.
///
/// This excludes the current working directory from the search to prevent
/// a local attacker (or AI-generated files) from placing a malicious `bwrap`
/// in the project directory.
pub fn find_system_bwrap() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let cwd_canonical = cwd.canonicalize().ok().unwrap_or(cwd);

    // Search PATH entries manually, excluding cwd
    let path_var = std::env::var_os("PATH")?;
    for path_entry in std::env::split_paths(&path_var) {
        // Skip if this PATH entry is inside the current working directory
        let canonical_entry = path_entry.canonicalize().ok().unwrap_or(path_entry.clone());
        if canonical_entry.starts_with(&cwd_canonical) {
            continue;
        }

        let candidate = canonical_entry.join(BWRAP_BINARY);
        if candidate.is_file() {
            // Check if this bwrap supports --argv0
            if supports_argv0(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

/// Check if a bwrap binary supports the `--argv0` flag.
///
/// `--argv0` is needed so the inner process sees its original argv[0] rather
/// than the synthetic one from bubblewrap, preserving the ability to detect
/// its own binary path via `/proc/self/exe` or `argv[0]`.
fn supports_argv0(bwrap_path: &Path) -> bool {
    // Run `bwrap --argv0 test -- /usr/bin/true` with a trivial bind mount.
    // If --argv0 is supported, this will succeed.
    let result = Command::new(bwrap_path)
        .args([
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--argv0",
            "test",
            "--",
            "/usr/bin/true",
        ])
        .output();

    match result {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Return a startup warning message if bubblewrap is missing.
pub fn missing_bwrap_warning() -> Option<String> {
    if cfg!(not(target_os = "linux")) {
        return None;
    }

    if find_system_bwrap().is_some() {
        return None;
    }

    Some(
        "bubblewrap (bwrap) is not available on this system. \
         Falling back to Landlock sandbox which has limited functionality. \
         Install bubblewrap for full sandbox support: \
         https://github.com/containers/bubblewrap"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bwrap_not_found_on_macos() {
        // On non-Linux, is_bwrap_available should be false
        if !cfg!(target_os = "linux") {
            assert!(!is_bwrap_available());
        }
    }

    #[test]
    fn test_missing_bwrap_warning_on_macos() {
        if !cfg!(target_os = "linux") {
            assert!(missing_bwrap_warning().is_none());
        }
    }
}
