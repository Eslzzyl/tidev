use std::path::Path;

use crate::{
    config::ConfigPaths,
    prompts::SessionMode,
};

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

/// Compose the system prompt by combining base prompt, instruction prompt, and session mode reminder.
/// Shared by all gateway channels.
pub fn compose_system_prompt(base_system_prompt: &str, instruction_prompt: &str) -> String {
    let mut prompt = String::new();
    if !base_system_prompt.trim().is_empty() {
        prompt.push_str(base_system_prompt.trim());
    }

    if !instruction_prompt.trim().is_empty() {
        if !prompt.is_empty() {
            prompt.push_str("\n\n");
        }
        prompt.push_str(instruction_prompt.trim());
    }

    if !prompt.is_empty() {
        prompt.push_str("\n\n");
    }

    prompt.push_str(SessionMode::Build.reminder());
    prompt
}