//! Shell detection and selection for command execution.
//!
//! On macOS and other Unix systems, uses `sh -lc` by default (on macOS this
//! is bash running in POSIX mode).
//! On Linux, auto-detects `bash` first, falling back to `sh`, because
//! dash-based `sh` rejects common bashisms.
//! On Windows, auto-detects `pwsh` (PowerShell 7+) first, falling back to
//! `powershell` (Windows PowerShell 5.1).  Users can override the detection
//! on any platform via `config.shell.windows_shell` / `config.shell.unix_shell`
//! in `config.toml`.
//!
//! # Initialization
//!
//! Call [`init`] once at engine startup with the optional user overrides from
//! the `[shell]` config section.  The shell tool in `exec.rs` then reads the
//! resolved shell via [`get`].

use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// ResolvedShell
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Global cached result
// ---------------------------------------------------------------------------

static RESOLVED: OnceLock<ResolvedShell> = OnceLock::new();

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialise the shell resolver.
///
/// Must be called once at engine startup, after the config has been loaded.
///
/// * `windows_shell` – optional user override from `config.shell.windows_shell`.
/// * `unix_shell` – optional user override from `config.shell.unix_shell`.
#[cfg(windows)]
pub fn init(windows_shell: Option<String>, _unix_shell: Option<String>) {
    RESOLVED.get_or_init(|| resolve(windows_shell));
}

/// Linux: auto-detect `bash`, fall back to `sh`.
#[cfg(all(not(windows), target_os = "linux"))]
pub fn init(_windows_shell: Option<String>, unix_shell: Option<String>) {
    RESOLVED.get_or_init(|| resolve_linux(unix_shell));
}

/// macOS and other Unix: always `sh -lc` unless overridden by config.
#[cfg(all(not(windows), not(target_os = "linux")))]
pub fn init(_windows_shell: Option<String>, unix_shell: Option<String>) {
    RESOLVED.get_or_init(|| resolve_default(unix_shell));
}

/// Return the resolved shell configuration.
///
/// # Panics
///
/// Panics if [`init`] has not been called yet.
pub fn get() -> &'static ResolvedShell {
    RESOLVED
        .get()
        .expect("shell::init() must be called before shell::get()")
}

// ---------------------------------------------------------------------------
// Shared resolution helpers (non-Windows)
// ---------------------------------------------------------------------------

/// Build a `ResolvedShell` from a user-configured program path.
///
/// The argument and display name are inferred from the executable name.
#[cfg(not(windows))]
fn resolve_from_config(shell: String) -> ResolvedShell {
    let arg = infer_shell_arg(&shell);
    let display_name = classify_shell_display_name(&shell);
    ResolvedShell {
        program: shell,
        arg,
        display_name,
    }
}

// ---------------------------------------------------------------------------
// Resolution logic (Linux)
// ---------------------------------------------------------------------------

/// Look for `bash` on PATH.
#[cfg(target_os = "linux")]
fn find_bash() -> Option<std::path::PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let bash = dir.join("bash");
        if bash.is_file() {
            return Some(bash);
        }
    }

    // Also check common install locations for bash.
    for candidate in ["/bin/bash", "/usr/bin/bash"] {
        let bash = std::path::Path::new(candidate);
        if bash.is_file() {
            return Some(bash.to_path_buf());
        }
    }

    None
}

/// Resolve the shell to use on Linux.
///
/// Priority:
/// 1. User-configured value from `config.shell.unix_shell`.
/// 2. `bash` on PATH (handles bashisms that dash-based `sh` rejects).
/// 3. `sh` (POSIX fallback).
#[cfg(target_os = "linux")]
fn resolve_linux(config_shell: Option<String>) -> ResolvedShell {
    // 1. User-configured value (from config.toml) takes priority.
    if let Some(shell) = config_shell {
        return resolve_from_config(shell);
    }

    // 2. Try bash first.
    if let Some(bash_path) = find_bash() {
        let path_str = bash_path.to_string_lossy().to_string();
        log::info!("Auto-detected shell: Bash ({path_str})");
        log::info!("Set shell.unix_shell in config.toml to override.");
        return ResolvedShell {
            program: path_str.clone(),
            arg: "-lc".into(),
            display_name: format!("Bash ({path_str})"),
        };
    }

    // 3. Fall back to POSIX sh.
    log::info!("bash not found. Falling back to sh.");
    ResolvedShell {
        program: "sh".into(),
        arg: "-lc".into(),
        display_name: "sh".into(),
    }
}

// ---------------------------------------------------------------------------
// Resolution logic (macOS and other Unix)
// ---------------------------------------------------------------------------

/// Resolve the shell to use on macOS and other Unix systems.
///
/// Priority:
/// 1. User-configured value from `config.shell.unix_shell`.
/// 2. `sh` (on macOS this is bash running in POSIX mode).
#[cfg(all(not(windows), not(target_os = "linux")))]
fn resolve_default(config_shell: Option<String>) -> ResolvedShell {
    // 1. User-configured value (from config.toml) takes priority.
    if let Some(shell) = config_shell {
        return resolve_from_config(shell);
    }

    // 2. Default to POSIX sh.
    ResolvedShell {
        program: "sh".into(),
        arg: "-lc".into(),
        display_name: "sh".into(),
    }
}

