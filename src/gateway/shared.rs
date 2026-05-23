use std::collections::HashMap;
use std::path::Path;

use crate::config::ConfigPaths;
use crate::prompts::SessionMode;
use crate::session::Message;

/// Compose the instruction prompt from config and instruction files.
/// Shared by all gateway channels.
pub fn compose_instruction_prompt(
    workspace_root: &Path,
    paths: &ConfigPaths,
    config: &crate::config::AppConfig,
) -> String {
    let (instruction_prompt, _) = crate::instructions::system_prompt_and_sources(
        workspace_root,
        &paths.config_dir,
        &config.instructions,
    )
    .unwrap_or_default();

    instruction_prompt
}

/// Target for model configuration in the interactive /model command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSelectionTarget {
    /// The main chat model.
    Chat,
    /// A specific agent type (explorer, oracle, etc.).
    Agent { agent_type: String },
    /// A memory module role (currently only "consolidation").
    Memory { role: String },
}

/// Return a human-readable display name for a target.
pub fn target_display_name(target: &ModelSelectionTarget) -> String {
    match target {
        ModelSelectionTarget::Chat => "Chat model".to_string(),
        ModelSelectionTarget::Agent { agent_type } => format!("Agent '{}'", agent_type),
        ModelSelectionTarget::Memory { role } => format!("Memory {}", role),
    }
}

/// Return the list of all configurable targets with (target_type_str, display_name).
///
/// target_type_str format: "chat", "agent:<type>", "memory:<role>"
pub fn all_target_entries() -> Vec<(&'static str, &'static str)> {
    vec![
        ("chat", "Chat model"),
        ("agent:explorer", "Explorer"),
        ("agent:librarian", "Librarian"),
        ("agent:oracle", "Oracle"),
        ("agent:designer", "Designer"),
        ("agent:fixer", "Fixer"),
        ("memory:consolidation", "Memory: Consolidation"),
    ]
}

/// Parse a target entry string back into a `ModelSelectionTarget`.
pub fn parse_target(entry: &str) -> Option<ModelSelectionTarget> {
    match entry {
        "chat" => Some(ModelSelectionTarget::Chat),
        "agent:explorer" => Some(ModelSelectionTarget::Agent { agent_type: "explorer".to_string() }),
        "agent:librarian" => Some(ModelSelectionTarget::Agent { agent_type: "librarian".to_string() }),
        "agent:oracle" => Some(ModelSelectionTarget::Agent { agent_type: "oracle".to_string() }),
        "agent:designer" => Some(ModelSelectionTarget::Agent { agent_type: "designer".to_string() }),
        "agent:fixer" => Some(ModelSelectionTarget::Agent { agent_type: "fixer".to_string() }),
        "memory:consolidation" => Some(ModelSelectionTarget::Memory { role: "consolidation".to_string() }),
        _ => None,
    }
}

/// Manages session mode (Plan/Build) tracking per gateway chat.
///
/// Each gateway chat (identified by a `chat_key`) has an independent mode
/// that can be toggled via `/plan` / `/build` commands.  New chats use the
/// configured `default_mode`.
#[derive(Debug, Clone)]
pub struct ModeManager {
    /// Per-chat session mode.  Chat keys are platform-specific
    /// (e.g. `"telegram:12345"`, `"qq:user:xxx"`).
    modes: HashMap<String, SessionMode>,
    /// Default mode for new / freshly-rotated chats.
    default_mode: SessionMode,
}

impl ModeManager {
    /// Create a new manager with the given default mode.
    pub fn new(default_mode: SessionMode) -> Self {
        Self {
            modes: HashMap::new(),
            default_mode,
        }
    }

    /// Get the current mode for `chat_key`, falling back to `default_mode`.
    pub fn get(&self, chat_key: &str) -> SessionMode {
        self.modes.get(chat_key).copied().unwrap_or(self.default_mode)
    }

    /// Set the mode for `chat_key`.
    pub fn set(&mut self, chat_key: &str, mode: SessionMode) {
        self.modes.insert(chat_key.to_string(), mode);
    }

    /// Reset `chat_key` back to the configured default mode.
    pub fn reset(&mut self, chat_key: &str) {
        self.modes.remove(chat_key);
    }

    /// Restore the mode for `chat_key` from a slice of conversation messages.
    ///
    /// Looks for the **most recent** user message that carries a `mode` tag
    /// and sets it as the current mode.  If none is found the mode is left
    /// at its existing value (default or previously set).
    pub fn restore_from_messages(&mut self, chat_key: &str, messages: &[Message]) {
        if let Some(last_user_with_mode) = messages
            .iter()
            .rev()
            .find(|m| m.role == crate::session::MessageRole::User && m.mode.is_some())
        {
            if let Some(mode) = last_user_with_mode.mode {
                self.set(chat_key, mode);
            }
        }
    }

    /// The display text shown when switching to a given mode.
    pub fn switch_message(mode: SessionMode) -> &'static str {
        match mode {
            SessionMode::Plan => "🔍 Switched to Plan mode (read-only).",
            SessionMode::Build => "🔧 Switched to Build mode (full tools).",
        }
    }
}
/// Returns empty vec if the model does not support thinking levels.
pub fn thinking_options_for_model_id(model_id: &str) -> Vec<&'static str> {
    let id = model_id.to_ascii_lowercase();
    if id.contains("deepseek") && id.contains("4") {
        vec!["deepseek:Off", "deepseek:High", "deepseek:Max"]
    } else if id.contains("qwen") && id.contains("3.") {
        vec!["qwen:Off", "qwen:On"]
    } else if id.contains("glm") {
        vec!["glm:Off", "glm:On"]
    } else {
        vec![]
    }
}
