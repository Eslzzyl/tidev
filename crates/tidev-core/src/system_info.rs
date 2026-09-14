//! System environment information detection for the model-visible environment block.

use std::path::Path;
use std::process::Command;

/// Operating system type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsType {
    Windows,
    Macos,
    Linux,
    Unknown,
}

impl OsType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Unknown => "unknown",
        }
    }
}

/// Detected system environment information.
#[derive(Debug, Clone)]
pub struct SystemInfo {
    pub os_type: OsType,
    pub is_wsl: bool,
    pub is_ssh: bool,
    pub current_date: String,
}

impl SystemInfo {
    pub fn detect() -> Self {
        Self {
            os_type: detect_os_type(),
            is_wsl: detect_wsl(),
            is_ssh: detect_ssh(),
            current_date: chrono::Local::now().format("%Y-%m-%d").to_string(),
        }
    }

    pub fn format_env(&self) -> String {
        let mut lines = vec![format!("Platform: {}", self.os_type.as_str())];

        if self.is_wsl {
            lines.push("WSL: yes".to_string());
        }

        if self.is_ssh {
            lines.push("SSH session: yes".to_string());
        }

        lines.push(format!("Today's date: {}", self.current_date));
        lines.join("\n  ")
    }
}

/// Check if the given directory is a git repository.
pub fn is_git_repo(workspace_root: &Path) -> bool {
    let output = Command::new("git")
        .current_dir(workspace_root)
        .args(["status"])
        .output();

    if let Ok(output) = output {
        return output.status.success();
    }

    workspace_root.join(".git").exists()
}

fn detect_os_type() -> OsType {
    #[cfg(target_os = "windows")]
    {
        OsType::Windows
    }

    #[cfg(target_os = "macos")]
    {
        OsType::Macos
    }

    #[cfg(target_os = "linux")]
    {
        OsType::Linux
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        OsType::Unknown
    }
}

fn detect_wsl() -> bool {
    if std::env::var("WSL_DISTRO_NAME").is_ok() {
        return true;
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(version) = std::fs::read_to_string("/proc/version") {
            let version = version.to_lowercase();
            return version.contains("microsoft") || version.contains("wsl");
        }
    }

    false
}

fn detect_ssh() -> bool {
    std::env::var("SSH_CLIENT").is_ok()
        || std::env::var("SSH_CONNECTION").is_ok()
        || std::env::var("SSH_TTY").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_known_platform() {
        assert_ne!(detect_os_type(), OsType::Unknown);
    }

    #[test]
    fn formats_retained_environment_fields() {
        let info = SystemInfo {
            os_type: OsType::Linux,
            is_wsl: true,
            is_ssh: true,
            current_date: "2026-09-14".to_string(),
        };

        let environment = info.format_env();
        assert!(environment.contains("Platform: linux"));
        assert!(environment.contains("WSL: yes"));
        assert!(environment.contains("SSH session: yes"));
        assert!(environment.contains("Today's date: 2026-09-14"));
    }
}
