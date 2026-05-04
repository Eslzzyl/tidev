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


