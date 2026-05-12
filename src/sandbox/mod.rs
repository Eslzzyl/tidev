//! Sandbox module for secure command execution.
//!
//! This module provides sandboxing capabilities for shell commands executed by
//! TiDev. Sandboxing restricts what system resources a command can access,
//! preventing accidental or malicious damage to the system.
//!
//! # Platform Support
//!
//! - **macOS**: Uses Seatbelt (sandbox-exec) for mandatory access control
//! - **Linux**: Uses Bubblewrap (bwrap) for filesystem isolation, with
//!   Landlock as a fallback when bwrap is not available
//!
//! # Architecture
//!
//! The sandbox flow is:
//!
//! 1. A `CommandSpec` describes the command to run and what resources it needs
//! 2. `SandboxManager::prepare()` transforms it into an `ExecEnv`
//! 3. The `ExecEnv` may wrap the command with sandbox-specific wrappers
//!    (e.g., `sandbox-exec` on macOS, `bwrap` on Linux)
//! 4. The caller spawns the command from `ExecEnv`

pub mod policy;
#[cfg(target_os = "macos")]
pub mod seatbelt;
#[cfg(target_os = "linux")]
pub mod launcher;
#[cfg(target_os = "linux")]
pub mod bwrap;
#[cfg(target_os = "linux")]
pub mod landlock;

mod process_hardening;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

pub use policy::SandboxPolicy;
pub use process_hardening::pre_exec_hardening;

// ---------------------------------------------------------------------------
// CommandSpec
// ---------------------------------------------------------------------------

/// Specification for a command to be executed, potentially within a sandbox.
///
/// This struct captures all the information needed to execute a command:
/// the program and arguments, working directory, environment variables,
/// timeout, and sandbox policy.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    /// The program to execute (e.g., "sh", "python", "cargo").
    pub program: String,

    /// Arguments to pass to the program.
    pub args: Vec<String>,

    /// Working directory for the command.
    pub cwd: PathBuf,

    /// Additional environment variables to set.
    pub env: HashMap<String, String>,

    /// Maximum execution time before the command is killed.
    pub timeout: Duration,

    /// Sandbox policy controlling resource access.
    pub sandbox_policy: SandboxPolicy,

    /// Optional justification for why this command needs to run.
    /// Used for logging and audit purposes.
    pub justification: Option<String>,
}

impl CommandSpec {
    /// Create a `CommandSpec` for running a shell command via the platform shell.
    pub fn shell(command: &str, cwd: PathBuf, timeout: Duration) -> Self {
        #[cfg(windows)]
        let (program, args) = (
            "cmd".to_string(),
            vec!["/C".to_string(), command.to_string()],
        );
        #[cfg(not(windows))]
        let (program, args) = (
            "sh".to_string(),
            vec!["-c".to_string(), command.to_string()],
        );

        Self {
            program,
            args,
            cwd,
            env: HashMap::new(),
            timeout,
            sandbox_policy: SandboxPolicy::default(),
            justification: None,
        }
    }

    /// Create a `CommandSpec` for running a program directly.
    pub fn program(program: &str, args: Vec<String>, cwd: PathBuf, timeout: Duration) -> Self {
        Self {
            program: program.to_string(),
            args,
            cwd,
            env: HashMap::new(),
            timeout,
            sandbox_policy: SandboxPolicy::default(),
            justification: None,
        }
    }

    /// Set the sandbox policy for this command.
    pub fn with_policy(mut self, policy: SandboxPolicy) -> Self {
        self.sandbox_policy = policy;
        self
    }

    /// Add environment variables for this command.
    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        self.env = env;
        self
    }

    /// Add a single environment variable.
    pub fn with_env_var(mut self, key: &str, value: &str) -> Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }

    /// Set a justification for this command (for logging/audit).
    pub fn with_justification(mut self, justification: &str) -> Self {
        self.justification = Some(justification.to_string());
        self
    }

    /// Get the original command as a single string (for display).
    pub fn display_command(&self) -> String {
        if self.program == "sh" && self.args.len() == 2 && self.args[0] == "-c" {
            // For shell commands, show the actual command
            self.args[1].clone()
        } else {
            // For other commands, join program and args
            let mut parts = vec![self.program.clone()];
            parts.extend(self.args.clone());
            parts.join(" ")
        }
    }
}

// ---------------------------------------------------------------------------
// SandboxType
// ---------------------------------------------------------------------------

