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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    /// Sandbox mode: "workspace-write" (default) or "danger-full-access".
    ///
    /// - `workspace-write`: Read access everywhere, write access restricted
    ///   to the current working directory and /tmp.
    /// - `danger-full-access`: No filesystem restrictions.
    pub mode: String,

    /// Additional directories where writes are allowed (workspace-write only).
    /// Each entry should be an absolute path.
    #[serde(default)]
    pub writable_roots: Vec<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            mode: "workspace-write".to_string(),
            writable_roots: Vec::new(),
        }
    }
}

impl SandboxConfig {
    /// Convert this config to a `SandboxPolicy` from the sandbox module.
    pub fn to_policy(&self) -> crate::sandbox::SandboxPolicy {
        use crate::sandbox::SandboxPolicy;

        match self.mode.as_str() {
            "danger-full-access" | "full_access" | "full" => SandboxPolicy::DangerFullAccess,
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
