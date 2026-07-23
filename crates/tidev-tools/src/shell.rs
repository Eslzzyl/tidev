//! Shell detection and selection for command execution.
//!
//! On Unix, always uses `sh -lc`.
//! On Windows, auto-detects `pwsh` (PowerShell 7+) first, falling back to
//! `powershell` (Windows PowerShell 5.1).  Users can override via
//! `config.shell.windows_shell` in `config.toml`.
//!
//! # Initialization
//!
//! Call [`init`] once at engine startup with the optional user override from
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
/// * `config_shell` – optional user override from `config.shell.windows_shell`.
#[cfg(windows)]
pub fn init(config_shell: Option<String>) {
    RESOLVED.get_or_init(|| resolve(config_shell));
}

/// Non-Windows: always `sh -lc`, ignores config.
#[cfg(not(windows))]
pub fn init(_config_shell: Option<String>) {
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
    RESOLVED
        .get()
        .expect("shell::init() must be called before shell::get()")
}

#[cfg(windows)]
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

/// Infer the shell argument (`-lc`, `-NoProfile -Command`, `/C`, …) from
/// the executable name the user provided.
#[cfg(windows)]
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
