//! macOS Seatbelt (sandbox-exec) profile generation.
//!
//! Seatbelt is Apple's mandatory access control framework that uses the
//! Scheme-based policy language (SBPL) to define what system resources a
//! process can access. This module generates sandbox profiles dynamically
//! based on the configured `SandboxPolicy`.
//!
//! # How it works
//!
//! 1. We generate a Seatbelt policy string in SBPL format
//! 2. We invoke `/usr/bin/sandbox-exec -p <policy>` to run the command
//! 3. The kernel enforces the policy, blocking unauthorized operations
//!
//! # References
//!
//! - Apple's sandbox(7) man page
//! - https://reverse.put.as/wp-content/uploads/2011/09/Apple-Sandbox-Guide-v1.0.pdf
//! - OpenAI Codex seatbelt_base_policy.sbpl
//! - DeepSeek-TUI seatbelt.rs

use super::policy::SandboxPolicy;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

/// Path to the sandbox-exec binary on macOS.
pub const SANDBOX_EXEC_PATH: &str = "/usr/bin/sandbox-exec";

/// Base seatbelt policy that provides minimal process functionality.
///
/// This policy:
/// - Denies everything by default
/// - Allows process execution and forking
/// - Allows signals within the same sandbox
/// - Allows basic process introspection
/// - Allows writing to /dev/null
/// - Allows reading various sysctl values
/// - Allows POSIX semaphores and pseudo-TTY operations
/// - Allows Mach IPC lookups
const SEATBELT_BASE_POLICY: &str = r#"
(version 1)
(deny default)

; Core process operations
(allow process-exec)
(allow process-fork)
(allow signal (target same-sandbox))
(allow process-info* (target same-sandbox))

; User preferences (needed by many CLI tools)
(allow user-preference-read)

; Basic I/O to /dev/null
(allow file-write-data
  (require-all
    (path "/dev/null")
    (vnode-type CHARACTER-DEVICE)))

; System information
(allow sysctl-read)

; IPC primitives
(allow ipc-posix-sem)
(allow ipc-posix-shm-read*)
(allow ipc-posix-shm-write-create)
(allow ipc-posix-shm-write-data)
(allow ipc-posix-shm-write-unlink)