/// The type of sandbox being used for execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxType {
    /// No sandboxing - command runs with full permissions.
    #[default]
    None,

    /// macOS Seatbelt (sandbox-exec) sandboxing.
    #[cfg(target_os = "macos")]
    MacosSeatbelt,

    /// Linux Bubblewrap (bwrap) sandboxing.
    #[cfg(target_os = "linux")]
    LinuxBubblewrap,

    /// Linux Landlock in-process sandboxing (fallback).
    #[cfg(target_os = "linux")]
    LinuxLandlock,
}

impl std::fmt::Display for SandboxType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxType::None => write!(f, "none"),
            #[cfg(target_os = "macos")]
            SandboxType::MacosSeatbelt => write!(f, "macos-seatbelt"),
            #[cfg(target_os = "linux")]
            SandboxType::LinuxBubblewrap => write!(f, "linux-bubblewrap"),
            #[cfg(target_os = "linux")]
            SandboxType::LinuxLandlock => write!(f, "linux-landlock"),
        }
    }
}

// ---------------------------------------------------------------------------
// ExecEnv
// ---------------------------------------------------------------------------

/// The execution environment after sandbox transformation.
///
/// This contains the actual command to run (which may include sandbox wrapper
/// commands) and all necessary environment configuration.
#[derive(Debug)]
pub struct ExecEnv {
    /// The full command to execute (may include sandbox wrapper).
    pub command: Vec<String>,

    /// Working directory for execution.
    pub cwd: PathBuf,

    /// Environment variables to set.
    pub env: HashMap<String, String>,

    /// Timeout for the command.
    pub timeout: Duration,

    /// The type of sandbox being used.
    pub sandbox_type: SandboxType,

    /// The original policy (for reference).
    pub policy: SandboxPolicy,
}

impl ExecEnv {
    /// Get the program to execute (first element of command).
    pub fn program(&self) -> &str {
        self.command
            .first()
            .map(std::string::String::as_str)
            .unwrap_or("sh")
    }

    /// Get the arguments (all elements after the first).
    pub fn args(&self) -> &[String] {
        if self.command.len() > 1 {
            &self.command[1..]
        } else {
            &[]
        }
    }

    /// Check if this execution is sandboxed.
    pub fn is_sandboxed(&self) -> bool {
        !matches!(self.sandbox_type, SandboxType::None)
    }
}

// ---------------------------------------------------------------------------
// Platform detection
// ---------------------------------------------------------------------------

