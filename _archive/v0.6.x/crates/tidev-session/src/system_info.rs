//! System environment information detection for providing context to the LLM.

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
    /// Operating system type.
    pub os_type: OsType,
    /// Kernel version (if available).
    pub kernel_version: Option<String>,
    /// Whether running under WSL.
    pub is_wsl: bool,
    /// Whether running in an SSH session.
    pub is_ssh: bool,
    /// Current date in YYYY-MM-DD format.
    pub current_date: String,
}

impl SystemInfo {
    /// Detect the current system environment.
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

    /// Format the system info as an `<env>` block for inclusion in the system prompt.
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

/// Detect the operating system type.
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
        return OsType::Unknown;
    }
}

/// Detect the kernel version.
fn detect_kernel_version() -> Option<String> {
    // Try to read /proc/version on Linux
    #[cfg(target_os = "linux")]
    {
        if let Ok(version) = std::fs::read_to_string("/proc/version") {
            let version = version.trim();
            // Extract the version string after "VERSION"
            if let Some(start) = version.find(" version ") {
                let after = &version[start + 9..];
                let end = after.find(' ').unwrap_or(after.len());
                return Some(after[..end].to_string());
            }
            return Some(version.to_string());
        }
    }

    // Try uname on other platforms
    if let Ok(output) = Command::new("uname").arg("-r").output()
        && output.status.success()
    {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !version.is_empty() {
            return Some(version);
        }
    }

    None
}

/// Detect if running under WSL (Windows Subsystem for Linux).
fn detect_wsl() -> bool {
    // Check for WSL environment variable (set by WSL)
    if std::env::var("WSL_DISTRO_NAME").is_ok() {
        return true;
    }

    // Check for WSL-specific procfs indicator
    #[cfg(target_os = "linux")]
    {
        if let Ok(version) = std::fs::read_to_string("/proc/version") {
            let version = version.to_lowercase();
            return version.contains("microsoft") || version.contains("wsl");
        }
    }

    false
}

/// Detect if running in an SSH session.
fn detect_ssh() -> bool {
    // SSH sets these environment variables when connected
    std::env::var("SSH_CLIENT").is_ok()
        || std::env::var("SSH_CONNECTION").is_ok()
        || std::env::var("SSH_TTY").is_ok()
}

/// Check if the given directory is a git repository.
/// Uses `git status` as primary detection, falls back to `.git` directory existence.
pub fn is_git_repo(workspace_root: &std::path::Path) -> bool {
    // Primary: use `git status` to verify this is a valid git repository
    let output = std::process::Command::new("git")
        .current_dir(workspace_root)
        .args(["status"])
        .output();

    if let Ok(output) = output {
        return output.status.success();
    }

    // Fallback: check for `.git` directory existence
    std::path::Path::new(workspace_root).join(".git").exists()
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
            current_date: "2024-01-21".to_string(),
        };

        let env = info.format_env();
        assert!(env.contains("Platform: linux"));
        assert!(env.contains("Kernel: 5.15.0-generic"));
        assert!(env.contains("Today's date: 2024-01-21"));
        assert!(!env.contains("WSL"));
        assert!(!env.contains("SSH"));
    }

    #[test]
    fn test_wsl_detection() {
        // This test will pass or fail based on actual environment
        let _ = detect_wsl();
    }

    #[test]
    fn test_ssh_detection() {
        // This test will pass or fail based on actual environment
        let _ = detect_ssh();
    }
}
