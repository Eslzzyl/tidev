use serde::{Deserialize, Serialize};

/// Top-level hook system configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HooksConfig {
    /// When true, all hooks are disabled.
    #[serde(default)]
    pub disable_all_hooks: bool,

    /// Hooks that run after a tool completes.
    #[serde(default)]
    pub post_tool_use: Vec<PostToolUseHookConfig>,
}

/// A single PostToolUse hook definition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PostToolUseHookConfig {
    /// Matcher pattern for tool names (pipe-separated for OR).
    ///
    /// Examples:
    /// - `"write|edit|apply_patch"` — run after file-modifying tools
    /// - `"bash"` — run after shell commands
    /// - `"*"` — run after every tool
    pub matcher: String,

    /// Shell command to execute.
    ///
    /// Supports these template variables:
    /// - `{filepath}` — absolute path to the file that was modified
    /// - `{workspace_root}` — absolute path to workspace root
    /// - `{tool_name}` — name of the tool that ran
    pub command: String,

    /// Optional: only run for these file extensions (e.g. `[".py", ".rs"]`).
    /// When non-empty, the hook only runs if `result.metadata.filepath`
    /// has a matching extension.
    #[serde(default)]
    pub extensions: Vec<String>,

    /// Timeout in seconds (default: 30).
    #[serde(default = "default_timeout_sec")]
    pub timeout_sec: u64,

    /// Status message shown in the TUI while the hook runs.
    #[serde(default)]
    pub status_message: String,

    /// Optional: working directory for the command.
    /// Defaults to the workspace root.
    #[serde(default)]
    pub cwd: Option<String>,

    /// Optional: human-readable label for this hook.
    #[serde(default)]
    pub name: Option<String>,
}

fn default_timeout_sec() -> u64 {
    30
}

impl PostToolUseHookConfig {
    /// Returns the display name for this hook (the `name` field, or a short
    /// form of the command).
    pub fn display_name(&self) -> &str {
        self.name
            .as_deref()
            .or_else(|| self.status_message.split(':').next())
            .unwrap_or(&self.command)
    }
}
