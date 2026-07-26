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

use crate::mcp::{McpManager, McpServerSummary};

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
    permission_config: PermissionConfig,
    mcp: McpManager,
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
            permission_config,
            mcp,
        }
    }

    /// Execute a tool call with cooperative cancellation and optional streaming.
    ///
    /// Shell commands honor the `cancel` token — when cancelled, the
    /// process group is killed and partial output is returned. Other tools ignore it.
    /// When `event_tx` is `Some`, shell output is streamed as
    /// [`BackendEvent::ShellOutput`] events.
    ///
    /// MCP-backed tools are dispatched directly to the [`McpManager`].
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
        let canonical = tidev_types::tools::canonical_tool_name(tool_name)?;
        definitions.into_iter().find(|d| d.name == canonical)
    }

    /// Returns `true` if the tool exists and its permission level is allowed
    /// in the given session mode according to the user's permission config.
    pub fn can_execute(&self, tool_name: &str, mode: SessionMode) -> bool {
        // Check built-in tools first.
        if self
            .definition_for(tool_name)
            .is_some_and(|def| self.permission_config.is_allowed(mode, def.permission))
        {
            return true;
        }
        // Then check MCP tools.
        self.mcp
            .can_execute(tool_name, mode, &self.permission_config)
    }

    /// Return a stable key for a tool call (used for permission memoization).
    pub fn permission_key_for_call(&self, call: &ToolCall) -> String {
        if call.name == "skill" {
            if let Ok(args) = serde_json::from_str::<tidev_types::tools::SkillArgs>(&call.arguments)
                && !args.name.trim().is_empty()
            {
                return SkillCatalog::permission_key_for_name(args.name.trim());
            }
            return SkillCatalog::permission_key_for_name("unknown");
        }

        // Check MCP first (MCP tools won't appear in built-in definitions).
        if self.mcp.definition_for(&call.name).is_some() {
            return self.mcp.permission_key_for_call(call);
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
            if let Ok(args) = serde_json::from_str::<tidev_types::tools::SkillArgs>(&call.arguments)
                && !args.name.trim().is_empty()
            {
                return format!("skill '{}'", args.name.trim());
            }
            return "skill".to_string();
        }

        // Check MCP first.
        if self.mcp.definition_for(&call.name).is_some() {
            return self.mcp.permission_label_for_call(call);
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
        ) -> anyhow::Result<Vec<tidev_types::tools::TodoItem>> {
            Ok(Vec::new())
        }
        fn replace_todos(
            &self,
            _session_id: Uuid,
            _todos: &[tidev_types::tools::TodoItem],
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn make_registry() -> ToolRegistry {
        ToolRegistry::new(
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp/.config"),
            SkillCatalog::default(),
            Arc::new(StubTodoStore),
            WebSearchConfig::default(),
            AuthStore::default(),
            0,
            PermissionConfig::default(),
            McpManager::new(PathBuf::from("/tmp"), BTreeMap::new()),
        )
    }

    #[test]
    fn permission_key_for_skill() {
        let reg = make_registry();
        let call = ToolCall {
            id: "c1".into(),
            name: "skill".into(),
            arguments: r#"{"name":"code-review"}"#.into(),
            thought_signature: None,
        };
        assert_eq!(reg.permission_key_for_call(&call), "skill:code-review");
    }

    #[test]
    fn permission_key_for_skill_without_name() {
        let reg = make_registry();
        let call = ToolCall {
            id: "c1".into(),
            name: "skill".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        assert_eq!(reg.permission_key_for_call(&call), "skill:unknown");
    }

    #[test]
    fn permission_label_for_skill() {
        let reg = make_registry();
        let call = ToolCall {
            id: "c1".into(),
            name: "skill".into(),
            arguments: r#"{"name":"code-review"}"#.into(),
            thought_signature: None,
        };
        assert_eq!(reg.permission_label_for_call(&call), "skill 'code-review'");
    }

    #[test]
    fn permission_label_for_skill_without_name() {
        let reg = make_registry();
        let call = ToolCall {
            id: "c1".into(),
            name: "skill".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        assert_eq!(reg.permission_label_for_call(&call), "skill");
    }

    #[test]
    fn permission_label_for_known_tool() {
        let reg = make_registry();
        let call = ToolCall {
            id: "c1".into(),
            name: "read".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        assert_eq!(reg.permission_label_for_call(&call), "read");
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
        let mcp = McpManager::new(PathBuf::from("/tmp"), BTreeMap::new());
        let tool = ToolDefinition::mcp(
            "mcp__srv__tool".into(),
            "Srv Tool".into(),
            "Does something".into(),
            serde_json::json!({"type": "object"}),
            tidev_types::tools::ToolPermission::Execute,
            "srv".into(),
            "tool".into(),
        );
        crate::mcp::insert_mock_tool(&mcp, "srv", make_stdio_config(), tool);
        ToolRegistry::new(
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp/.config"),
            SkillCatalog::default(),
            Arc::new(StubTodoStore),
            WebSearchConfig::default(),
            AuthStore::default(),
            0,
            PermissionConfig::default(),
            mcp,
        )
    }

    #[test]
    fn test_mcp_definition_for_includes_mcp_tools() {
        let reg = make_registry_with_mcp();
        let def = reg.definition_for("mcp__srv__tool");
        assert!(def.is_some());
        assert_eq!(def.unwrap().mcp_target(), Some(("srv", "tool")));
    }

    #[test]
    fn test_mcp_permission_key_for_call() {
        let reg = make_registry_with_mcp();
        let call = ToolCall {
            id: "c1".into(),
            name: "mcp__srv__tool".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        assert_eq!(
            reg.permission_key_for_call(&call),
            "mcp:srv:tool"
        );
    }

    #[test]
    fn test_mcp_permission_label_for_call() {
        let reg = make_registry_with_mcp();
        let call = ToolCall {
            id: "c1".into(),
            name: "mcp__srv__tool".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        assert_eq!(
            reg.permission_label_for_call(&call),
            "srv / tool (Srv Tool)"
        );
    }

    #[test]
    fn test_mcp_can_execute() {
        let reg = make_registry_with_mcp();
        // MCP tool has ToolPermission::Execute, Build mode allows execute.
        assert!(reg.can_execute("mcp__srv__tool", SessionMode::Build));
        // Plan mode allows execute by default too.
        assert!(reg.can_execute("mcp__srv__tool", SessionMode::Plan));
    }

    #[test]
    fn test_mcp_definitions_for_model_includes_mcp_tools() {
        let reg = make_registry_with_mcp();
        // Use a model that doesn't apply_patch (the default path).
        // Simply check that MCP tools are included in definitions_for_model.
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
            system_prompt: String::new(),
            api_key: None,
            extra_body: None,
            thinking_level: tidev_config::reasoning::ThinkingLevelType::None,
        });
        let mcp_names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(
            mcp_names.contains(&"mcp__srv__tool"),
            "MCP tool should be in definitions_for_model output: {mcp_names:?}"
        );
    }

    #[test]
    fn test_mcp_summaries() {
        let reg = make_registry_with_mcp();
        let summaries = reg.mcp_summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "srv");
        assert_eq!(summaries[0].tool_count, 1);
    }

    #[test]
    fn test_mcp_manager_accessor() {
        let reg = make_registry_with_mcp();
        let mcp = reg.mcp_manager();
        assert!(mcp.has_server("srv"));
    }
}
