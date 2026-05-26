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
/// * `paths` – config file paths, used to persist auto-detection result.
#[cfg(windows)]
pub fn init(config_shell: Option<String>, paths: Option<&crate::config::ConfigPaths>) {
    RESOLVED.get_or_init(|| resolve(config_shell, paths));
}

/// Non-Windows: always `sh -lc`, ignores config.
#[cfg(not(windows))]
pub fn init(_config_shell: Option<String>, _paths: Option<&crate::config::ConfigPaths>) {
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
    RESOLVED.get().expect("shell::init() must be called before shell::get()")
}

// ---------------------------------------------------------------------------
// Resolution logic (Windows only)
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn resolve(config_shell: Option<String>, paths: Option<&crate::config::ConfigPaths>) -> ResolvedShell {
    // 1. User-configured value (from config.toml) takes priority.
    if let Some(shell) = config_shell {
        let arg = infer_shell_arg(&shell);
        return ResolvedShell {
            program: shell,
            arg,
            display_name: format!("custom shell"),
        };
    }

    // 2. Auto-detect: find a non-WSL bash on PATH.
    if let Some(bash_path) = find_bash_on_path() {
        let path_str = bash_path.to_string_lossy().to_string();
        let display = format!("Bash ({path_str})");

        // Persist to config so the same value is used on next startup.
        if let Some(p) = paths {
            persist_to_config(p, &path_str);
        }

        eprintln!("ℹ️  Auto-detected shell: {display}");
        eprintln!("   Set shell.windows_shell in config.toml to override.");

        return ResolvedShell {
            program: path_str,
            arg: "-lc".into(),
            display_name: display,
        };
    }

    // 3. No bash found – fall back to PowerShell.
    eprintln!("ℹ️  No bash found on PATH, using PowerShell for shell tool.");
    eprintln!("   Install Git for Windows (https://git-scm.com) for better POSIX support.");
    ResolvedShell {
        program: "powershell".into(),
        arg: "-NoProfile -Command".into(),
        display_name: "PowerShell".into(),
    }
}

/// Walk `PATH` looking for a `bash.exe` that is **not** the WSL one
/// (which lives in `%SystemRoot%\System32`).
#[cfg(windows)]
fn find_bash_on_path() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let bash = dir.join("bash.exe");
            if bash.is_file() {
                let s = bash.to_string_lossy().to_lowercase();
                // WSL bash is always under System32; exclude it.
                if !s.contains("system32") && !s.contains(r"windows\system") {
                    Some(bash)
                } else {
                    None
                }
            } else {
                None
            }
        })
    })
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
        // Default to `-lc` since most custom shells are POSIX-like.
        _ => "-lc".into(),
    }
}

/// Persist the auto-detected shell path to `config.toml` so that
/// subsequent startups use it directly without re-detection.
#[cfg(windows)]
fn persist_to_config(paths: &crate::config::ConfigPaths, shell_path: &str) {
    match crate::config::AppConfig::load_or_create(paths) {
        Ok(mut config) => {
            config.shell.windows_shell = Some(shell_path.to_string());
            if let Err(e) = config.save(paths) {
                log::warn!("shell: failed to persist auto-detected shell: {e}");
            } else {
                log::info!("shell: persisted auto-detected shell to config");
            }
        }
        Err(e) => {
            log::warn!("shell: failed to load config for persistence: {e}");
        }
    }
}
