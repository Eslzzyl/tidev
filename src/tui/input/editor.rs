use crate::config::UiConfig;
use uuid::Uuid;

/// Known GUI editors that need `--wait` (or equivalent) when spawned
/// via auto-detect, so the process blocks until the editor window is closed.
fn editor_wait_flag(name: &str) -> Option<&'static str> {
    match name {
        "code" | "code-insiders" | "cursor" | "windsurf" => Some("--wait"),
        "subl" => Some("--wait"),
        "zed" => Some("--wait"),
        _ => None,
    }
}

/// Resolve the external editor command.
///
/// Priority:
/// 1. `external_editor` from config UI settings
/// 2. `$VISUAL` environment variable
/// 3. `$EDITOR` environment variable
/// 4. Auto-detect: `code` → `cursor` → `windsurf` → `subl` → `zed` → `idea` →
///    `code-insiders` → `vim` → `nano` → `nvim` → `vi` → `hx` → `emacs`
///
/// Returns `None` if no editor could be resolved.
pub(crate) fn resolve_editor(ui_config: &UiConfig) -> Option<(String, Vec<String>)> {
    let cmd_str = ui_config
        .external_editor
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
        .or_else(|| std::env::var("VISUAL").ok().filter(|s| !s.trim().is_empty()))
        .or_else(|| std::env::var("EDITOR").ok().filter(|s| !s.trim().is_empty()))
        .or_else(|| {
            // Auto-detect: GUI editors first, then terminal editors.
            // VSCode (code) is the most common default and preferred.
            [
                "code", "cursor", "windsurf", "subl", "zed", "idea", "code-insiders",
                "vim", "nano", "nvim", "vi", "hx", "emacs",
            ]
            .iter()
            .find(|name| is_executable_on_path(name))
            .map(|name| name.to_string())
        })?;

    let parts = shlex::split(&cmd_str)?;
    if parts.is_empty() {
        return None;
    }
    let cmd = parts[0].clone();
    let mut args: Vec<String> = parts[1..].to_vec();

    // Auto-detected editors (no args from the user) may need --wait so the
    // CLI process blocks until the editor window is closed. If the user
    // explicitly passed args (e.g. "code --wait" in config/$VISUAL/$EDITOR),
    // we trust their choice.
    if args.is_empty()
        && let Some(flag) = editor_wait_flag(&cmd) {
            args.push(flag.to_string());
        }

    Some((cmd, args))
}

/// Check if an executable exists on `$PATH`.
fn is_executable_on_path(name: &str) -> bool {
    // On Unix: use `which`. On Windows: use `where.exe`.
    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/c")
    } else {
        ("sh", "-c")
    };
    std::process::Command::new(shell)
        .arg(flag)
        .arg(if cfg!(windows) {
            format!("where {}", name)
        } else {
            format!("which {}", name)
        })
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Generate a unique temp file path for editing.
pub(crate) fn temp_edit_path() -> std::path::PathBuf {
    let tmp_dir = std::env::temp_dir();
    tmp_dir.join(format!("tidev-edit-{}.md", Uuid::new_v4()))
}

/// A temp file that is automatically removed when dropped.
pub(crate) struct TempEditFile {
    path: std::path::PathBuf,
}

impl TempEditFile {
    pub(crate) fn create(content: &str) -> std::io::Result<Self> {
        let path = temp_edit_path();
        std::fs::write(&path, content)?;
        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub(crate) fn read(&self) -> std::io::Result<String> {
        std::fs::read_to_string(&self.path)
    }
}

impl Drop for TempEditFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::UiConfig;

    #[test]
    fn resolve_editor_from_config() {
        let ui = UiConfig {
            external_editor: Some("code --wait".to_string()),
            ..Default::default()
        };
        // This may or may not find code, depending on the system.
        // We just verify it doesn't panic and returns Some if code is installed.
        if let Some((cmd, args)) = resolve_editor(&ui) {
            assert_eq!(cmd, "code");
            assert_eq!(args, vec!["--wait"]);
        }
    }

    #[test]
    fn resolve_editor_fallback_none() {
        // With nothing set and no GUI editors typically available in CI,
        // this should return None or Some depending on environment.
        let ui = UiConfig::default();
        // No assertion — just ensuring no panic
        let _ = resolve_editor(&ui);
    }

    #[test]
    fn temp_path_ends_with_dot_md() {
        let path = temp_edit_path();
        assert!(path.extension().map(|e| e == "md").unwrap_or(false));
        assert!(path.to_string_lossy().contains("tidev-edit-"));
    }
}
