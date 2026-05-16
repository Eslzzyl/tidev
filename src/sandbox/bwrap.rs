//! Bubblewrap-based filesystem sandboxing for Linux.
//!
//! Bubblewrap (`bwrap`) is a user-space sandbox that uses Linux user
//! namespaces to construct restricted filesystem views. It is the preferred
//! sandbox on Linux because it provides stronger isolation than Landlock
//! (separate mount namespace, PID namespace).
//!
//! # How it works
//!
//! 1. We construct a set of `bwrap` arguments that define the filesystem view
//! 2. The command is wrapped as: `bwrap <args> -- <original_command>`
//! 3. bwrap creates a new mount namespace with the specified bind mounts
//! 4. The inner command runs with restricted filesystem access
//!
//! # Security model
//!
//! - The root filesystem is mounted read-only by default (`--ro-bind / /`)
//! - Explicit writable directories are layered on top with `--bind`
//! - `/proc` is mounted fresh to hide host process information
//! - `/tmp` is a fresh tmpfs (or bind-mounted from host)
//! - PID namespace isolates the process tree

use super::policy::SandboxPolicy;
use std::path::Path;
use std::path::PathBuf;

/// The name of the bubblewrap binary to use.
pub const BWRAP_BINARY: &str = "bwrap";

/// Create the bubblewrap arguments for a given sandbox policy.
///
/// Returns a list of arguments that, when prefixed with `bwrap` and
/// suffixed with `-- <original_command>`, sandbox the command according
/// to `policy`.
pub fn create_bwrap_args(
    policy: &SandboxPolicy,
    cwd: &Path,
    writable_roots: &[PathBuf],
) -> Vec<String> {
    let mut args = Vec::new();

    // --unshare-ipc: separate IPC namespace
    // --unshare-pid: separate PID namespace (process isolation)
    // --unshare-uts: separate host/domain name
    // --die-with-parent: kill sandbox if parent dies
    args.push("--unshare-ipc".to_string());
    args.push("--unshare-pid".to_string());
    args.push("--unshare-uts".to_string());
    args.push("--die-with-parent".to_string());
    args.push("--hostname".to_string());
    args.push("tidev".to_string());

    // Default: mount entire filesystem as read-only
    args.push("--ro-bind".to_string());
    args.push("/".to_string());
    args.push("/".to_string());

    // Add writable roots on top of the read-only view
    let writable_dirs = compute_writable_dirs(policy, cwd, writable_roots);
    for dir in &writable_dirs {
        args.push("--bind".to_string());
        args.push(dir.to_string_lossy().to_string());
        args.push(dir.to_string_lossy().to_string());
    }

    // Set up /dev (minimal device nodes)
    args.push("--dev".to_string());
    args.push("/dev".to_string());

    // Set up /proc (fresh process info)
    args.push("--proc".to_string());
    args.push("/proc".to_string());

    // Set up /tmp as a fresh tmpfs
    args.push("--tmpfs".to_string());
    args.push("/tmp".to_string());

    // If TMPDIR is set and different from /tmp, also mount it
    if let Ok(tmpdir) = std::env::var("TMPDIR") {
        let tmpdir_path = PathBuf::from(&tmpdir);
        if tmpdir_path.is_absolute() && tmpdir_path != PathBuf::from("/tmp") {
            args.push("--bind".to_string());
            args.push(tmpdir.to_string());
            args.push(tmpdir);
        }
    }

    // Separator between bwrap args and the command to execute
    args.push("--".to_string());

    args
}

/// Compute the list of writable directories for a given policy.
///
/// For `ReadOnly` policies, this returns an empty list.
/// For `WorkspaceWrite`, this returns `[cwd, /tmp, TMPDIR]` plus any
/// user-configured writable roots.
fn compute_writable_dirs(
    policy: &SandboxPolicy,
    cwd: &Path,
    extra_roots: &[PathBuf],
) -> Vec<PathBuf> {
    match policy {
        SandboxPolicy::ReadOnly
        | SandboxPolicy::DangerFullAccess
        | SandboxPolicy::ExternalSandbox => vec![],

        SandboxPolicy::WorkspaceWrite { writable_roots: _ } => {
            let mut dirs = Vec::new();

            // cwd is always writable
            dirs.push(cwd.to_path_buf());

            // /tmp is always writable
            dirs.push(PathBuf::from("/tmp"));

            // TMPDIR is always writable if set
            if let Ok(tmpdir) = std::env::var("TMPDIR") {
                let p = PathBuf::from(&tmpdir);
                if p.is_absolute() {
                    dirs.push(p);
                }
            }

            // User-configured extra writable roots
            for root in extra_roots {
                if root.is_absolute() {
                    dirs.push(root.clone());
                }
            }

            dirs
        }
    }
}

