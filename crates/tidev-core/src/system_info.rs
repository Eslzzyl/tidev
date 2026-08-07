//! System environment information detection for providing context to the LLM.

use std::path::Path;
use std::process::Command;

use tidev_utils::encoding::decode_command_output;

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
    pub kernel_version: Option<String>,
    pub is_wsl: bool,
    pub is_ssh: bool,
    pub current_date: String,
}

impl SystemInfo {
    pub fn detect() -> Self {
        let os_type = detect_os_type();
        let kernel_version = detect_kernel_version();
        let is_wsl = detect_wsl();
        let is_ssh = detect_ssh();
        let current_date = chrono::Local::now().format("%Y-%m-%d").to_string();

        Self {
            os_type,
            kernel_version,
            is_wsl,
            is_ssh,
            current_date,
        }
    }

    pub fn format_env(&self) -> String {
        let mut lines = Vec::new();

        lines.push(format!("Platform: {}", self.os_type.as_str()));

        if let Some(ref kernel) = self.kernel_version {
            lines.push(format!("Kernel: {}", kernel));
        }

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

fn detect_kernel_version() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(version) = std::fs::read_to_string("/proc/version") {
            let version = version.trim();
            if let Some(start) = version.find(" version ") {
                let after = &version[start + 9..];
                let end = after.find(' ').unwrap_or(after.len());
                return Some(after[..end].to_string());
            }
            return Some(version.to_string());
        }
    }

    if let Ok(output) = Command::new("uname").arg("-r").output()
        && output.status.success()
    {
        let version = decode_command_output(&output.stdout).trim().to_string();
        if !version.is_empty() {
            return Some(version);
        }
    }

    None
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
    fn test_detect_os_type() {
        let os_type = detect_os_type();
        assert_ne!(os_type, OsType::Unknown);
    }

    #[test]
    fn test_system_info_format() {
        let info = SystemInfo {
            os_type: OsType::Linux,
            kernel_version: Some("5.15.0-generic".to_string()),
            is_wsl: false,
            is_ssh: false,
            current_date: "2026-07-13".to_string(),
        };

        let env = info.format_env();
        assert!(env.contains("Platform: linux"));
        assert!(env.contains("Kernel: 5.15.0-generic"));
        assert!(env.contains("Today's date: 2026-07-13"));
        assert!(!env.contains("WSL"));
        assert!(!env.contains("SSH"));
    }
}
