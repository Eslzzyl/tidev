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
            if let Ok(args) = serde_json::from_str::<tidev_types::tools::SkillArgs>(&call.arguments)
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
            if let Ok(args) = serde_json::from_str::<tidev_types::tools::SkillArgs>(&call.arguments)
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tidev_config::ApiType;
    use tidev_config::auth::ActiveModel;

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
        )
    }

    fn make_model(model_id: &str) -> ActiveModel {
        ActiveModel {
            provider_id: "test".into(),
            provider_display_name: "Test".into(),
            base_url: "https://api.test.com".into(),
            api_type: ApiType::OpenAiChatCompletions,
            model_id: model_id.into(),
            request_model_id: model_id.into(),
            display_name: model_id.into(),
            context_window: 200_000,
            max_output_tokens: 8_000,
            temperature: None,
            supports_images: false,
            system_prompt: String::new(),
            api_key: None,
            extra_body: None,
            thinking_level: tidev_config::ThinkingLevelType::None,
        }
    }

    // ── definitions_for_model ───────────────────────────────────────────

    #[test]
    fn definitions_for_gpt_4o_excludes_write_edit() {
        let reg = make_registry();
        let model = make_model("gpt-4o");
        let defs = reg.definitions_for_model(&model);
        assert!(!defs.iter().any(|d| d.name == "write"));
        assert!(!defs.iter().any(|d| d.name == "edit"));
        assert!(defs.iter().any(|d| d.name == "apply_patch"));
    }

    #[test]
    fn definitions_for_claude_includes_write_edit() {
        let reg = make_registry();
        let model = make_model("claude-3-5-sonnet");
        let defs = reg.definitions_for_model(&model);
        assert!(defs.iter().any(|d| d.name == "write"));
        assert!(defs.iter().any(|d| d.name == "edit"));
        assert!(!defs.iter().any(|d| d.name == "apply_patch"));
    }

    #[test]
    fn definitions_for_gpt4_legacy_includes_write_edit() {
        // GPT-4 (non-o) should NOT use apply_patch
        let reg = make_registry();
        let model = make_model("gpt-4");
        let defs = reg.definitions_for_model(&model);
        assert!(defs.iter().any(|d| d.name == "write"));
        assert!(defs.iter().any(|d| d.name == "edit"));
        assert!(!defs.iter().any(|d| d.name == "apply_patch"));
    }

    #[test]
    fn definitions_for_deepseek_includes_write_edit() {
        let reg = make_registry();
        let model = make_model("deepseek-v4-flash");
        let defs = reg.definitions_for_model(&model);
        assert!(defs.iter().any(|d| d.name == "write"));
        assert!(!defs.iter().any(|d| d.name == "apply_patch"));
    }

    #[test]
    fn definitions_includes_core_tools() {
        let reg = make_registry();
        let model = make_model("claude-3-5-sonnet");
        let defs = reg.definitions_for_model(&model);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"read"));
        assert!(names.contains(&"shell"));
        assert!(names.contains(&"glob"));
        assert!(names.contains(&"grep"));
        assert!(names.contains(&"task"));
        assert!(names.contains(&"question"));
        assert!(names.contains(&"skill"));
        assert!(names.contains(&"todowrite"));
    }

    // ── definition_for (alias lookup) ────────────────────────────────────

    #[test]
    fn definition_for_exact_name() {
        let reg = make_registry();
        assert!(reg.definition_for("read").is_some());
    }

    #[test]
    fn definition_for_alias_name() {
        let reg = make_registry();
        // "read_file" is a canonical alias for "read"
        assert!(reg.definition_for("read_file").is_some());
    }

    #[test]
    fn definition_for_unknown_name() {
        let reg = make_registry();
        assert!(reg.definition_for("nonexistent_tool").is_none());
    }

    // ── can_execute ─────────────────────────────────────────────────────

    #[test]
    fn can_execute_read_in_plan_mode() {
        let reg = make_registry();
        assert!(reg.can_execute("read", SessionMode::Plan));
    }

    #[test]
    fn can_execute_write_in_plan_mode_default_false() {
        let reg = make_registry();
        // Default permission: plan mode disallows write
        assert!(!reg.can_execute("write", SessionMode::Plan));
    }

    #[test]
    fn can_execute_write_in_build_mode() {
        let reg = make_registry();
        assert!(reg.can_execute("write", SessionMode::Build));
    }

    #[test]
    fn can_execute_unknown_tool() {
        let reg = make_registry();
        assert!(!reg.can_execute("nonexistent", SessionMode::Plan));
    }

    // ── permission_key_for_call ─────────────────────────────────────────

    #[test]
    fn permission_key_for_known_tool() {
        let reg = make_registry();
        let call = ToolCall {
            id: "c1".into(),
            name: "read".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        // Built-in tools use their own name as the permission key
        assert_eq!(reg.permission_key_for_call(&call), "read");
    }

    #[test]
    fn permission_key_for_skill_with_name() {
        let reg = make_registry();
        let call = ToolCall {
            id: "c1".into(),
            name: "skill".into(),
            arguments: r#"{"name":"debug"}"#.into(),
            thought_signature: None,
        };
        assert_eq!(reg.permission_key_for_call(&call), "skill:debug");
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
    fn permission_key_for_alias_tool() {
        let reg = make_registry();
        let call = ToolCall {
            id: "c1".into(),
            name: "read_file".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        // Falls back to canonical name "read"
        assert_eq!(reg.permission_key_for_call(&call), "read");
    }

    #[test]
    fn permission_key_for_unknown_tool() {
        let reg = make_registry();
        let call = ToolCall {
            id: "c1".into(),
            name: "unknown_tool".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        // Falls back to original name since no canonical mapping either
        assert_eq!(reg.permission_key_for_call(&call), "unknown_tool");
    }

    // ── permission_label_for_call ───────────────────────────────────────

    #[test]
    fn permission_label_for_skill_with_name() {
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
}