// ---------------------------------------------------------------------------
// Resolution logic (Windows only)
// ---------------------------------------------------------------------------

/// Look for `pwsh.exe` on PATH.
#[cfg(windows)]
fn find_pwsh() -> Option<std::path::PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let pwsh = dir.join("pwsh.exe");
        if pwsh.is_file() {
            return Some(pwsh);
        }
    }

    // Also check common install location for pwsh
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".to_string());
    let candidate = std::path::Path::new(&pf)
        .join("PowerShell")
        .join("7")
        .join("pwsh.exe");
    if candidate.is_file() {
        return Some(candidate);
    }

    None
}

/// Resolve the shell to use on Windows.
///
/// Priority:
/// 1. User-configured value from `config.shell.windows_shell`.
/// 2. `pwsh` (PowerShell 7+) on PATH.
/// 3. `powershell` (Windows PowerShell 5.1).
#[cfg(windows)]
fn resolve(config_shell: Option<String>) -> ResolvedShell {
    // 1. User-configured value (from config.toml) takes priority.
    if let Some(shell) = config_shell {
        let arg = infer_shell_arg(&shell);
        let display_name = classify_shell_display_name(&shell);
        return ResolvedShell {
            program: shell,
            arg,
            display_name,
        };
    }

    // 2. Try pwsh (PowerShell 7+) first.
    if let Some(pwsh_path) = find_pwsh() {
        let path_str = pwsh_path.to_string_lossy().to_string();
        log::info!("Auto-detected shell: PowerShell 7+ ({path_str})");
        log::info!("Set shell.windows_shell in config.toml to override.");
        return ResolvedShell {
            program: path_str.clone(),
            arg: "-NoProfile -Command".into(),
            display_name: format!("PowerShell 7+ ({path_str})"),
        };
    }

    // 3. Fall back to Windows PowerShell 5.1.
    log::info!("PowerShell 7+ (pwsh) not found. Falling back to Windows PowerShell 5.1.");
    ResolvedShell {
        program: "powershell".into(),
        arg: "-NoProfile -Command".into(),
        display_name: "Windows PowerShell 5.1 (powershell)".into(),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Classify a shell executable for a human-readable display name.
fn classify_shell_display_name(program: &str) -> String {
    let name = std::path::Path::new(program)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    match name.to_lowercase().as_str() {
        "bash" | "sh" | "zsh" | "fish" | "dash" | "ksh" => format!("Bash ({program})"),
        "pwsh" => format!("PowerShell 7+ ({program})"),
        "powershell" => format!("Windows PowerShell 5.1 ({program})"),
        "nu" => format!("Nushell ({program})"),
        _ => format!("Custom shell ({program})"),
    }
}

/// Infer the shell argument (`-lc`, `-NoProfile -Command`, `/C`, …) from
/// the executable name the user provided.
fn infer_shell_arg(program: &str) -> String {
    let name = std::path::Path::new(program)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    match name.to_lowercase().as_str() {
        "bash" | "sh" | "zsh" | "fish" | "dash" | "ksh" => "-lc".into(),
        "powershell" | "pwsh" => "-NoProfile -Command".into(),
        "cmd" => "/C".into(),
        "nu" => "-c".into(),
        // Default to `-lc` since most custom shells are POSIX-like.
        _ => "-lc".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_shell_arg_known_shells() {
        assert_eq!(infer_shell_arg("bash"), "-lc");
        assert_eq!(infer_shell_arg("/bin/sh"), "-lc");
        assert_eq!(infer_shell_arg("zsh"), "-lc");
        assert_eq!(infer_shell_arg("powershell"), "-NoProfile -Command");
        assert_eq!(
            infer_shell_arg("C:/Program Files/PowerShell/7/pwsh.exe"),
            "-NoProfile -Command"
        );
        assert_eq!(infer_shell_arg("cmd.exe"), "/C");
        assert_eq!(infer_shell_arg("nu"), "-c");
        assert_eq!(infer_shell_arg("myshell"), "-lc");
    }

    #[test]
    fn classify_shell_display_name_known_shells() {
        assert_eq!(classify_shell_display_name("bash"), "Bash (bash)");
        assert_eq!(
            classify_shell_display_name("/opt/homebrew/bin/zsh"),
            "Bash (/opt/homebrew/bin/zsh)"
        );
        assert_eq!(classify_shell_display_name("pwsh"), "PowerShell 7+ (pwsh)");
        assert_eq!(
            classify_shell_display_name("powershell"),
            "Windows PowerShell 5.1 (powershell)"
        );
        assert_eq!(classify_shell_display_name("nu"), "Nushell (nu)");
        assert_eq!(classify_shell_display_name("fish"), "Bash (fish)");
    }
}
