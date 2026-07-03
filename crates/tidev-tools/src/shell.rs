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
pub fn init(config_shell: Option<String>, paths: Option<&tidev_config::paths::ConfigPaths>) {
    RESOLVED.get_or_init(|| resolve(config_shell, paths));
}

/// Non-Windows: always `sh -lc`, ignores config.
#[cfg(not(windows))]
pub fn init(_config_shell: Option<String>, _paths: Option<&tidev_config::paths::ConfigPaths>) {
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

/// Return `true` if the resolved shell is a known bash-like shell.
pub fn is_bash_like(shell: &ResolvedShell) -> bool {
    let name = std::path::Path::new(&shell.program)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    matches!(
        name.as_str(),
        "bash" | "sh" | "zsh" | "fish" | "dash" | "ksh"
    ) || shell.arg == "-lc"
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
        _ => format!("Custom shell ({program})"),
    }
}

// ---------------------------------------------------------------------------
// Resolution logic (Windows only)
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn resolve(
    config_shell: Option<String>,
    paths: Option<&tidev_config::paths::ConfigPaths>,
) -> ResolvedShell {
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

/// Walk `PATH` and common install directories for a `bash.exe` that is
/// **not** the WSL one.
///
/// WSL ships two `bash.exe` shims that we must exclude:
/// - `%SystemRoot%\System32\bash.exe`
/// - `%LOCALAPPDATA%\Microsoft\WindowsApps\bash.exe`  (Windows 10+ shim)
#[cfg(windows)]
fn find_bash_on_path() -> Option<PathBuf> {
    /// Return `true` if the path points to a real bash (not a WSL shim).
    fn is_real_bash(path: &std::path::Path) -> bool {
        if !path.is_file() {
            return false;
        }
        let s = path.to_string_lossy().to_lowercase();
        // WSL bash shims live under System32 or WindowsApps; exclude both.
        !s.contains("system32") && !s.contains(r"windows\system") && !s.contains("windowsapps")
    }

    // 1. Search PATH directories.
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let bash = dir.join("bash.exe");
            if is_real_bash(&bash) {
                return Some(bash);
            }
        }
    }

    // 2. Search common install directories (Git Bash, MSYS2, Cygwin).
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".to_string());
    let pf86 = std::env::var("ProgramFiles(x86)")
        .unwrap_or_else(|_| "C:\\Program Files (x86)".to_string());

    let candidates = [
        format!("{pf}\\Git\\bin"),
        format!("{pf}\\Git\\usr\\bin"),
        format!("{pf86}\\Git\\bin"),
        format!("{pf86}\\Git\\usr\\bin"),
        "C:\\msys64\\usr\\bin".into(),
        "C:\\tools\\msys64\\usr\\bin".into(),
        "C:\\cygwin64\\bin".into(),
        "C:\\cygwin\\bin".into(),
    ];

    for dir in &candidates {
        let bash = std::path::Path::new(dir).join("bash.exe");
        if is_real_bash(&bash) {
            return Some(bash);
        }
    }

    None
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
fn persist_to_config(paths: &tidev_config::paths::ConfigPaths, shell_path: &str) {
    match tidev_config::AppConfig::load(paths) {
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
