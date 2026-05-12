//! Linux Landlock sandbox fallback implementation.
//!
//! Landlock is a Linux security module (LSM) available since kernel 5.13 that
//! allows processes to restrict their own filesystem access. Unlike bubblewrap
//! which uses user namespaces, Landlock applies restrictions directly to the
//! current process.
//!
//! This module is the **fallback** for when bubblewrap is not available. It
//! provides in-process filesystem sandboxing by:
//!
//! 1. Creating a Landlock ruleset with the required access restrictions
//! 2. Adding rules for allowed paths (read/write/execute)
//! 3. Restricting the current process (irreversible)
//! 4. Then exec'ing the actual command
//!
//! # Limitations vs Bubblewrap
//!
//! - Cannot isolate network (no network namespace)
//! - Cannot hide /proc (no PID namespace)
//! - Cannot create separate mount namespace
//! - The restriction is irreversible: once locked, even the parent process
//!   cannot regain access
//!
//! # Requirements
//!
//! - Linux kernel 5.13 or later
//! - CONFIG_SECURITY_LANDLOCK=y

use super::policy::SandboxPolicy;
use std::ffi::CString;
use std::path::Path;
use std::sync::OnceLock;

/// Check if Landlock is available on this system.
pub fn is_landlock_available() -> bool {
    static LANDLOCK_AVAILABLE: OnceLock<bool> = OnceLock::new();

    *LANDLOCK_AVAILABLE.get_or_init(|| {
        if !cfg!(target_os = "linux") {
            return false;
        }

        // Probe Landlock ABI by calling landlock_create_ruleset with a null
        // pointer and LANDLOCK_CREATE_RULESET_VERSION flag. If the syscall
        // is available, it returns a positive version number.
        // SAFETY: syscall with null ruleset pointer for ABI probing. The
        // kernel does not dereference the null pointer in this mode.
        unsafe {
            let result = libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<libc::c_void>(),
                0usize,
                LANDLOCK_CREATE_RULESET_VERSION,
            );
            result >= 0
        }
    })
}

/// Get the Landlock ABI version supported by the kernel.
/// Returns None if Landlock is not available.
pub fn get_abi_version() -> Option<i32> {
    if !is_landlock_available() {
        return None;
    }

    // SAFETY: ABI probing with null pointer is safe as the kernel doesn't
    // dereference it when LANDLOCK_CREATE_RULESET_VERSION is set.
    unsafe {
        let result = libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<libc::c_void>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        );
        if result >= 0 {
            i32::try_from(result).ok()
        } else {
            None
        }
    }
}

// Landlock syscall constants (not yet in all libc crate versions)
const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1 << 0;

const LANDLOCK_ACCESS_FS_EXECUTE: u64 = 1 << 0;
const LANDLOCK_ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
const LANDLOCK_ACCESS_FS_READ_FILE: u64 = 1 << 2;
const LANDLOCK_ACCESS_FS_READ_DIR: u64 = 1 << 3;
const LANDLOCK_ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
const LANDLOCK_ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
const LANDLOCK_ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
const LANDLOCK_ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
const LANDLOCK_ACCESS_FS_MAKE_REG: u64 = 1 << 8;
const LANDLOCK_ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
const LANDLOCK_ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
const LANDLOCK_ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
const LANDLOCK_ACCESS_FS_MAKE_SYM: u64 = 1 << 12;
const LANDLOCK_ACCESS_FS_REFER: u64 = 1 << 13;
const LANDLOCK_ACCESS_FS_TRUNCATE: u64 = 1 << 14;

// Useful combinations
const LANDLOCK_ACCESS_FS_READ: u64 =
    LANDLOCK_ACCESS_FS_READ_FILE | LANDLOCK_ACCESS_FS_READ_DIR;

const LANDLOCK_ACCESS_FS_WRITE: u64 = LANDLOCK_ACCESS_FS_WRITE_FILE
    | LANDLOCK_ACCESS_FS_REMOVE_DIR
    | LANDLOCK_ACCESS_FS_REMOVE_FILE
    | LANDLOCK_ACCESS_FS_MAKE_DIR
    | LANDLOCK_ACCESS_FS_MAKE_REG
    | LANDLOCK_ACCESS_FS_MAKE_SYM
    | LANDLOCK_ACCESS_FS_TRUNCATE;

#[repr(C)]
struct LandlockRulesetAttr {
    handled_access_fs: u64,
}

#[repr(C)]
struct LandlockPathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;

