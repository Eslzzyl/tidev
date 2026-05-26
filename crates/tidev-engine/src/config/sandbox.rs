//! Sandbox configuration for TiDev.
//!
//! Defines the `SandboxConfig` struct which controls how shell commands are
//! sandboxed during execution. Configuration is stored in the `[sandbox]`
//! section of `config.toml`.

use serde::{Deserialize, Serialize};

/// Controls sandbox behavior for shell command execution.
///
/// The sandbox restricts what filesystem resources a shell command can
/// access. Network access is always allowed.
///
/// Supported modes:
/// - `workspace-write` (default on Unix): Read access everywhere, write restricted to workspace and /tmp.
/// - `read-only`: Read-only access to the entire filesystem.
/// - `danger-full-access` (default on Windows): No filesystem restrictions.
///
/// Note: Sandbox enforcement is not implemented on Windows. Any mode other
/// than `danger-full-access` would give a false sense of security while also
/// routing through a `cmd /C` shell path, so `to_policy()` always returns
/// `DangerFullAccess` on Windows regardless of the configured mode.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    /// Sandbox mode: "workspace-write" (default), "read-only", or "danger-full-access".
    pub mode: String,

    /// Additional directories where writes are allowed (workspace-write only).
    /// Each entry should be an absolute path.
    #[serde(default)]
    pub writable_roots: Vec<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        #[cfg(windows)]
        {
            // Sandbox is not supported on Windows; default to unrestricted.
            Self {
                mode: "danger-full-access".to_string(),
                writable_roots: Vec::new(),
            }
        }
        #[cfg(not(windows))]
        Self {
            mode: "workspace-write".to_string(),
            writable_roots: Vec::new(),
        }
    }
}

impl SandboxConfig {
    /// Convert this config to a `SandboxPolicy` from the sandbox module.
    pub fn to_policy(&self) -> crate::sandbox::SandboxPolicy {
        #[cfg(windows)]
        {
            // Sandbox enforcement is not implemented on Windows.
            // Using any non-DangerFullAccess policy would give a false sense
            // of security while also routing through a cmd /C shell path.
            let _ = self;
            crate::sandbox::SandboxPolicy::DangerFullAccess
        }

        #[cfg(not(windows))]
        {
            use crate::sandbox::SandboxPolicy;

            match self.mode.as_str() {
                "danger-full-access" | "full_access" | "full" => SandboxPolicy::DangerFullAccess,
                "read-only" | "read_only" => SandboxPolicy::ReadOnly,
                _ => SandboxPolicy::WorkspaceWrite {
                    writable_roots: self
                        .writable_roots
                        .iter()
                        .map(std::path::PathBuf::from)
                        .collect(),
                },
            }
        }
    }
}
