use std::path::Path;

use crate::config::ConfigPaths;

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

/// Get thinking level option strings for a model ID.
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
