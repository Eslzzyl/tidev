use anyhow::Result;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::{prompts::SessionMode, session::ToolCall, storage::SessionStore};
use crate::skills::SkillCatalog;

use super::tools::{execute_tool_call, tool_definitions};
use super::{ToolDefinition, canonical_tool_name};

#[derive(Clone, Debug)]
pub struct ToolRegistry {
    workspace_root: PathBuf,
    max_output_bytes: usize,
    definitions: Vec<ToolDefinition>,
    skills: SkillCatalog,
}

impl ToolRegistry {
    pub fn new(workspace_root: PathBuf, config_dir: PathBuf, skill_sources: Vec<String>) -> Self {
        let skills = SkillCatalog::discover(&workspace_root, &config_dir, &skill_sources);
        let definitions = tool_definitions(skills.tool_description());

        Self {
            workspace_root,
            max_output_bytes: 12_000,
            definitions,
            skills,
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    pub fn permission_key_for_call(&self, call: &ToolCall) -> String {
        if call.name == "skill" {
            if let Ok(args) = serde_json::from_str::<crate::tooling::SkillArgs>(&call.arguments)
                && !args.name.trim().is_empty()
            {
                return SkillCatalog::permission_key_for_name(args.name.trim());
            }

            return SkillCatalog::permission_key_for_name("unknown");
        }

        canonical_tool_name(&call.name)
            .unwrap_or(&call.name)
            .to_string()
    }

    pub fn permission_label_for_call(&self, call: &ToolCall) -> String {
        if call.name == "skill" {
            if let Ok(args) = serde_json::from_str::<crate::tooling::SkillArgs>(&call.arguments)
                && !args.name.trim().is_empty()
            {
                return format!("skill '{}'", args.name.trim());
            }

            return "skill".to_string();
        }

        canonical_tool_name(&call.name)
            .unwrap_or(&call.name)
            .to_string()
    }

    pub fn available_definitions(&self, mode: SessionMode) -> Vec<ToolDefinition> {
        self.definitions
            .iter()
            .filter(|definition| definition.permission.is_allowed_in(mode))
            .cloned()
            .collect()
    }

    pub fn max_output_bytes(&self) -> usize {
        self.max_output_bytes
    }

    pub fn can_execute(&self, tool_name: &str, mode: SessionMode) -> bool {
        self.definition_for(tool_name)
            .is_some_and(|definition| definition.permission.is_allowed_in(mode))
    }

    pub fn definition_for(&self, tool_name: &str) -> Option<&ToolDefinition> {
        let canonical_name = super::canonical_tool_name(tool_name)?;
        self.definitions
            .iter()
            .find(|definition| definition.name == canonical_name)
    }

    pub fn execute_call(
        &self,
        store: &SessionStore,
        session_id: Uuid,
        call: &ToolCall,
    ) -> Result<String> {
        execute_tool_call(
            &self.workspace_root,
            &self.skills,
            store,
            session_id,
            call,
            self.max_output_bytes,
        )
    }
}
