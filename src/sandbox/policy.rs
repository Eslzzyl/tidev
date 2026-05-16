//! Sandbox policy definitions for command execution restrictions.
//!
//! This module defines the policies that control what resources a sandboxed
//! process can access. Policies range from full unrestricted access to
//! tightly controlled workspace-only write access.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Determines execution restrictions for shell commands.
///
/// The sandbox policy controls filesystem access for executed commands.
/// Network access is always allowed regardless of the policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SandboxPolicy {
    /// No restrictions whatsoever. Use with extreme caution.
    #[serde(rename = "danger-full-access")]
    DangerFullAccess,

    /// Read-only access to the entire filesystem.
    /// The process can read any file but cannot write anywhere.
    #[serde(rename = "read-only")]
    ReadOnly,

    /// Indicates the process is already running in an external sandbox.
    /// This avoids double-sandboxing which can cause issues.
    #[serde(rename = "external-sandbox")]
    ExternalSandbox,

    /// Read-only filesystem access plus write access to specified directories.
    ///
    /// This is the default and recommended policy. It allows:
    /// - Read access to the entire filesystem
    /// - Write access only to the current working directory and specified roots
    #[serde(rename = "workspace-write")]
    WorkspaceWrite {
        /// Additional directories where writes are allowed.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        writable_roots: Vec<PathBuf>,
    },
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
        }
    }
}

impl SandboxPolicy {
    /// Check if this policy allows full disk read access.
    pub fn has_full_disk_read_access(&self) -> bool {
        matches!(
            self,
            SandboxPolicy::DangerFullAccess
                | SandboxPolicy::ReadOnly
                | SandboxPolicy::WorkspaceWrite { .. }
        )
    }

    /// Check if a specific path is writable under this policy.
    pub fn is_path_writable(&self, path: &Path, cwd: &Path) -> bool {
        match self {
            SandboxPolicy::DangerFullAccess => true,
            SandboxPolicy::ReadOnly | SandboxPolicy::ExternalSandbox => false,
            SandboxPolicy::WorkspaceWrite { .. } => {
                if path.starts_with(cwd) {
                    return true;
                }
                if path.starts_with("/tmp") {
                    return true;
                }
                if let Ok(tmpdir) = std::env::var("TMPDIR")
                    && path.starts_with(&tmpdir)
                {
                    return true;
                }
                false
            }
        }
    }

    /// Convert a policy name string to the enum.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "danger-full-access" | "full_access" | "full" => Some(SandboxPolicy::DangerFullAccess),
            "read-only" | "read_only" | "readonly" | "read" => Some(SandboxPolicy::ReadOnly),
            "external-sandbox" | "external_sandbox" | "external" => {
                Some(SandboxPolicy::ExternalSandbox)
            }
            "workspace-write" | "workspace_write" | "workspace" | "default" => {
                Some(SandboxPolicy::default())
            }
            _ => None,
        }
    }

    /// Return a human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            SandboxPolicy::DangerFullAccess => "off",
            SandboxPolicy::ReadOnly => "read-only",
            SandboxPolicy::ExternalSandbox => "external sandbox",
            SandboxPolicy::WorkspaceWrite { .. } => "workspace-write",
        }
    }
}

/// A writable root directory with an optional list of exception paths
/// that must remain read-only even within the writable root.
#[derive(Debug, Clone)]
pub struct WritableRoot {
    /// The root directory that is writable.
    pub root: PathBuf,
    /// Paths within `root` that must remain read-only.
    pub exceptions: Vec<PathBuf>,
}

impl WritableRoot {
    /// Create a new writable root.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            exceptions: Vec::new(),
        }
    }

    /// Create a writable root with exception paths.
    pub fn with_exceptions(root: PathBuf, exceptions: Vec<PathBuf>) -> Self {
        Self { root, exceptions }
    }

    /// Check if a path is writable within this root.
    pub fn is_path_writable(&self, path: &Path) -> bool {
        if !path.starts_with(&self.root) {
            return false;
        }
        for exception in &self.exceptions {
            if path.starts_with(exception) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy() {
        let policy = SandboxPolicy::default();
        assert!(matches!(policy, SandboxPolicy::WorkspaceWrite { .. }));
        assert!(policy.has_full_disk_read_access());
    }

    #[test]
    fn test_read_only_policy() {
        let policy = SandboxPolicy::ReadOnly;
        assert!(policy.has_full_disk_read_access());
        assert!(!policy.is_path_writable(Path::new("/etc"), Path::new("/workspace")));
    }

    #[test]
    fn test_danger_full_access_policy() {
        let policy = SandboxPolicy::DangerFullAccess;
        assert!(policy.has_full_disk_read_access());
        assert!(policy.is_path_writable(Path::new("/etc"), Path::new("/workspace")));
    }

    #[test]
    fn test_workspace_writable() {
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
        };
        assert!(policy.has_full_disk_read_access());

        let cwd = Path::new("/workspace");
        assert!(policy.is_path_writable(Path::new("/workspace/src/main.rs"), cwd));
        assert!(policy.is_path_writable(Path::new("/tmp/build"), cwd));
        assert!(!policy.is_path_writable(Path::new("/etc/config"), cwd));
    }

    #[test]
    fn test_policy_from_name() {
        assert!(matches!(
            SandboxPolicy::from_name("full"),
            Some(SandboxPolicy::DangerFullAccess)
        ));
        assert!(matches!(
            SandboxPolicy::from_name("read-only"),
            Some(SandboxPolicy::ReadOnly)
        ));
        assert!(matches!(
            SandboxPolicy::from_name("workspace-write"),
            Some(SandboxPolicy::WorkspaceWrite { .. })
        ));
        assert!(SandboxPolicy::from_name("unknown").is_none());
    }

    #[test]
    fn test_writable_root() {
        let root = WritableRoot::new(PathBuf::from("/project"));
        assert!(root.is_path_writable(Path::new("/project/src/main.rs")));
        assert!(!root.is_path_writable(Path::new("/other/file.txt")));
    }

    #[test]
    fn test_writable_root_with_exceptions() {
        let root = WritableRoot::with_exceptions(
            PathBuf::from("/project"),
            vec![PathBuf::from("/project/.tidev")],
        );
        assert!(root.is_path_writable(Path::new("/project/src/main.rs")));
        assert!(!root.is_path_writable(Path::new("/project/.tidev/config")));
    }

    #[test]
    fn test_policy_serialization() {
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![PathBuf::from("/extra")],
        };

        let json = serde_json::to_string(&policy).unwrap();
        assert!(json.contains("workspace-write"));

        let parsed: SandboxPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, parsed);
    }
}
