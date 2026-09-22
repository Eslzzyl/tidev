//! External editor integration — detect editors, create temp files,
//! suspend/resume the TUI, and wait for the editor to finish.

use std::process::Command;
use std::{io, path::PathBuf};

use anyhow::Context;
use tidev_config::UiConfig;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Editor detection
// ---------------------------------------------------------------------------

/// Known GUI editors that need `--wait` (or equivalent) when spawned
/// via auto-detect, so the process blocks until the editor window is closed.
fn editor_wait_flag(name: &str) -> Option<&'static str> {
    let name = std::path::Path::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(name)
        .to_ascii_lowercase();
    match name.as_str() {
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
pub fn resolve_editor(ui_config: &UiConfig) -> Option<(String, Vec<String>)> {
    let cmd_str = ui_config
        .external_editor
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
        .or_else(|| {
            std::env::var("VISUAL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(|| {
            // Auto-detect: GUI editors first, then terminal editors.
            [
                "code",
                "cursor",
                "windsurf",
                "subl",
                "zed",
                "idea",
                "code-insiders",
                "vim",
                "nano",
                "nvim",
                "vi",
                "hx",
                "emacs",
            ]
            .iter()
            .find_map(|name| which::which(name).ok())
            .map(|path| path.to_string_lossy().into_owned())
        })?;

    let parts = shlex::split(&cmd_str)?;
    if parts.is_empty() {
        return None;
    }
    let mut cmd = parts[0].clone();
    let mut args: Vec<String> = parts[1..].to_vec();

    // Auto-detected editors (no args from the user) may need --wait so the
    // CLI process blocks until the editor window is closed. If the user
    // explicitly passed args, trust their choice.
    if args.is_empty()
        && let Some(flag) = editor_wait_flag(&cmd)
    {
        args.push(flag.to_string());
    }

    if let Ok(path) = which::which(&cmd) {
        cmd = path.to_string_lossy().into_owned();
    }

    Some((cmd, args))
}

// ---------------------------------------------------------------------------
// Temp edit file
// ---------------------------------------------------------------------------

/// Generate a unique temp file path for editing.
fn temp_edit_path() -> PathBuf {
    let tmp_dir = std::env::temp_dir();
    tmp_dir.join(format!("tidev-edit-{}.md", Uuid::new_v4()))
}

/// A temp file that is automatically removed when dropped.
pub struct TempEditFile {
    path: PathBuf,
}

impl TempEditFile {
    pub fn create(content: &str) -> std::io::Result<Self> {
        let path = temp_edit_path();
        std::fs::write(&path, content)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn read(&self) -> std::io::Result<String> {
        let bytes = std::fs::read(&self.path)?;
        tidev_utils::encoding::decode_text(&bytes)
            .map(|document| document.into_text())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

impl Drop for TempEditFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

// ---------------------------------------------------------------------------
// Suspend / resume helpers
// ---------------------------------------------------------------------------

/// Suspend the TUI: leave alternate screen, disable raw mode, show cursor, so
/// the editor can take over the terminal.
pub fn suspend_tui() -> anyhow::Result<()> {
    use crossterm::cursor::Show;
    use crossterm::event::{DisableBracketedPaste, DisableFocusChange, DisableMouseCapture};
    use crossterm::execute;
    use crossterm::terminal::{DisableLineWrap, LeaveAlternateScreen, disable_raw_mode};
    execute!(
        std::io::stdout(),
        LeaveAlternateScreen,
        DisableLineWrap,
        DisableBracketedPaste,
        DisableFocusChange,
        DisableMouseCapture,
        Show,
    )
    .context("failed to leave alternate screen")?;
    disable_raw_mode().context("failed to disable raw mode")?;
    Ok(())
}

/// Resume the TUI: enable raw mode, enter alternate screen, hide cursor.
pub fn resume_tui() -> anyhow::Result<()> {
    use crossterm::cursor::Hide;
    use crossterm::event::{EnableBracketedPaste, EnableFocusChange, EnableMouseCapture};
    use crossterm::execute;
    use crossterm::terminal::{EnableLineWrap, EnterAlternateScreen, enable_raw_mode};
    enable_raw_mode().context("failed to enable raw mode")?;
    execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        EnableLineWrap,
        EnableBracketedPaste,
        EnableFocusChange,
        EnableMouseCapture,
        Hide,
    )
    .context("failed to enter alternate screen")?;
    Ok(())
}

/// Open an external editor with the given text content.
/// Returns the edited text, or an error.
pub fn open_external_editor(text: &str, ui_config: &UiConfig) -> anyhow::Result<String> {
    let Some((cmd, mut args)) = resolve_editor(ui_config) else {
        anyhow::bail!("No editor found. Set external_editor in config, $VISUAL, or $EDITOR.");
    };

    let edit_file = TempEditFile::create(text).context("Failed to create temp file for editing")?;

    suspend_tui()?;

    args.push(edit_file.path().to_string_lossy().to_string());
    let status_result = Command::new(&cmd).args(&args).status();
    let resume_result = resume_tui();

    resume_result?;
    let status = status_result.with_context(|| format!("Failed to start editor '{cmd}'"))?;

    // Report editor exit status if it failed.
    if let Some(exit_code) = status.code()
        && exit_code != 0
    {
        log::warn!("Editor {cmd} exited with code {exit_code}");
    }

    let edited = edit_file.read().context("Failed to read edited file")?;
    Ok(edited)
}
