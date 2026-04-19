use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

use crate::mcp::McpManager;
use crate::tooling::SkillCatalog;
use crate::{
    config::PermissionConfig,
    prompts::SessionMode,
    session::{ToolCall, ToolExecutionResult},
    storage::SessionStore,
};

use super::tools::{execute_tool_call, tool_definitions};
use super::{FileReadTracker, ToolDefinition, canonical_tool_name};

#[derive(Clone, Debug)]
pub struct ToolRegistry {
    workspace_root: PathBuf,
    config_dir: PathBuf,
    max_output_bytes: usize,
    definitions: Vec<ToolDefinition>,
    skills: SkillCatalog,
    mcp: McpManager,
    permission_config: PermissionConfig,
    file_read_tracker: Arc<FileReadTracker>,
}

impl ToolRegistry {
    pub fn new(
        workspace_root: PathBuf,
        config_dir: PathBuf,
        skill_sources: Vec<String>,
        mcp: McpManager,
        permission_config: PermissionConfig,
        file_read_tracker: Arc<FileReadTracker>,
    ) -> Self {
        let skills = SkillCatalog::discover(&workspace_root, &config_dir, &skill_sources);
        let definitions = tool_definitions(skills.tool_description());

        Self {
            workspace_root,
            config_dir,
            max_output_bytes: 12_000,
            definitions,
            skills,
            mcp,
            permission_config,
            file_read_tracker,
        }
    }

    pub fn file_read_tracker(&self) -> Arc<FileReadTracker> {
        self.file_read_tracker.clone()
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    pub fn mcp_summaries(&self) -> Vec<crate::mcp::McpServerSummary> {
        self.mcp.summaries()
    }

    pub fn mcp_manager(&self) -> McpManager {
        self.mcp.clone()
    }

    pub async fn refresh_mcp_tools(&self) -> Result<()> {
        self.mcp.refresh_all().await
    }

    pub async fn refresh_mcp_server(&self, name: &str) -> Result<()> {
        self.mcp.refresh_server(name).await
    }

    pub async fn toggle_mcp_server(&self, name: &str) -> Result<()> {
        self.mcp.toggle_server(name).await
    }

    pub async fn disconnect_mcp_server(&self, name: &str) -> Result<()> {
        self.mcp.disconnect_server(name).await
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

        self.definition_for(&call.name)
            .map(|definition| definition.permission_key())
            .unwrap_or_else(|| {
                canonical_tool_name(&call.name)
                    .unwrap_or(&call.name)
                    .to_string()
            })
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

        self.definition_for(&call.name)
            .map(|definition| definition.permission_label())
            .unwrap_or_else(|| {
                canonical_tool_name(&call.name)
                    .unwrap_or(&call.name)
                    .to_string()
            })
    }

    pub fn available_definitions(&self, mode: SessionMode) -> Vec<ToolDefinition> {
        let mut definitions = self
            .definitions
            .iter()
            .filter(|definition| {
                definition
                    .permission
                    .is_allowed_in(mode, &self.permission_config)
            })
            .cloned()
            .collect::<Vec<_>>();

        definitions.extend(
            self.mcp
                .available_definitions(mode, &self.permission_config),
        );
        definitions
    }

    /// Returns all tool definitions (unfiltered), used for LLM requests.
    /// In plan mode, the LLM can see all tools, but execution will be blocked.
    pub fn all_definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = self.definitions.clone();
        definitions.extend(self.mcp.all_definitions());
        definitions
    }

    pub fn max_output_bytes(&self) -> usize {
        self.max_output_bytes
    }

    pub fn can_execute(&self, tool_name: &str, mode: SessionMode) -> bool {
        self.definition_for(tool_name).is_some_and(|definition| {
            definition
                .permission
                .is_allowed_in(mode, &self.permission_config)
        })
    }

    pub fn definition_for(&self, tool_name: &str) -> Option<ToolDefinition> {
        if let Some(definition) = self
            .definitions
            .iter()
            .find(|definition| definition.name == tool_name)
        {
            return Some(definition.clone());
        }

        if let Some(definition) = self.mcp.definition_for(tool_name) {
            return Some(definition);
        }

        let canonical_name = super::canonical_tool_name(tool_name)?;
        self.definitions
            .iter()
            .find(|definition| definition.name == canonical_name)
            .cloned()
    }

    pub fn execute_call(
        &self,
        runtime: &tokio::runtime::Handle,
        store: &SessionStore,
        session_id: Uuid,
        call: &ToolCall,
    ) -> Result<ToolExecutionResult> {
        if self.mcp.definition_for(&call.name).is_some() {
            return runtime.block_on(self.mcp.execute_call(call));
        }

        execute_tool_call(
            &self.workspace_root,
            &self.config_dir,
            &self.skills,
            &self.file_read_tracker,
            store,
            session_id,
            call,
            self.max_output_bytes,
        )
    }
}
