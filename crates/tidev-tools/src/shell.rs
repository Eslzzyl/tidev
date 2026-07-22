//! Shell detection and selection for command execution.
//!
//! On Unix, always uses `sh -lc`.
//! On Windows, defaults to PowerShell.  Users can override via
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
        "powershell" | "pwsh" => format!("PowerShell ({program})"),
        "nu" => format!("Nushell ({program})"),
        _ => format!("Custom shell ({program})"),
    }
}

// ---------------------------------------------------------------------------
// Resolution logic (Windows only)
// ---------------------------------------------------------------------------

/// Resolve the shell to use on Windows.
///
/// Priority:
/// 1. User-configured value from `config.shell.windows_shell`.
/// 2. Default to PowerShell.
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

    // 2. Default to PowerShell.
    eprintln!("ℹ️  Defaulting to PowerShell for shell tool.");
    eprintln!("   Set shell.windows_shell in config.toml to override.");
    ResolvedShell {
        program: "powershell".into(),
        arg: "-NoProfile -Command".into(),
        display_name: "PowerShell".into(),
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
