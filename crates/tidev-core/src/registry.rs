//! Tool registry — wraps `tidev_tools` dispatch with tidev-core concerns.
//!
//! This module provides [`ToolRegistry`], the single entry point for tool
//! execution within tidev-core. It delegates to `tidev_tools::execute_tool_call`
//! / `execute_tool_call_streaming` while managing the `ToolContext` lifecycle
//! (workspace paths, skills catalog, web search config, etc.).

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_types::message::{BackendEvent, ToolCall, ToolExecutionResult};
use tidev_types::prompts::SessionMode;
use tidev_types::tools::{PermissionConfig, ToolDefinition};

use tidev_config::auth::ActiveModel;
use tidev_config::{AuthStore, WebSearchConfig};
use tidev_tools::execute_tool_call;
use tidev_tools::{SkillCatalog, TodoPersistence};

/// Tool execution entry point for tidev-core.
///
/// Wraps `tidev_tools` dispatch, managing the shared configuration that each
/// tool invocation needs (workspace paths, skills, credentials, etc.).
#[derive(Clone)]
pub struct ToolRegistry {
    workspace_root: PathBuf,
    config_dir: PathBuf,
    skills: SkillCatalog,
    todo: Arc<dyn TodoPersistence + Send + Sync>,
    web_search_config: WebSearchConfig,
    auth_store: AuthStore,
    max_output_bytes: usize,
    permission_config: PermissionConfig,
}

impl ToolRegistry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace_root: PathBuf,
        config_dir: PathBuf,
        skills: SkillCatalog,
        todo: Arc<dyn TodoPersistence + Send + Sync>,
        web_search_config: WebSearchConfig,
        auth_store: AuthStore,
        max_output_bytes: usize,
        permission_config: PermissionConfig,
    ) -> Self {
        Self {
            workspace_root,
            config_dir,
            skills,
            todo,
            web_search_config,
            auth_store,
            max_output_bytes,
            permission_config,
        }
    }

    /// Execute a tool call with cooperative cancellation and optional streaming.
    ///
    /// Shell commands honor the `cancel` token — when cancelled, the
    /// process group is killed and partial output is returned. Other tools ignore it.
    /// When `event_tx` is `Some`, shell output is streamed as
    /// [`BackendEvent::ShellOutput`] events.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute(
        &self,
        call: &ToolCall,
        session_id: Uuid,
        mode: SessionMode,
        allow_outside: bool,
        sensitive_file_approved: bool,
        cancel: &CancellationToken,
        event_tx: Option<UnboundedSender<BackendEvent>>,
    ) -> ToolExecutionResult {
        let ctx = tidev_tools::ToolContext {
            workspace_root: &self.workspace_root,
            config_dir: &self.config_dir,
            skills: &self.skills,
            todo: self.todo.clone(),
            session_id,
            max_output_bytes: self.max_output_bytes,
            mode,
            allow_outside,
            sensitive_file_approved,
            web_search_config: &self.web_search_config,
            auth_store: &self.auth_store,
            event_tx,
        };
        execute_tool_call(&ctx, call, cancel).await
    }

    /// Return all available tool definitions (unfiltered).
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let skill_description = self.skills.tool_description();
        tidev_tools::tool_definitions(skill_description)
    }

    /// Return tool definitions filtered for the given model.
    ///
    /// GPT models (gpt-4o, gpt-4o-mini, gpt-5, etc.) receive `apply_patch` but
    /// not `write`/`edit`. All other models (Claude, DeepSeek, Gemini, GPT-4,
    /// any OSS model) receive `write`/`edit` but not `apply_patch`.
    pub fn definitions_for_model(&self, model: &ActiveModel) -> Vec<ToolDefinition> {
        let mut definitions = tidev_tools::tool_definitions(self.skills.tool_description());
        if model.use_apply_patch() {
            definitions.retain(|d| d.name != "edit" && d.name != "write");
        } else {
            definitions.retain(|d| d.name != "apply_patch");
        }
        definitions
    }

    /// Access the skill catalog.
    pub fn skills(&self) -> &SkillCatalog {
        &self.skills
    }

    // ── Tool lookup helpers (for TUI permission UI) ─────────────────────

    /// Look up a [`ToolDefinition`] by name (supports canonical name aliases).
    pub fn definition_for(&self, tool_name: &str) -> Option<ToolDefinition> {
        let definitions = self.definitions();
        // First try exact match.
        if let Some(def) = definitions.iter().find(|d| d.name == tool_name) {
            return Some(def.clone());
        }
        // Fall back to canonical name lookup.
        let canonical = tidev_types::tools::canonical_tool_name(tool_name)?;
        definitions.into_iter().find(|d| d.name == canonical)
    }

    /// Returns `true` if the tool exists and its permission level is allowed
    /// in the given session mode according to the user's permission config.
    pub fn can_execute(&self, tool_name: &str, mode: SessionMode) -> bool {
        self.definition_for(tool_name)
            .is_some_and(|def| self.permission_config.is_allowed(mode, def.permission))
    }

    /// Return a stable key for a tool call (used for permission memoization).
    pub fn permission_key_for_call(&self, call: &ToolCall) -> String {
        if call.name == "skill" {
            if let Ok(args) =
                serde_json::from_str::<tidev_types::tools::SkillArgs>(&call.arguments)
                && !args.name.trim().is_empty()
            {
                return SkillCatalog::permission_key_for_name(args.name.trim());
            }
            return SkillCatalog::permission_key_for_name("unknown");
        }

        self.definition_for(&call.name)
            .as_ref()
            .map(|def| def.permission_key())
            .unwrap_or_else(|| {
                tidev_types::tools::canonical_tool_name(&call.name)
                    .unwrap_or(&call.name)
                    .to_string()
            })
    }

    /// Return a human-readable label for a tool call (used for permission UI).
    pub fn permission_label_for_call(&self, call: &ToolCall) -> String {
        if call.name == "skill" {
            if let Ok(args) =
                serde_json::from_str::<tidev_types::tools::SkillArgs>(&call.arguments)
                && !args.name.trim().is_empty()
            {
                return format!("skill '{}'", args.name.trim());
            }
            return "skill".to_string();
        }

        self.definition_for(&call.name)
            .as_ref()
            .map(|def| def.permission_label())
            .unwrap_or_else(|| {
                tidev_types::tools::canonical_tool_name(&call.name)
                    .unwrap_or(&call.name)
                    .to_string()
            })
    }
}