/// Build the full command line for a bubblewrap-sandboxed command.
///
/// Returns `(program, args)` where `program` is the bwrap binary path
/// and `args` are all arguments including the command to execute.
pub fn build_bwrap_command(
    bwrap_path: &Path,
    policy: &SandboxPolicy,
    cwd: &Path,
    writable_roots: &[PathBuf],
    command_program: &str,
    command_args: &[String],
) -> (String, Vec<String>) {
    let mut args = create_bwrap_args(policy, cwd, writable_roots);

    // Append the actual command
    args.push(command_program.to_string());
    args.extend(command_args.iter().cloned());

    (bwrap_path.to_string_lossy().to_string(), args)
}

/// Return a warning message if the system is WSL1 (unsupported for bwrap).
pub fn wsl1_warning() -> Option<String> {
    if cfg!(target_os = "linux") && crate::sandbox::launcher::is_wsl1() {
        Some(
            "WSL1 does not support the user namespaces required by bubblewrap. \
             Sandboxed shell commands will fall back to Landlock isolation."
                .to_string(),
        )
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::SandboxPolicy;

    #[test]
    fn test_create_bwrap_args_readonly() {
        let policy = SandboxPolicy::ReadOnly;
        let args = create_bwrap_args(&policy, Path::new("/workspace"), &[]);

        assert!(args.contains(&"--ro-bind".to_string()));
        assert!(args.contains(&"--unshare-pid".to_string()));

        // Should not have any --bind (writeable mounts)
        let bind_count = args.iter().filter(|a| *a == "--bind").count();
        assert_eq!(bind_count, 0);

        // Should have --ro-bind for /
        let ro_bind_pos = args.iter().position(|a| a == "--ro-bind").unwrap();
        assert_eq!(args[ro_bind_pos + 1], "/");
        assert_eq!(args[ro_bind_pos + 2], "/");
    }

    #[test]
    fn test_create_bwrap_args_workspace_write() {
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
        };
        let args = create_bwrap_args(&policy, Path::new("/workspace"), &[]);

        // Should have --bind for /workspace
        let bind_pos = args.iter().position(|a| a == "--bind").unwrap();
        assert_eq!(args[bind_pos + 1], "/workspace");
        assert_eq!(args[bind_pos + 2], "/workspace");

        // Network is always allowed — no --unshare-net
        assert!(!args.contains(&"--unshare-net".to_string()));

        // Should have /dev, /proc, /tmp
        assert!(args.contains(&"/dev".to_string()));
        assert!(args.contains(&"/proc".to_string()));
        assert!(args.contains(&"/tmp".to_string()));
    }

    #[test]
    fn test_compute_writable_dirs_readonly() {
        let policy = SandboxPolicy::ReadOnly;
        let dirs = compute_writable_dirs(&policy, Path::new("/workspace"), &[]);
        assert!(dirs.is_empty());
    }

    #[test]
    fn test_compute_writable_dirs_workspace_write() {
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
        };
        let dirs = compute_writable_dirs(&policy, Path::new("/workspace"), &[]);
        assert!(dirs.contains(&PathBuf::from("/workspace")));
        assert!(dirs.contains(&PathBuf::from("/tmp")));
    }

    #[test]
    fn test_build_bwrap_command() {
        let policy = SandboxPolicy::ReadOnly;
        let (program, args) = build_bwrap_command(
            Path::new("/usr/bin/bwrap"),
            &policy,
            Path::new("/workspace"),
            &[],
            "sh",
            &["-c".to_string(), "echo hi".to_string()],
        );

        assert_eq!(program, "/usr/bin/bwrap");
        assert!(args.contains(&"--".to_string()));
        let dash_dash_pos = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(args[dash_dash_pos + 1], "sh");
        assert_eq!(args[dash_dash_pos + 2], "-c");
        assert_eq!(args[dash_dash_pos + 3], "echo hi");
    }

    #[test]
    fn test_writable_dirs_workspace_with_extra_roots() {
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![PathBuf::from("/extra")],
        };
        let dirs =
            compute_writable_dirs(&policy, Path::new("/workspace"), &[PathBuf::from("/extra")]);
        assert!(dirs.contains(&PathBuf::from("/workspace")));
        assert!(dirs.contains(&PathBuf::from("/tmp")));
        assert!(dirs.contains(&PathBuf::from("/extra")));
    }
}