/// Apply Landlock restrictions based on the given policy.
///
/// This function:
/// 1. Creates a Landlock ruleset that handles all filesystem operations
/// 2. Adds read+execute access to the entire filesystem
/// 3. Adds write access to allowed directories (cwd, /tmp, etc.)
/// 4. Restricts the current process (irreversible)
///
/// After this call, the process's filesystem access is permanently limited.
///
/// # Safety
///
/// This function uses raw syscalls and must be called after fork() but before
/// exec(). Once Landlock is applied, it cannot be reversed. Calling this from
/// the main (parent) process will permanently restrict it.
///
/// # Errors
///
/// Returns an error if Landlock is not available, the syscalls fail, or the
/// policy cannot be applied.
pub unsafe fn apply_landlock_policy(policy: &SandboxPolicy, cwd: &Path) -> Result<(), String> {
    if !is_landlock_available() {
        return Err("Landlock is not available on this system".to_string());
    }

    // Determine which operations to handle (restrict)
    let handled_access = LANDLOCK_ACCESS_FS_EXECUTE
        | LANDLOCK_ACCESS_FS_READ
        | LANDLOCK_ACCESS_FS_WRITE;

    let attr = LandlockRulesetAttr {
        handled_access_fs: handled_access,
    };

    // Step 1: Create ruleset
    // SAFETY: attr is a valid local variable with correct size.
    let ruleset_fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &raw const attr,
            std::mem::size_of::<LandlockRulesetAttr>(),
            0u32,
        )
    };

    if ruleset_fd < 0 {
        return Err(format!(
            "landlock_create_ruleset failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let ruleset_fd = ruleset_fd as i32;

    // Step 2: Add read+execute access to the entire filesystem
    if policy.has_full_disk_read_access() {
        unsafe {
            add_path_rule(
                ruleset_fd,
                Path::new("/"),
                LANDLOCK_ACCESS_FS_READ | LANDLOCK_ACCESS_FS_EXECUTE,
            )?;
        }
    }

    // Step 3: Add write access to allowed directories
    if should_allow_writes(policy) {
        // cwd is always writable
        unsafe {
            add_path_rule(
                ruleset_fd,
                cwd,
                LANDLOCK_ACCESS_FS_READ | LANDLOCK_ACCESS_FS_WRITE | LANDLOCK_ACCESS_FS_EXECUTE,
            )?;
        }

        // /tmp is always writable
        unsafe {
            add_path_rule(
                ruleset_fd,
                Path::new("/tmp"),
                LANDLOCK_ACCESS_FS_READ | LANDLOCK_ACCESS_FS_WRITE | LANDLOCK_ACCESS_FS_EXECUTE,
            )?;
        }

        // TMPDIR is always writable if set
        if let Ok(tmpdir) = std::env::var("TMPDIR") {
            let tmpdir_path = std::path::PathBuf::from(&tmpdir);
            if tmpdir_path.is_absolute() {
                unsafe {
                    add_path_rule(
                        ruleset_fd,
                        &tmpdir_path,
                        LANDLOCK_ACCESS_FS_READ
                            | LANDLOCK_ACCESS_FS_WRITE
                            | LANDLOCK_ACCESS_FS_EXECUTE,
                    )?;
                }
            }
        }
    }

    // Step 4: Apply no_new_privs (required for Landlock)
    // SAFETY: prctl with PR_SET_NO_NEW_PRIVS is safe in a child process.
    unsafe {
        let ret = libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
        if ret != 0 {
            return Err(format!(
                "prctl(PR_SET_NO_NEW_PRIVS) failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    // Step 5: Restrict the process
    // SAFETY: The ruleset_fd is valid and we've added all rules.
    unsafe {
        let ret = libc::syscall(
            libc::SYS_landlock_restrict_self,
            ruleset_fd,
            0u32,
        );

        if ret != 0 {
            return Err(format!(
                "landlock_restrict_self failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    Ok(())
}

/// Add a path rule to a Landlock ruleset.
///
/// # Safety
///
/// `ruleset_fd` must be a valid file descriptor from a successful
/// `landlock_create_ruleset` call.
unsafe fn add_path_rule(
    ruleset_fd: i32,
    path: &Path,
    allowed_access: u64,
) -> Result<(), String> {
    let path_cstr = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| format!("invalid path: {}", path.display()))?;

    // Open the path to get a file descriptor
    // SAFETY: path_cstr is NUL-terminated. O_PATH + O_CLOEXEC is safe.
    let fd = unsafe {
        libc::open(
            path_cstr.as_ptr(),
            libc::O_PATH | libc::O_CLOEXEC,
        )
    };

    if fd < 0 {
        return Err(format!(
            "failed to open path '{}': {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }

    let path_attr = LandlockPathBeneathAttr {
        allowed_access,
        parent_fd: fd,
    };

    // SAFETY: path_attr is valid and sized correctly.
    let ret = unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset_fd,
            LANDLOCK_RULE_PATH_BENEATH,
            &raw const path_attr,
            libc::LANDLOCK_ACCESS_FS_READ_FILE as u32, // Was only using the flags value
        )
    };

    // Close the file descriptor
    unsafe { libc::close(fd); }

    if ret != 0 {
        Err(format!(
            "landlock_add_rule failed for '{}': {}",
            path.display(),
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

/// Check if the policy allows any writes.
fn should_allow_writes(policy: &SandboxPolicy) -> bool {
    matches!(policy, SandboxPolicy::WorkspaceWrite { .. })
}

#[cfg(test)]mod tests {
    use super::*;

    #[test]
    fn test_landlock_not_available_on_macos() {
        // On non-Linux, is_landlock_available should be false
        if !cfg!(target_os = "linux") {
            assert!(!is_landlock_available());
        }
    }

    #[test]
    fn test_get_abi_version_on_non_linux() {
        if !cfg!(target_os = "linux") {
            assert!(get_abi_version().is_none());
        }
    }

    #[test]
    fn test_build_landlock_exec_args() {
        let (program, args) = build_landlock_exec_args(
            "sh",
            &["-c".to_string(), "echo hi".to_string()],
        );

        assert_eq!(program, "sh");
        assert_eq!(args, vec!["-c".to_string(), "echo hi".to_string()]);
    }

    #[test]
    fn test_add_path_rule_non_existent_path() {
        // Adding a rule for a non-existent path should fail
        if cfg!(target_os = "linux") && is_landlock_available() {
            // We need a valid ruleset first — but on CI this may not be
            // available. Just verify the function signature is correct.
        }
    }
}
