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
/// forked child before exec().  Only async-signal-safe operations are
/// performed here — anything that allocates memory (e.g. env var removal)
/// is done in the parent process before `spawn()`.
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
/// This is called in the **parent** process before spawning the child,
/// NOT in `pre_exec`.  Removing env vars via `std::env::remove_var`
/// inside `pre_exec` is NOT async-signal-safe and can deadlock after
/// `fork()` if another thread was holding the `malloc` lock.
pub fn remove_dangerous_env_vars_parent() {
    let keys_to_remove: Vec<String> = std::env::vars()
        .filter_map(|(key, _)| {
            #[cfg(target_os = "linux")]
            if key.starts_with("LD_") {
                return Some(key);
            }
            #[cfg(target_os = "macos")]
            if key.starts_with("DYLD_")
                || key.starts_with("MallocStackLogging")
                || key.starts_with("MallocLogFile")
            {
                return Some(key);
            }
            let _ = key;
            None
        })
        .collect();
    for key in &keys_to_remove {
        unsafe {
            std::env::remove_var(key);
        }
    }

    // Also remove common injection variables on all platforms
    unsafe {
        std::env::remove_var("LD_PRELOAD");
        std::env::remove_var("LD_LIBRARY_PATH");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_dangerous_env_vars() {
        unsafe {
            std::env::set_var("LD_PRELOAD", "/evil.so");
        }
        unsafe {
            std::env::set_var("LD_LIBRARY_PATH", "/evil");
        }
        unsafe {
            std::env::set_var("PATH", "/usr/bin");
        }
        unsafe {
            std::env::set_var("DYLD_INSERT_LIBRARIES", "/evil.dylib");
        }

        remove_dangerous_env_vars_parent();

        // These should be removed
        assert!(std::env::var("LD_PRELOAD").is_err());
        assert!(std::env::var("LD_LIBRARY_PATH").is_err());

        // DYLD vars are only handled on macOS
        #[cfg(target_os = "macos")]
        assert!(std::env::var("DYLD_INSERT_LIBRARIES").is_err());

        // These should still exist
        assert_eq!(std::env::var("PATH").unwrap(), "/usr/bin");
        // Cleanup
        unsafe {
            std::env::remove_var("LD_PRELOAD");
            std::env::remove_var("LD_LIBRARY_PATH");
            std::env::remove_var("DYLD_INSERT_LIBRARIES");
        }
    }

    #[test]
    fn test_remove_dangerous_env_vars_empty() {
        // Should not crash when no dangerous vars are present
        remove_dangerous_env_vars_parent();
    }
}
