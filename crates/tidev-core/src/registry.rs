//! Tool registry — wraps `tidev_tools` dispatch with tidev-core concerns.
//!
//! This module provides [`ToolRegistry`], the single entry point for tool
//! execution within tidev-core. It delegates to `tidev_tools::execute_tool_call`
//! / `execute_tool_call_streaming` while managing the `ToolContext` lifecycle
//! (workspace paths, skills catalog, web search config, etc.).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_types::message::{BackendEvent, ToolCall, ToolExecutionResult};
use tidev_types::prompts::SessionMode;
use tidev_types::tools::ToolDefinition;

use tidev_config::{AuthStore, WebSearchConfig};
use tidev_tools::execute_tool_call;
use tidev_tools::execute_tool_call_streaming;
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
        todo: Arc<dyn TodoPersistence + Send + Sync>,    web_search_config: WebSearchConfig,
    auth_store: AuthStore,
    max_output_bytes: usize,
}

impl ToolRegistry {
    pub fn new(
        workspace_root: PathBuf,
        config_dir: PathBuf,
        skills: SkillCatalog,
    todo: Arc<dyn TodoPersistence + Send + Sync>,        web_search_config: WebSearchConfig,
        auth_store: AuthStore,
        max_output_bytes: usize,
    ) -> Self {
        Self {
            workspace_root,
            config_dir,
            skills,
            todo,
            web_search_config,
            auth_store,
            max_output_bytes,
        }
    }

    /// Execute a streaming tool call with cooperative cancellation.
    ///
    /// Bash commands honor the `cancel` token — when cancelled, the
    /// process group is killed and partial output is returned. Other tools ignore it.
    /// When `event_tx` is `Some`, shell output is streamed as
    /// [`BackendEvent::ShellOutput`] events.
    pub fn execute_streaming(
        &self,
        call: &ToolCall,
        session_id: Uuid,
        mode: SessionMode,
        allow_outside: bool,
        sensitive_file_approved: bool,
        cancel: &CancellationToken,
        event_tx: Option<UnboundedSender<BackendEvent>>,
    ) -> Result<ToolExecutionResult> {
        let ctx = tidev_tools::ToolContext {
            workspace_root: &self.workspace_root,
            config_dir: &self.config_dir,
            skills: &self.skills,
            todo: &*self.todo,
            session_id,
            max_output_bytes: self.max_output_bytes,
            mode,
            allow_outside,
            sensitive_file_approved,
            web_search_config: &self.web_search_config,
            auth_store: &self.auth_store,
            event_tx,
        };
        execute_tool_call_streaming(&ctx, call, cancel)
    }

    /// Execute a non-streaming tool call (no cancellation, no event streaming).
    pub fn execute(
        &self,
        call: &ToolCall,
        session_id: Uuid,
        mode: SessionMode,
        allow_outside: bool,
        sensitive_file_approved: bool,
    ) -> Result<ToolExecutionResult> {
        let ctx = tidev_tools::ToolContext {
            workspace_root: &self.workspace_root,
            config_dir: &self.config_dir,
            skills: &self.skills,
            todo: &*self.todo,
            session_id,
            max_output_bytes: self.max_output_bytes,
            mode,
            allow_outside,
            sensitive_file_approved,
            web_search_config: &self.web_search_config,
            auth_store: &self.auth_store,
            event_tx: None,
        };
        execute_tool_call(&ctx, call)
    }

    /// Return all available tool definitions (excluding MCP).
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let skill_description = self.skills.tool_description();
        tidev_tools::tool_definitions(skill_description)
    }
}
