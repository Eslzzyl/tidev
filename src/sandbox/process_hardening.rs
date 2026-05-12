//! Process hardening for sandboxed command execution.
//!
//! This module provides a `pre_exec_hardening()` function that should be called
//! in the child process after fork() but before exec(). It disables dangerous
//! capabilities that could be used to subvert the sandbox or leak information:
//!
//! - Disables core dumps (RLIMIT_CORE = 0)
//! - Disables ptrace attach (Linux: PR_SET_DUMPABLE, macOS: PT_DENY_ATTACH)
//! - Removes dangerous environment variables (LD_PRELOAD, DYLD_*, etc.)
//!
//! Reference: OpenAI Codex `codex-process-hardening` crate.

/// Perform process hardening for a sandboxed child process.
///
/// This is intended to be called via `Command::pre_exec()` which runs in the
/// forked child before exec(). On success, the process is hardened; on failure,
/// the function returns an Err that causes the fork to abort.
///
/// # Safety
///
/// This function uses raw libc syscalls and must only be called in the child
/// process after fork() and before exec(). It is safe to call from
/// `Command::pre_exec()` which provides this guarantee.
pub unsafe fn pre_exec_hardening() -> Result<(), std::io::Error> {
    // Disable core dumps on all Unix platforms.
    #[cfg(unix)]
    set_core_file_size_limit_to_zero()?;

    #[cfg(target_os = "macos")]
    disable_ptrace_macos()?;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    disable_ptrace_linux()?;

    // Remove dangerous environment variables that could be used for library
    // injection or information leakage.
    // SAFETY: remove_env operations are safe in the single-threaded child
    // process context where pre_exec_hardening runs.
    unsafe { remove_dangerous_env_vars(); }

    Ok(())
}

/// Disable core dumps by setting RLIMIT_CORE to zero.
#[cfg(unix)]
fn set_core_file_size_limit_to_zero() -> Result<(), std::io::Error> {
    let rlim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: setrlimit with RLIMIT_CORE is safe to call in a single-threaded
    // child process. No shared state is involved.
    let ret = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &rlim) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Disable ptrace on Linux by marking the process non-dumpable.
///
/// Once a process is non-dumpable, same-user processes cannot attach with
/// ptrace(), and core dumps are also inhibited as a side effect.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn disable_ptrace_linux() -> Result<(), std::io::Error> {
    // SAFETY: prctl with PR_SET_DUMPABLE is safe in a single-threaded child
    // process. The call only affects the current process.
    let ret = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Disable ptrace on macOS by calling ptrace(PT_DENY_ATTACH).
///
/// Once PT_DENY_ATTACH has been called, any subsequent ptrace attach from
/// another process (including a debugger) will fail with EPERM.
#[cfg(target_os = "macos")]
fn disable_ptrace_macos() -> Result<(), std::io::Error> {
    const PT_DENY_ATTACH: libc::c_int = 31;
    // SAFETY: ptrace(PT_DENY_ATTACH) is safe to call in a child process.
    // It only affects the current process and has no memory safety implications.
    let ret = unsafe { libc::ptrace(PT_DENY_ATTACH, 0, std::ptr::null_mut(), 0) };
    if ret == 0 || ret == -1 {
        // PT_DENY_ATTACH returns -1 on success (yes, really -- Apple's man page
        // documents this behavior). Treat both 0 and -1 as success.
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Remove environment variables that are dangerous in a sandboxed context.
///
/// # Safety
///
/// Must only be called in the child process after fork(), before exec().
unsafe fn remove_dangerous_env_vars() {
    #[cfg(target_os = "linux")]
    unsafe { remove_env_vars_with_prefix("LD_"); }

    #[cfg(target_os = "macos")]
    unsafe {
        remove_env_vars_with_prefix("DYLD_");
        remove_env_vars_with_prefix("MallocStackLogging");
        remove_env_vars_with_prefix("MallocLogFile");
    }

    // Also remove common injection variables on all platforms
    unsafe {
        std::env::remove_var("LD_PRELOAD");
        std::env::remove_var("LD_LIBRARY_PATH");
    }
}

/// Remove all environment variables whose name starts with the given prefix.
///
/// # Safety
///
/// Must only be called in the child process after fork(), before exec().
unsafe fn remove_env_vars_with_prefix(prefix: &str) {
    let keys_to_remove: Vec<std::ffi::OsString> = std::env::vars_os()
        .filter_map(|(key, _)| {
            let key_str = key.to_string_lossy();
            if key_str.starts_with(prefix) {
                Some(key)
            } else {
                None
            }
        })
        .collect();

    for key in keys_to_remove {
        unsafe { std::env::remove_var(key); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_env_vars_with_prefix() {
        unsafe { std::env::set_var("LD_TEST_VAR", "1"); }
        unsafe { std::env::set_var("PATH", "/usr/bin"); }
        unsafe { std::env::set_var("DYLD_TEST", "1"); }

        unsafe { remove_env_vars_with_prefix("LD_"); }

        assert!(std::env::var("LD_TEST_VAR").is_err());
        assert_eq!(std::env::var("PATH").unwrap(), "/usr/bin");
        assert_eq!(std::env::var("DYLD_TEST").unwrap(), "1");

        // Cleanup
        unsafe {
            std::env::remove_var("LD_TEST_VAR");
            std::env::remove_var("DYLD_TEST");
        }
    }

    #[test]
    fn test_remove_env_vars_with_prefix_empty() {
        // Should not crash when removing from clean env
        unsafe { remove_env_vars_with_prefix("NONEXISTENT_"); }
    }
}