/// Detect what sandbox technology is available on the current platform.
pub fn get_platform_sandbox() -> Option<SandboxType> {
    #[cfg(target_os = "macos")]
    {
        if seatbelt::is_available() {
            return Some(SandboxType::MacosSeatbelt);
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Prefer bubblewrap over Landlock (stronger isolation)
        if launcher::is_bwrap_available() {
            return Some(SandboxType::LinuxBubblewrap);
        }
        if landlock::is_landlock_available() {
            return Some(SandboxType::LinuxLandlock);
        }
    }

    None
}

/// Check if sandboxing is available on this platform.
pub fn is_sandbox_available() -> bool {
    get_platform_sandbox().is_some()
}

// ---------------------------------------------------------------------------
// SandboxManager
// ---------------------------------------------------------------------------

/// Manager for sandbox operations.
///
/// The `SandboxManager` is responsible for:
/// - Detecting available sandbox technologies
/// - Transforming `CommandSpecs` into sandboxed `ExecEnvs`
#[derive(Debug, Default)]
pub struct SandboxManager {
    /// Cached sandbox availability check.
    sandbox_available: Option<bool>,
}

impl SandboxManager {
    /// Create a new `SandboxManager`.
    pub fn new() -> Self {
        Self {
            sandbox_available: None,
        }
    }

    /// Check if sandboxing is available.
    pub fn is_available(&mut self) -> bool {
        if let Some(available) = self.sandbox_available {
            return available;
        }

        let available = is_sandbox_available();
        self.sandbox_available = Some(available);
        available
    }

    /// Transform a `CommandSpec` into an `ExecEnv`.
    ///
    /// This is the core method:
    /// - If the policy is `DangerFullAccess` or `ExternalSandbox`, the command
    ///   is returned as-is (no sandboxing).
    /// - If the policy is `ReadOnly` or `WorkspaceWrite`, the command is wrapped
    ///   with the platform sandbox (e.g., `sandbox-exec` on macOS, `bwrap` on Linux).
    pub fn prepare(&self, spec: &CommandSpec) -> ExecEnv {
        // If no sandboxing is needed, return the command as-is
        match spec.sandbox_policy {
            SandboxPolicy::DangerFullAccess | SandboxPolicy::ExternalSandbox => {
                return self.prepare_plain(spec);
            }
            _ => {}
        }

        // Platform-specific sandbox preparation
        #[cfg(target_os = "macos")]
        {
            if seatbelt::is_available() {
                return self.prepare_seatbelt(spec);
            }
        }

        #[cfg(target_os = "linux")]
        {
            if launcher::is_bwrap_available() {
                return self.prepare_bubblewrap(spec);
            }
            if landlock::is_landlock_available() {
                return self.prepare_landlock(spec);
            }
        }

        // Fallback: no sandbox available
        self.prepare_plain(spec)
    }

    /// Prepare a command without sandboxing.
    fn prepare_plain(&self, spec: &CommandSpec) -> ExecEnv {
        let mut command = vec![spec.program.clone()];
        command.extend(spec.args.clone());

        ExecEnv {
            command,
            cwd: spec.cwd.clone(),
            env: spec.env.clone(),
            timeout: spec.timeout,
            sandbox_type: SandboxType::None,
            policy: spec.sandbox_policy.clone(),
        }
    }

    /// Prepare a command with macOS Seatbelt sandboxing.
    #[cfg(target_os = "macos")]
    fn prepare_seatbelt(&self, spec: &CommandSpec) -> ExecEnv {
        let mut command = vec![spec.program.clone()];
        command.extend(spec.args.clone());

        let seatbelt_args = seatbelt::create_seatbelt_args(command, &spec.sandbox_policy, &spec.cwd);

        let env = spec.env.clone();

        ExecEnv {
            command: {
                let mut cmd = vec![seatbelt::SANDBOX_EXEC_PATH.to_string()];
                cmd.extend(seatbelt_args);
                cmd
            },
            cwd: spec.cwd.clone(),
            env,
            timeout: spec.timeout,
            sandbox_type: SandboxType::MacosSeatbelt,
            policy: spec.sandbox_policy.clone(),
        }
    }

    /// Prepare a command with Linux Bubblewrap sandboxing.
    ///
    /// Bubblewrap creates a new mount namespace with restricted filesystem
    /// access. The command is wrapped as `bwrap <args> -- <original_command>`.
    #[cfg(target_os = "linux")]
    fn prepare_bubblewrap(&self, spec: &CommandSpec) -> ExecEnv {
        let bwrap_path = match launcher::find_system_bwrap() {
            Some(path) => path,
            None => {
                // Fallback to plain execution if bwrap is not available
                return self.prepare_plain(spec);
            }
        };

        // Collect writable roots from policy
        let extra_roots = match &spec.sandbox_policy {
            SandboxPolicy::WorkspaceWrite { writable_roots, .. } => writable_roots.clone(),
            _ => vec![],
        };

        let (program, args) = bwrap::build_bwrap_command(
            &bwrap_path,
            &spec.sandbox_policy,
            &spec.cwd,
            &extra_roots,
            &spec.program,
            &spec.args,
        );

        ExecEnv {
            command: {
                let mut cmd = vec![program];
                cmd.extend(args);
                cmd
            },
            cwd: spec.cwd.clone(),
            env: spec.env.clone(),
            timeout: spec.timeout,
            sandbox_type: SandboxType::LinuxBubblewrap,
            policy: spec.sandbox_policy.clone(),
        }
    }

    /// Prepare a command with Linux Landlock sandboxing.
    ///
    /// Landlock applies in-process filesystem restrictions. Unlike bubblewrap
    /// (which wraps the command), Landlock must be applied in the child process
    /// before exec(). The ExecEnv returned here uses the original command;
    /// the caller is responsible for calling `landlock::apply_landlock_policy()`
    /// via `pre_exec()` before executing the command.
    #[cfg(target_os = "linux")]
    fn prepare_landlock(&self, spec: &CommandSpec) -> ExecEnv {
        let command = {
            let mut cmd = vec![spec.program.clone()];
            cmd.extend(spec.args.clone());
            cmd
        };

        // Landlock does not wrap the command; it restricts the process
        // in-place before exec. The command runs as-is.
        ExecEnv {
            command,
            cwd: spec.cwd.clone(),
            env: spec.env.clone(),
            timeout: spec.timeout,
            sandbox_type: SandboxType::LinuxLandlock,
            policy: spec.sandbox_policy.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Convenience function for external callers
// ---------------------------------------------------------------------------

/// Prepare a `CommandSpec` for execution with sandboxing, returning an
/// `ExecEnv` that can be used to spawn the process.
pub fn prepare_command(spec: CommandSpec) -> ExecEnv {
    let manager = SandboxManager::new();
    manager.prepare(&spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_spec_shell() {
        let spec = CommandSpec::shell(
            "echo hello",
            PathBuf::from("/tmp"),
            Duration::from_secs(30),
        );
        assert_eq!(spec.program, "sh");
        assert_eq!(spec.args, vec!["-c", "echo hello"]);
        assert_eq!(spec.cwd, PathBuf::from("/tmp"));
        assert_eq!(spec.timeout, Duration::from_secs(30));
        assert!(matches!(spec.sandbox_policy, SandboxPolicy::WorkspaceWrite { .. }));
    }

    #[test]
    fn test_command_spec_program() {
        let spec = CommandSpec::program(
            "python",
            vec!["script.py".to_string()],
            PathBuf::from("/workspace"),
            Duration::from_secs(60),
        );
        assert_eq!(spec.program, "python");
        assert_eq!(spec.args, vec!["script.py"]);
    }

    #[test]
    fn test_command_spec_with_policy() {
        let spec = CommandSpec::shell("ls", PathBuf::from("."), Duration::from_secs(10))
            .with_policy(SandboxPolicy::ReadOnly);
        assert_eq!(spec.sandbox_policy, SandboxPolicy::ReadOnly);
    }

    #[test]
    fn test_command_spec_with_env() {
        let spec = CommandSpec::shell("echo", PathBuf::from("."), Duration::from_secs(10))
            .with_env_var("KEY", "VALUE");
        assert_eq!(spec.env.get("KEY"), Some(&"VALUE".to_string()));
    }

    #[test]
    fn test_display_command_shell() {
        let spec = CommandSpec::shell("ls -la", PathBuf::from("."), Duration::from_secs(10));
        assert_eq!(spec.display_command(), "ls -la");
    }

    #[test]
    fn test_display_command_program() {
        let spec = CommandSpec::program(
            "cargo",
            vec!["build".to_string()],
            PathBuf::from("."),
            Duration::from_secs(120),
        );
        assert_eq!(spec.display_command(), "cargo build");
    }

    #[test]
    fn test_prepare_danger_full_access() {
        let spec = CommandSpec::shell("rm -rf /", PathBuf::from("/"), Duration::from_secs(10))
            .with_policy(SandboxPolicy::DangerFullAccess);
        let manager = SandboxManager::new();
        let env = manager.prepare(&spec);
        assert!(!env.is_sandboxed());
        assert_eq!(env.sandbox_type, SandboxType::None);
    }

    #[test]
    fn test_prepare_readonly_no_sandbox_available() {
        let spec = CommandSpec::shell("ls", PathBuf::from("."), Duration::from_secs(10))
            .with_policy(SandboxPolicy::ReadOnly);
        let manager = SandboxManager::new();
        let env = manager.prepare(&spec);
        // When sandbox is available (macOS in CI), the command is wrapped with
        // sandbox-exec. When unavailable, it falls through to plain execution.
        // Either way it should not crash.
        if cfg!(not(target_os = "macos")) {
            assert_eq!(env.program(), "sh");
            assert_eq!(env.args(), &["-c", "ls"]);
        } else {
            // On macOS, sandbox-exec should wrap the command
            assert_eq!(env.program(), "/usr/bin/sandbox-exec");
        }
    }

    #[test]
    fn test_exec_env_program_and_args() {
        let env = ExecEnv {
            command: vec![
                "sh".to_string(),
                "-c".to_string(),
                "echo test".to_string(),
            ],
            cwd: PathBuf::from("/tmp"),
            env: HashMap::new(),
            timeout: Duration::from_secs(30),
            sandbox_type: SandboxType::None,
            policy: SandboxPolicy::ReadOnly,
        };
        assert_eq!(env.program(), "sh");
        assert_eq!(env.args(), &["-c", "echo test"]);
        assert!(!env.is_sandboxed());
    }
}
