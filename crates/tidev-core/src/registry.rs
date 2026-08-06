//! Tool registry — wraps `tidev_tools` dispatch with tidev-core concerns.
//!
//! This module provides [`ToolRegistry`], the single entry point for tool
//! execution within tidev-core. It delegates to `tidev_tools::execute_tool_call`
//! / `execute_tool_call_streaming` while managing the `ToolContext` lifecycle
//! (workspace paths, skills catalog, web search config, etc.).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_agent::AgentEventSender;
use tidev_llm::message::{ToolCall, ToolExecutionResult};
use tidev_tools::types::ToolDefinition;

use tidev_config::auth::ActiveModel;
use tidev_config::{AuthStore, WebSearchConfig};
use tidev_tools::execute_tool_call;
use tidev_tools::{ShellOutput, SkillCatalog, TodoPersistence};

use crate::mcp::{McpManager, McpServerSummary};
use crate::mode::Mode;
use crate::tool_adapter::execute_builtin_via_agent;

/// Tool execution entry point for tidev-core.
///
/// Wraps `tidev_tools` dispatch, managing the shared configuration that each
/// tool invocation needs (workspace paths, skills, credentials, etc.).
/// Also owns the [`McpManager`] for MCP-backed tools.
#[derive(Clone)]
pub struct ToolRegistry {
    workspace_root: PathBuf,
    config_dir: PathBuf,
    skills: SkillCatalog,
    todo: Arc<dyn TodoPersistence + Send + Sync>,
    web_search_config: WebSearchConfig,
    auth_store: AuthStore,
    max_output_bytes: usize,
    mcp: McpManager,
    pending_instruction_sources: Arc<Mutex<HashMap<Uuid, Vec<String>>>>,
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
        mcp: McpManager,
    ) -> Self {
        Self {
            workspace_root,
            config_dir,
            skills,
            todo,
            web_search_config,
            auth_store,
            max_output_bytes,
            mcp,
            pending_instruction_sources: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Execute a tool call with cooperative cancellation and optional streaming.
    ///
    /// Shell commands honor the `cancel` token — when cancelled, the
    /// process group is killed and partial output is returned. Other tools ignore it.
    /// When `event_tx` is `Some`, shell output is streamed as
    /// [`ShellOutput`] events.
    ///
    /// MCP-backed tools are dispatched directly to the [`McpManager`].
    #[allow(clippy::too_many_arguments)]
    pub async fn execute(
        &self,
        call: &ToolCall,
        session_id: Uuid,
        request_id: u64,
        mode: Mode,
        allow_outside: bool,
        sensitive_file_approved: bool,
        cancel: &CancellationToken,
        event_tx: Option<UnboundedSender<ShellOutput>>,
    ) -> ToolExecutionResult {
        // MCP tool dispatch.
        if self.mcp.definition_for(&call.name).is_some() {
            match self.mcp.execute_call(call).await {
                Ok(result) => return result,
                Err(e) => {
                    return ToolExecutionResult::new(format!("Error: MCP tool call failed: {e:#}"));
                }
            }
        }

        // Built-in tool dispatch.
        let ctx = tidev_tools::ToolContext {
            workspace_root: &self.workspace_root,
            config_dir: &self.config_dir,
            skills: &self.skills,
            todo: self.todo.clone(),
            session_id,
            request_id,
            max_output_bytes: self.max_output_bytes,
            read_only: mode == Mode::Plan,
            allow_outside,
            sensitive_file_approved,
            web_search_config: &self.web_search_config,
            auth_store: &self.auth_store,
            event_tx,
            instruction_sources: Some(Arc::new(Mutex::new(Vec::new()))),
        };
        let source_sink = ctx.instruction_sources.clone().expect("source sink");
        let result = execute_tool_call(&ctx, call, cancel).await;
        if let Ok(mut pending) = self.pending_instruction_sources.lock()
            && let Ok(sources) = source_sink.lock()
            && !sources.is_empty()
        {
            pending
                .entry(session_id)
                .or_default()
                .extend(sources.iter().cloned());
        }
        result
    }

    /// Take instruction sources discovered by tools in a session.
    pub fn take_instruction_sources(&self, session_id: Uuid) -> Vec<String> {
        self.pending_instruction_sources
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(&session_id))
            .unwrap_or_default()
    }

    /// Return all available tool definitions (unfiltered, without MCP tools).
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let skill_description = self.skills.tool_description();
        tidev_tools::tool_definitions(skill_description)
    }

    /// Return tool definitions filtered for the given model.
    ///
    /// GPT models (gpt-4o, gpt-4o-mini, gpt-5, etc.) receive `apply_patch` but
    /// not `write`/`edit`. All other models (Claude, DeepSeek, Gemini, GPT-4,
    /// any OSS model) receive `write`/`edit` but not `apply_patch`.
    ///
    /// MCP tools from connected servers are appended at the end.
    pub fn definitions_for_model(&self, model: &ActiveModel) -> Vec<ToolDefinition> {
        let mut definitions = tidev_tools::tool_definitions(self.skills.tool_description());
        if model.use_apply_patch() {
            definitions.retain(|d| d.name != "edit" && d.name != "write");
        } else {
            definitions.retain(|d| d.name != "apply_patch");
        }
        definitions.extend(self.mcp.all_definitions());
        definitions
    }

    /// Access the skill catalog.
    pub fn skills(&self) -> &SkillCatalog {
        &self.skills
    }

    /// Execute a built-in call through the generic agent registry while
    /// preserving the original host execution and streaming contract.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn execute_via_agent(
        &self,
        call: &ToolCall,
        session_id: Uuid,
        request_id: u64,
        mode: Mode,
        allow_outside: bool,
        sensitive_file_approved: bool,
        cancel: &CancellationToken,
        event_tx: Option<AgentEventSender>,
        stream_shell: bool,
    ) -> ToolExecutionResult {
        execute_builtin_via_agent(
            self,
            call,
            session_id,
            request_id,
            mode,
            allow_outside,
            sensitive_file_approved,
            cancel,
            event_tx,
            stream_shell,
        )
        .await
    }

    pub(crate) fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }

    /// Access the MCP manager.
    pub fn mcp_manager(&self) -> &McpManager {
        &self.mcp
    }

    /// Return summaries of all configured MCP servers.
    pub fn mcp_summaries(&self) -> Vec<McpServerSummary> {
        self.mcp.summaries()
    }

    // ── Tool lookup helpers (for TUI permission UI) ─────────────────────

    /// Look up a [`ToolDefinition`] by name (supports canonical name aliases
    /// and MCP tools).
    pub fn definition_for(&self, tool_name: &str) -> Option<ToolDefinition> {
        // First try exact match in built-in tools.
        let definitions = self.definitions();
        if let Some(def) = definitions.iter().find(|d| d.name == tool_name) {
            return Some(def.clone());
        }
        // Then try MCP tools.
        if let Some(def) = self.mcp.definition_for(tool_name) {
            return Some(def);
        }
        // Fall back to canonical name lookup.
        let canonical = tidev_utils::tool_name::canonical_tool_name(tool_name)?;
        definitions.into_iter().find(|d| d.name == canonical)
    }

    /// Returns `true` if the tool exists and its permission level is allowed
    /// in the given session mode (hardcoded per mode).
    pub fn can_execute(&self, tool_name: &str, mode: Mode) -> bool {
        // Check built-in tools first.
        if self
            .definition_for(tool_name)
            .is_some_and(|def| def.permission.allowed_in_read_only(mode == Mode::Plan))
        {
            return true;
        }
        // Then check MCP tools.
        self.mcp.can_execute(tool_name, mode)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// Stub TodoPersistence for tests.
    struct StubTodoStore;
    impl TodoPersistence for StubTodoStore {
        fn load_todos(
            &self,
            _session_id: Uuid,
        ) -> anyhow::Result<Vec<tidev_tools::types::TodoItem>> {
            Ok(Vec::new())
        }
        fn replace_todos(
            &self,
            _session_id: Uuid,
            _todos: &[tidev_tools::types::TodoItem],
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    // ── MCP integration tests ──────────────────────────────────────────

    fn make_stdio_config() -> tidev_config::mcp::McpServerConfig {
        tidev_config::mcp::McpServerConfig::Stdio {
            command: "node".into(),
            args: vec!["server.js".into()],
            cwd: None,
            env: BTreeMap::new(),
        }
    }

    fn make_registry_with_mcp() -> ToolRegistry {
        let mcp = McpManager::new(
            PathBuf::from("/tmp"),
            BTreeMap::from([("srv".to_string(), make_stdio_config())]),
        );
        ToolRegistry::new(
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp/.config"),
            SkillCatalog::default(),
            Arc::new(StubTodoStore),
            WebSearchConfig::default(),
            AuthStore::default(),
            0,
            mcp,
        )
    }

    #[test]
    fn test_mcp_definition_for_disconnected_server_is_unavailable() {
        let reg = make_registry_with_mcp();
        let def = reg.definition_for("mcp__srv__tool");
        assert!(def.is_none());
    }

    #[test]
    fn test_mcp_cannot_execute_when_disconnected() {
        let reg = make_registry_with_mcp();
        assert!(!reg.can_execute("mcp__srv__tool", Mode::Build));
        assert!(!reg.can_execute("mcp__srv__tool", Mode::Plan));
    }

    #[test]
    fn test_mcp_definitions_for_model_excludes_disconnected_tools() {
        let reg = make_registry_with_mcp();
        // Use a model that doesn't apply_patch (the default path).
        let defs = reg.definitions_for_model(&tidev_config::auth::ActiveModel {
            provider_id: "test".into(),
            provider_display_name: "Test".into(),
            base_url: String::new(),
            api_type: tidev_config::types::ApiType::OpenAiChatCompletions,
            model_id: "test-model".into(),
            request_model_id: String::new(),
            display_name: "Test Model".into(),
            context_window: 0,
            max_output_tokens: 4096,
            temperature: None,
            supports_images: false,
            supports_parallel_tool_calls: true,
            system_prompt: String::new(),
            api_key: None,
            extra_body: None,
            thinking_level: tidev_config::reasoning::ThinkingLevelType::None,
        });
        let mcp_names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(
            !mcp_names.contains(&"mcp__srv__tool"),
            "Disconnected MCP tools must not be offered to the model: {mcp_names:?}"
        );
    }

    #[test]
    fn test_mcp_summaries() {
        let reg = make_registry_with_mcp();
        let summaries = reg.mcp_summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "srv");
        assert_eq!(summaries[0].tool_count, 0);
    }

    #[test]
    fn test_mcp_manager_accessor() {
        let reg = make_registry_with_mcp();
        let mcp = reg.mcp_manager();
        assert!(mcp.has_server("srv"));
    }
}
