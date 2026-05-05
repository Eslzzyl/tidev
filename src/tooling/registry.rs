use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::mcp::McpManager;
use crate::memory::types::MemoryStore;
use crate::session::BackendEvent;
use crate::tooling::SkillCatalog;
use crate::{
    config::PermissionConfig,
    prompts::SessionMode,
    session::{ToolCall, ToolExecutionResult},
    storage::SessionStore,
};

use super::tools::tool_definitions;
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
    memory_store: Arc<MemoryStore>,
    active_model: Option<crate::config::ActiveModel>,
    rtk_enabled: bool,
}

impl ToolRegistry {
    pub fn new(
        workspace_root: PathBuf,
        config_dir: PathBuf,
        skill_sources: Vec<String>,
        mcp: McpManager,
        permission_config: PermissionConfig,
        file_read_tracker: Arc<FileReadTracker>,
        memory_store: Arc<MemoryStore>,
        rtk_enabled: bool,
        worktree: Option<PathBuf>,
    ) -> Self {
        let skills = SkillCatalog::discover(
            &workspace_root,
            &config_dir,
            &skill_sources,
            worktree.as_deref(),
        );
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
            memory_store,
            active_model: None,
            rtk_enabled,
        }
    }

    pub fn set_active_model(&mut self, model: crate::config::ActiveModel) {
        self.active_model = Some(model);
    }

    pub fn model_supports_images(&self) -> bool {
        self.active_model
            .as_ref()
            .map(|m| m.supports_images)
            .unwrap_or(false)
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

    pub fn skills(&self) -> &SkillCatalog {
        &self.skills
    }

    pub fn mcp_summaries(&self) -> Vec<crate::mcp::McpServerSummary> {
        self.mcp.summaries()
    }

    pub fn memory_store(&self) -> Arc<crate::memory::types::MemoryStore> {
        self.memory_store.clone()
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

    /// Returns all tool definitions filtered by the active model.
    ///
    /// GPT models (`gpt-4o`, `gpt-4o-mini`, etc.) get `apply_patch` but not
    /// `edit`/`write`. All other models (Claude, DeepSeek, etc.) get `edit`/`write`
    /// but not `apply_patch`.  This matches opencode's tool-per-model logic.
    /// If no model has been set yet, all tools are returned unchanged.
    pub fn all_definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = self.definitions.clone();

        if let Some(ref model) = self.active_model {
            if model.use_apply_patch() {
                definitions.retain(|d| d.name != "edit" && d.name != "write");
            } else {
                definitions.retain(|d| d.name != "apply_patch");
            }
        }

        definitions.extend(self.mcp.all_definitions());
        definitions
    }

    pub fn max_output_bytes(&self) -> usize {
        self.max_output_bytes
    }

    pub fn rtk_enabled(&self) -> bool {
        self.rtk_enabled
    }

    pub fn can_execute(&self, tool_name: &str, mode: SessionMode) -> bool {
        self.definition_for(tool_name).is_some_and(|definition| {
            definition
                .permission
                .is_allowed_in(mode, &self.permission_config)
        })
    }

    /// Returns `true` if the tool call is read-only and can safely be
    /// executed in parallel with other read-only calls.
    ///
    /// Tools with `Read` or `Search` permission are inherently read-only.
    /// All other tools (`Write`, `Edit`, `Execute`, `Session`) mutate state
    /// and must be executed serially to prevent conflicts.
    pub fn is_read_only_call(&self, call: &ToolCall) -> bool {
        self.definition_for(&call.name).is_some_and(|def| {
            matches!(
                def.permission,
                super::ToolPermission::Read | super::ToolPermission::Search
            )
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
        session_id: uuid::Uuid,
        call: &ToolCall,
        mode: SessionMode,
        allow_outside: bool,
    ) -> Result<ToolExecutionResult> {
        if self.mcp.definition_for(&call.name).is_some() {
            return runtime.block_on(self.mcp.execute_call(call));
        }

        let mut result = super::builtin::execute_tool_call(
            &self.workspace_root,
            &self.config_dir,
            &self.skills,
            store,
            session_id,
            call,
            self.max_output_bytes,
            self.rtk_enabled,
            &self.memory_store,
            mode,
            allow_outside,
        )?;

        // Image capability check: If the result contains images but the model doesn't support them,
        // we strip the image attachments and return a message indicating the limitation.
        if !self.model_supports_images() && !result.attachments.is_empty() {
            let had_images = result
                .attachments
                .iter()
                .any(|a| matches!(a, crate::session::MessageAttachment::Image { .. }));
            if had_images {
                result
                    .attachments
                    .retain(|a| !matches!(a, crate::session::MessageAttachment::Image { .. }));
                result.output.push_str("\n\n(Note: Image reading was attempted, but the current model does not support image input. Images have been removed from the request.)");
            }
        }

        // For "read" tool, record the read if it was successful and didn't result in an attachment-only error
        if super::canonical_tool_name(&call.name) == Some("read") {
            let arguments: serde_json::Value = serde_json::from_str(&call.arguments)?;
            if let Some(path_str) = arguments.get("path").and_then(|v| v.as_str()) {
                let absolute_path = super::builtin::utils::resolve_workspace_path(
                    &self.workspace_root,
                    std::path::Path::new(path_str),
                    allow_outside,
                )?;
                if absolute_path.exists() && absolute_path.is_file() {
                    self.file_read_tracker
                        .record_read(store, session_id, &absolute_path)?;
                }
            }
        }

        Ok(result)
    }

    /// Execute a tool call on a blocking thread with panic protection.
    ///
    /// Clones all inputs (ToolRegistry is cheap to clone, SessionStore::clone
    /// opens a new SQLite connection), spawns a blocking tokio task, wraps the
    /// execution in `catch_unwind`, and returns a `JoinHandle`.
    ///
    /// Never panics — caught panics become a `ToolExecutionResult` with an
    /// error message.
    pub fn execute_call_spawned(
        &self,
        runtime_handle: tokio::runtime::Handle,
        store: SessionStore,
        session_id: uuid::Uuid,
        call: ToolCall,
        mode: SessionMode,
        allow_outside: bool,
    ) -> JoinHandle<ToolExecutionResult> {
        let registry = self.clone();
        tokio::task::spawn_blocking(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                registry.execute_call(
                    &runtime_handle,
                    &store,
                    session_id,
                    &call,
                    mode,
                    allow_outside,
                )
            }));
            match result {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => ToolExecutionResult::new(format!("Error: {e}")),
                Err(panic) => {
                    let msg = if let Some(s) = panic.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = panic.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        "unknown panic".to_string()
                    };
                    ToolExecutionResult::new(format!("Tool panicked: {msg}"))
                }
            }
        })
    }

    /// Execute a tool call on a blocking thread with streaming output events.
    ///
    /// Like [`execute_call_spawned`], but the bash tool will emit
    /// [`BackendEvent::ShellOutput`] events as output is produced.
    /// Other tools ignore the sender and execute normally.
    pub fn execute_call_spawned_streaming(
        &self,
        runtime_handle: tokio::runtime::Handle,
        store: SessionStore,
        session_id: uuid::Uuid,
        call: ToolCall,
        mode: SessionMode,
        allow_outside: bool,
        event_tx: UnboundedSender<BackendEvent>,
    ) -> JoinHandle<ToolExecutionResult> {
        let registry = self.clone();
        tokio::task::spawn_blocking(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    registry.execute_call_with_stream(
                        &runtime_handle,
                        &store,
                        session_id,
                        &call,
                        mode,
                        allow_outside,
                        event_tx,
                    )
                }));
            match result {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => ToolExecutionResult::new(format!("Error: {e}")),
                Err(panic) => {
                    let msg = if let Some(s) = panic.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = panic.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        "unknown panic".to_string()
                    };
                    ToolExecutionResult::new(format!("Tool panicked: {msg}"))
                }
            }
        })
    }

    /// Like [`execute_call`] but threads a streaming event sender to the bash tool.
    fn execute_call_with_stream(
        &self,
        runtime: &tokio::runtime::Handle,
        store: &SessionStore,
        session_id: uuid::Uuid,
        call: &ToolCall,
        mode: SessionMode,
        allow_outside: bool,
        event_tx: UnboundedSender<BackendEvent>,
    ) -> Result<ToolExecutionResult> {
        if self.mcp.definition_for(&call.name).is_some() {
            return runtime.block_on(self.mcp.execute_call(call));
        }

        let mut result = super::builtin::execute_tool_call_streaming(
            &self.workspace_root,
            &self.config_dir,
            &self.skills,
            store,
            session_id,
            call,
            self.max_output_bytes,
            self.rtk_enabled,
            &self.memory_store,
            mode,
            allow_outside,
            Some(event_tx),
        )?;

        // Image capability check
        if !self.model_supports_images() && !result.attachments.is_empty() {
            let had_images = result
                .attachments
                .iter()
                .any(|a| matches!(a, crate::session::MessageAttachment::Image { .. }));
            if had_images {
                result
                    .attachments
                    .retain(|a| !matches!(a, crate::session::MessageAttachment::Image { .. }));
                result.output.push_str("\n\n(Note: Image reading was attempted, but the current model does not support image input. Images have been removed from the request.)");
            }
        }

        // For "read" tool, record the read if it was successful
        if super::canonical_tool_name(&call.name) == Some("read") {
            let arguments: serde_json::Value = serde_json::from_str(&call.arguments)?;
            if let Some(path_str) = arguments.get("path").and_then(|v| v.as_str()) {
                let absolute_path = super::builtin::utils::resolve_workspace_path(
                    &self.workspace_root,
                    std::path::Path::new(path_str),
                    allow_outside,
                )?;
                if absolute_path.exists() && absolute_path.is_file() {
                    self.file_read_tracker
                        .record_read(store, session_id, &absolute_path)?;
                }
            }
        }

        Ok(result)
    }
}