; Terminal support (essential for shell commands)
(allow pseudo-tty)
(allow file-read* file-write* file-ioctl (literal "/dev/ptmx"))
(allow file-read* file-write* file-ioctl (regex #"^/dev/ttys[0-9]+$"))

; macOS-specific device access
(allow file-read* (literal "/dev/urandom"))
(allow file-read* (literal "/dev/random"))
(allow file-ioctl (literal "/dev/dtracehelper"))

; Mach IPC (needed by many system services)
(allow mach-lookup)
"#;

/// Check if sandbox-exec is available and permitted on this system.
pub fn is_available() -> bool {
    static SEATBELT_AVAILABLE: OnceLock<bool> = OnceLock::new();

    *SEATBELT_AVAILABLE.get_or_init(|| {
        if !Path::new(SANDBOX_EXEC_PATH).exists() {
            return false;
        }

        // Try running sandbox-exec with a trivial "allow all" policy
        let output = Command::new(SANDBOX_EXEC_PATH)
            .args(["-p", "(version 1)(allow default)", "--", "/usr/bin/true"])
            .output();

        match output {
            Ok(result) => result.status.success(),
            Err(_) => false,
        }
    })
}

/// Create the command-line arguments for sandbox-exec.
///
/// Returns a Vec of arguments that should be prepended to the command.
/// The format is: `sandbox-exec -p <policy> -- <original command>`
pub fn create_seatbelt_args(
    command: Vec<String>,
    policy: &SandboxPolicy,
    _sandbox_cwd: &Path,
) -> Vec<String> {
    let full_policy = generate_policy(policy);

    let mut args = vec!["-p".to_string(), full_policy];

    // Separator between sandbox-exec args and the actual command
    args.push("--".to_string());
    args.extend(command);

    args
}

/// Generate the complete Seatbelt policy string for the given policy.
fn generate_policy(policy: &SandboxPolicy) -> String {
    let mut full_policy = SEATBELT_BASE_POLICY.to_string();

    // Add read access policy
    if policy.has_full_disk_read_access() {
        full_policy.push_str("\n; Full filesystem read access\n(allow file-read*)");
    }

    // Add write access policy for workspace-write
    let file_write_policy = generate_write_policy(policy);
    if !file_write_policy.is_empty() {
        full_policy.push_str("\n\n; Write access policy\n");
        full_policy.push_str(&file_write_policy);
    }

    // Add common macOS directories that tools often need
    full_policy.push_str("\n\n; Common system directories\n");
    full_policy.push_str(r#"(allow file-read* (subpath "/usr/lib"))"#);
    full_policy.push('\n');
    full_policy.push_str(r#"(allow file-read* (subpath "/usr/share"))"#);
    full_policy.push('\n');
    full_policy.push_str(r#"(allow file-read* (subpath "/System/Library"))"#);
    full_policy.push('\n');
    full_policy.push_str(r#"(allow file-read* (subpath "/Library/Preferences"))"#);

    full_policy
}

/// Generate the write policy section for a given policy.
fn generate_write_policy(policy: &SandboxPolicy) -> String {
    match policy {
        SandboxPolicy::ReadOnly
        | SandboxPolicy::ExternalSandbox
        | SandboxPolicy::DangerFullAccess => String::new(),

        SandboxPolicy::WorkspaceWrite { writable_roots } => {
            let mut write_rules = String::new();

            // Allow write to /tmp
            write_rules.push_str(r#"(allow file-write* (subpath "/tmp"))"#);
            write_rules.push('\n');

            // Allow write to TMPDIR if set
            if let Ok(tmpdir) = std::env::var("TMPDIR")
                && !tmpdir.is_empty()
            {
                write_rules.push_str(&format!(r#"(allow file-write* (subpath "{}"))"#, tmpdir));
                write_rules.push('\n');
            }

            if writable_roots.is_empty() {
                return write_rules;
            }

            // Add write rules for each writable root
            for root in writable_roots {
                let root_str = root.to_string_lossy();
                write_rules.push_str(&format!(r#"(allow file-write* (subpath "{}"))"#, root_str));
                write_rules.push('\n');
            }

            write_rules
        }
    }
}

/// Get the platform sandbox type name for this module.
pub fn sandbox_type_name() -> &'static str {
    "macos-seatbelt"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_base_policy_is_valid_sbpl() {
        // Base policy should be valid SBPL — at minimum it should compile
        // (we can't run sandbox-exec in tests since it requires entitlement)
        let policy = generate_policy(&SandboxPolicy::ReadOnly);
        assert!(policy.contains("(version 1)"));
        assert!(policy.contains("(deny default)"));
        assert!(policy.contains("(allow process-exec)"));
        assert!(policy.contains("(allow file-read*)"));
        assert!(policy.contains("(allow file-read*)"));
    }

    #[test]
    fn test_writable_roots_in_policy() {
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![PathBuf::from("/workspace")],
        };

        let profile = generate_policy(&policy);
        assert!(profile.contains("/workspace"));
    }

    #[test]
    fn test_create_seatbelt_args() {
        let policy = SandboxPolicy::ReadOnly;
        let command = vec!["sh".to_string(), "-c".to_string(), "ls -la".to_string()];

        let args = create_seatbelt_args(command.clone(), &policy, Path::new("/tmp"));

        // First arg should be -p
        assert_eq!(args[0], "-p");
        // Args should contain the command after --
        let dash_dash_pos = args.iter().position(|a| a == "--").unwrap();
        assert!(dash_dash_pos > 0);
        assert_eq!(&args[dash_dash_pos + 1..], &command[..]);
    }
}
