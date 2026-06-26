//! Shell detection and selection for command execution.
//!
//! On Unix, always uses `sh -lc`.
//! On Windows, detects the best available bash-like shell (Git Bash, MSYS2,
//! Cygwin, etc.) and uses it instead of PowerShell.  This ensures
//! LLM-generated POSIX commands work correctly on Windows.
//!
//! # Initialization
//!
//! Call [`init`] once at engine startup with the optional user override from
//! the `[shell]` config section and the config paths (for persisting
//! auto-detection results).  The bash tool in `exec.rs` then reads the
//! resolved shell via [`get`].

#[cfg(windows)]
use std::path::PathBuf;
use std::sync::OnceLock;

/// Describes the shell program and argument to use for command execution.
#[derive(Debug)]
pub struct ResolvedShell {
    /// Path or name of the executable (e.g. `"C:\\Program Files\\Git\\bin\\bash.exe"` or `"sh"`).
    pub program: String,
    /// Shell argument (e.g. `"-lc"` for bash, `"-NoProfile -Command"` for PowerShell).
    pub arg: String,
    /// Human-readable label (for logging / diagnostics).
    pub display_name: String,
}

static RESOLVED: OnceLock<ResolvedShell> = OnceLock::new();

/// Initialise the shell resolver.
///
/// Must be called once at engine startup, after the config has been loaded.
///
/// * `config_shell` – optional user override from `config.shell.windows_shell`.
/// * `paths` – config file paths, used to persist auto-detection result.
#[cfg(windows)]
pub fn init(config_shell: Option<String>, paths: Option<&tidev_config::ConfigPaths>) {
    RESOLVED.get_or_init(|| resolve(config_shell, paths));
}

#[cfg(not(windows))]
pub fn init(_config_shell: Option<String>, _paths: Option<&tidev_config::ConfigPaths>) {
    RESOLVED.get_or_init(|| ResolvedShell {
        program: "sh".into(),
        arg: "-lc".into(),
        display_name: "sh".into(),
    });
}

/// Return the resolved shell configuration.
///
/// # Panics
///
/// Panics if [`init`] has not been called yet.
pub fn get() -> &'static ResolvedShell {
    RESOLVED.get().expect("shell::init() has not been called")
}

/// Detect the best available shell on Windows.
#[cfg(windows)]
fn resolve(config_shell: Option<String>, _paths: Option<&tidev_config::ConfigPaths>) -> ResolvedShell {
    // User override from config takes precedence
    if let Some(path) = config_shell {
        let path = path.trim();
        if !path.is_empty() {
            let arg = shell_arg_for(path);
            return ResolvedShell {
                program: path.to_string(),
                arg,
                display_name: path.to_string(),
            };
        }
    }

    // Probe common Git for Windows install paths
    for probe in &[
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
        r"C:\ProgramData\scoop\apps\git\current\bin\bash.exe",
        r"C:\Users\*\scoop\apps\git\current\bin\bash.exe",
        r"C:\tools\msys64\usr\bin\bash.exe",
        r"C:\msys64\usr\bin\bash.exe",
        r"C:\cygwin64\bin\bash.exe",
        r"C:\cygwin\bin\bash.exe",
    ] {
        // Expand wildcards manually — simple file existence check
        let probe_path = std::path::Path::new(probe);
        if probe_path.exists() {
            log::info!("shell: auto-detected {}", probe);
            return ResolvedShell {
                program: probe.to_string(),
                arg: "-lc".into(),
                display_name: "bash (Git Bash)".into(),
            };
        }
    }

    // Also try `where bash` to find bash on PATH
    if let Ok(output) = std::process::Command::new("where").arg("bash").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout);
            let first_line = path.lines().next().unwrap_or("").trim().to_string();
            if !first_line.is_empty() && PathBuf::from(&first_line).exists() {
                log::info!("shell: auto-detected via PATH: {}", first_line);
                return ResolvedShell {
                    program: first_line,
                    arg: "-lc".into(),
                    display_name: "bash (PATH)".into(),
                };
            }
        }
    }

    // Fallback: PowerShell
    log::info!("shell: no bash found, falling back to PowerShell");
    ResolvedShell {
        program: "powershell.exe".into(),
        arg: "-NoProfile -Command".into(),
        display_name: "powershell".into(),
    }
}

/// Determine the shell argument based on the shell program name.
#[cfg(windows)]
fn shell_arg_for(shell_path: &str) -> String {
    let lower = shell_path.to_lowercase();
    if lower.contains("powershell") || lower.contains("pwsh") {
        "-NoProfile -Command".into()
    } else if lower.contains("cmd") {
        "/C".into()
    } else {
        "-lc".into()
    }
}

/// Persist the auto-detected shell path to `config.toml` so that
/// subsequent startups use it directly without re-detection.
#[cfg(windows)]
fn persist_to_config(paths: &tidev_config::ConfigPaths, shell_path: &str) {
    // Config persistence requires loading AppConfig from paths
    // For now, just log the detected path
    log::info!("shell: would persist auto-detected shell to config: {shell_path}");
}

/// Check whether a resolved shell is bash-like (POSIX-compatible).
pub fn is_bash_like(shell: &ResolvedShell) -> bool {
    let lower = shell.program.to_lowercase();
    // Git Bash, MSYS2, Cygwin bash, WSL, or any standard Unix shell
    // Must NOT be PowerShell (which also ends in "sh" but is not POSIX)
    if lower.contains("powershell") || lower.contains("pwsh") || lower.contains("cmd") {
        return false;
    }
    lower.contains("bash")
        || lower.contains("sh")
        || lower.contains("zsh")
        || lower.contains("fish")
        || lower.ends_with("sh")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unix_shell_is_sh() {
        #[cfg(not(windows))]
        {
            init(None, None);
            let shell = get();
            assert_eq!(shell.program, "sh");
            assert_eq!(shell.arg, "-lc");
        }
    }

    #[test]
    fn test_is_bash_like() {
        assert!(is_bash_like(&ResolvedShell {
            program: "/bin/bash".into(),
            arg: "-lc".into(),
            display_name: "bash".into(),
        }));
        assert!(is_bash_like(&ResolvedShell {
            program: "/bin/zsh".into(),
            arg: "-lc".into(),
            display_name: "zsh".into(),
        }));
        assert!(!is_bash_like(&ResolvedShell {
            program: "powershell.exe".into(),
            arg: "-NoProfile -Command".into(),
            display_name: "powershell".into(),
        }));
    }
}
