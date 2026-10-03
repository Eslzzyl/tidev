//! Tool registry — wraps `tidev_tools` dispatch with tidev-core concerns.
//!
//! This module provides [`ToolRegistry`], the single entry point for tool
//! execution within tidev-core. It delegates to `tidev_tools::execute_tool_call`
//! / `execute_tool_call_streaming` while managing the `ToolContext` lifecycle
//! (workspace paths, skills catalog, web search config, etc.).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_agent::AgentEventSender;
use tidev_llm::message::{ToolCall, ToolExecutionResult};
use tidev_tools::types::{ToolDefinition, ToolOrigin};

use tidev_codemode::{CodeModeRuntime, CodeModeRuntimeConfig, CodeModeTool};
use tidev_config::auth::ActiveModel;
use tidev_config::{AuthStore, CodeModeConfig, CodeModeExposure, WebSearchConfig};
use tidev_tools::execute_tool_call;
use tidev_tools::{ShellOutput, SkillCatalog, TodoPersistence};

use crate::mcp::{
    MCP_CALL_TOOL_NAME, MCP_LIST_TOOL_NAME, MCP_SEARCH_TOOL_NAME, McpManager, McpServerSummary,
};
use crate::mode::Mode;
use crate::tool_adapter::execute_builtin_via_agent;

pub const CODE_MODE_TOOL_NAME: &str = "codemode";
pub const CODE_MODE_SEARCH_TOOL_NAME: &str = "search_tools";
pub const CODE_MODE_CALL_TOOL_NAME: &str = "call_tool";

const CODE_MODE_SEARCH_DESCRIPTION: &str = "Search the current tidev tool catalog. Pass an optional query and limit; the result is a JSON array of records with name, description, input_schema, origin, and MCP target metadata when applicable.";
const CODE_MODE_CALL_DESCRIPTION: &str = "Call one tool returned by search_tools. Pass its name and a JSON object under arguments; the result has status, output, and metadata fields.";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeModeSearchArguments {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeModeCallArguments {
    name: String,
    arguments: Value,
}

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
    code_mode_exposure: CodeModeExposure,
    code_mode_runtime: Arc<CodeModeRuntime>,
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
        code_mode: &CodeModeConfig,
    ) -> Self {
        let timeout = Duration::from_secs(code_mode.timeout_seconds.max(1));
        let runtime = CodeModeRuntime::new(CodeModeRuntimeConfig {
            max_feed_duration: timeout,
            ..CodeModeRuntimeConfig::default()
        });
        Self {
            workspace_root,
            config_dir,
            skills,
            todo,
            web_search_config,
            auth_store,
            max_output_bytes,
            mcp,
            code_mode_exposure: code_mode.exposure,
            code_mode_runtime: Arc::new(runtime),
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
    /// MCP router tools are dispatched directly to the [`McpManager`].
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
        match call.name.as_str() {
            MCP_LIST_TOOL_NAME => match self.mcp.execute_list(call) {
                Ok(result) => return result,
                Err(error) => {
                    return ToolExecutionResult::new(format!(
                        "Error: MCP tool list failed: {error:#}"
                    ));
                }
            },
            MCP_SEARCH_TOOL_NAME => match self.mcp.execute_search(call) {
                Ok(result) => return result,
                Err(error) => {
                    return ToolExecutionResult::new(format!(
                        "Error: MCP tool search failed: {error:#}"
                    ));
                }
            },
            MCP_CALL_TOOL_NAME => match self.mcp.execute_mcp_call(call, mode).await {
                Ok(result) => return result,
                Err(error) => {
                    return ToolExecutionResult::new(format!(
                        "Error: MCP tool call failed: {error:#}"
                    ));
                }
            },
            _ => {}
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

    /// Return all available tool definitions, including the fixed MCP router.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let ordinary = self.ordinary_definitions();
        match self.code_mode_exposure {
            CodeModeExposure::Off => ordinary,
            CodeModeExposure::On => {
                let mut definitions = ordinary.clone();
                definitions.push(code_mode_definition());
                definitions
            }
            CodeModeExposure::Only => vec![code_mode_definition()],
        }
    }

    /// Return tool definitions filtered for the given model.
    ///
    /// GPT models (gpt-4o, gpt-4o-mini, gpt-5, gpt-6, etc.) receive `apply_patch` but
    /// not `write`/`edit`. All other models (Claude, DeepSeek, Gemini, GPT-4,
    /// any OSS model) receive `write`/`edit` but not `apply_patch`.
    ///
    /// The fixed MCP router definitions are included regardless of current
    /// server connection state.
    pub fn definitions_for_model(&self, model: &ActiveModel) -> Vec<ToolDefinition> {
        let ordinary = self.ordinary_definitions_for_model(model);
        match self.code_mode_exposure {
            CodeModeExposure::Off => ordinary,
            CodeModeExposure::On => {
                let mut definitions = ordinary.clone();
                definitions.push(code_mode_definition());
                definitions
            }
            CodeModeExposure::Only => vec![code_mode_definition()],
        }
    }

    /// Return the tools callable from a Code Mode script.
    pub fn code_mode_tools(&self) -> Vec<CodeModeTool> {
        vec![
            CodeModeTool {
                name: CODE_MODE_SEARCH_TOOL_NAME.to_string(),
                description: CODE_MODE_SEARCH_DESCRIPTION.to_string(),
            },
            CodeModeTool {
                name: CODE_MODE_CALL_TOOL_NAME.to_string(),
                description: CODE_MODE_CALL_DESCRIPTION.to_string(),
            },
        ]
    }

    fn ordinary_definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = tidev_tools::tool_definitions();
        definitions.extend(self.mcp.model_definitions());
        definitions
    }

    fn ordinary_definitions_for_model(&self, model: &ActiveModel) -> Vec<ToolDefinition> {
        let mut definitions = self.ordinary_definitions();
        if model.use_apply_patch() {
            definitions.retain(|d| d.name != "edit" && d.name != "write");
        } else {
            definitions.retain(|d| d.name != "apply_patch");
        }
        definitions
    }

    fn code_mode_catalog(&self, model: &ActiveModel, mode: Mode) -> Vec<ToolDefinition> {
        let mut definitions = tidev_tools::tool_definitions();
        definitions.retain(|definition| {
            definition
                .permission
                .allowed_in_read_only(mode == Mode::Plan)
        });
        definitions.extend(self.mcp.available_definitions(mode));

        if model.use_apply_patch() {
            definitions
                .retain(|definition| definition.name != "edit" && definition.name != "write");
        } else {
            definitions.retain(|definition| definition.name != "apply_patch");
        }

        definitions.retain(|definition| {
            !matches!(
                tidev_utils::tool_name::canonical_tool_name(&definition.name),
                Some("task") | Some("question") | Some(CODE_MODE_TOOL_NAME)
            )
        });
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        definitions
    }

    pub(crate) fn search_code_mode_tools(
        &self,
        model: &ActiveModel,
        mode: Mode,
        arguments: Value,
    ) -> Result<Value> {
        let arguments: CodeModeSearchArguments =
            serde_json::from_value(arguments).context("invalid arguments for search_tools")?;
        let limit = arguments.limit.unwrap_or(10).clamp(1, 50);
        let terms: Vec<String> = arguments
            .query
            .unwrap_or_default()
            .trim()
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();

        let mut matches: Vec<(usize, ToolDefinition)> = self
            .code_mode_catalog(model, mode)
            .into_iter()
            .filter_map(|definition| {
                code_mode_search_score(&definition, &terms).map(|score| (score, definition))
            })
            .collect();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.name.cmp(&right.name))
        });

        Ok(Value::Array(
            matches
                .into_iter()
                .take(limit)
                .map(|(_, definition)| code_mode_tool_record(&definition))
                .collect(),
        ))
    }

    pub(crate) fn prepare_code_mode_call(
        &self,
        model: &ActiveModel,
        mode: Mode,
        arguments: Value,
    ) -> Result<ToolCall> {
        let arguments: CodeModeCallArguments =
            serde_json::from_value(arguments).context("invalid arguments for call_tool")?;
        if arguments.name.trim().is_empty() {
            bail!("call_tool requires a non-empty tool name");
        }
        if !arguments.arguments.is_object() {
            bail!("call_tool arguments must be a JSON object");
        }

        let definition = self
            .code_mode_catalog(model, mode)
            .into_iter()
            .find(|definition| definition.name == arguments.name)
            .with_context(|| {
                format!(
                    "tool '{}' is not currently available; call search_tools again",
                    arguments.name
                )
            })?;

        let (name, call_arguments) = match definition.origin {
            ToolOrigin::Local => (
                definition.name,
                serde_json::to_string(&arguments.arguments)
                    .context("failed to serialize call_tool arguments")?,
            ),
            ToolOrigin::Mcp {
                server_name,
                tool_name,
            } => (
                MCP_CALL_TOOL_NAME.to_string(),
                serde_json::to_string(&json!({
                    "server": server_name,
                    "tool": tool_name,
                    "arguments": arguments.arguments,
                }))
                .context("failed to serialize MCP call arguments")?,
            ),
        };

        Ok(ToolCall {
            id: String::new(),
            name,
            arguments: call_arguments,
            thought_signature: None,
        })
    }

    pub(crate) fn code_mode_runtime(&self) -> Arc<CodeModeRuntime> {
        self.code_mode_runtime.clone()
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

    /// Look up a [`ToolDefinition`] by name (supports canonical name aliases).
    pub fn definition_for(&self, tool_name: &str) -> Option<ToolDefinition> {
        // First try exact match in the fixed tool definitions.
        let definitions = self.definitions();
        if let Some(def) = definitions.iter().find(|d| d.name == tool_name) {
            return Some(def.clone());
        }
        // Fall back to canonical name lookup.
        let canonical = tidev_utils::tool_name::canonical_tool_name(tool_name)?;
        definitions.into_iter().find(|d| d.name == canonical)
    }

    /// Returns `true` if the tool exists and its permission level is allowed
    /// in the given session mode (hardcoded per mode).
    pub fn can_execute(&self, tool_name: &str, mode: Mode) -> bool {
        if tool_name == MCP_CALL_TOOL_NAME && mode == Mode::Plan {
            return false;
        }
        self.definition_for(tool_name)
            .is_some_and(|def| def.permission.allowed_in_read_only(mode == Mode::Plan))
    }

    /// Determine whether a concrete call is allowed in the given session mode.
    ///
    /// `mcp_call` checks the live target because its permission depends on the
    /// selected MCP tool rather than its fixed outer definition.
    pub fn can_execute_call(&self, call: &ToolCall, mode: Mode) -> anyhow::Result<bool> {
        if call.name == MCP_CALL_TOOL_NAME {
            return self.mcp.can_execute_mcp_call(call, mode);
        }
        Ok(self.can_execute(&call.name, mode))
    }

    pub(crate) fn can_execute_code_mode_call(
        &self,
        call: &ToolCall,
        mode: Mode,
    ) -> anyhow::Result<bool> {
        if call.name == MCP_CALL_TOOL_NAME {
            return self.mcp.can_execute_mcp_call(call, mode);
        }
        let definitions = self.ordinary_definitions();
        let definition = definitions
            .iter()
            .find(|definition| definition.name == call.name)
            .or_else(|| {
                tidev_utils::tool_name::canonical_tool_name(&call.name).and_then(|canonical| {
                    definitions
                        .iter()
                        .find(|definition| definition.name == canonical)
                })
            });
        Ok(definition.is_some_and(|definition| {
            definition
                .permission
                .allowed_in_read_only(mode == Mode::Plan)
        }))
    }
}

fn code_mode_definition() -> ToolDefinition {
    ToolDefinition {
        name: CODE_MODE_TOOL_NAME.to_string(),
        display_name: "Code Mode".to_string(),
        description: format!(
            "Run one Python script in a fresh sandbox. The host provides two generic functions; the current tool catalog is available through search_tools at runtime.\n\n\
             search_tools({{\"query\": \"optional search terms\", \"limit\": 10}}) returns a JSON array. Each record contains name, description, input_schema, origin, and MCP target metadata when applicable. Omit the query to receive the first records.\n\
             call_tool({{\"name\": \"tool name from search_tools\", \"arguments\": {{}}}}) invokes one selected tool and returns {{\"status\": \"completed\", \"output\": \"...\", \"metadata\": {{...}}}}. An approval rejection returns status \"rejected\"; inspect output for the tool result or error text.\n\
             Use print(value) or return value to include data in the final Code Mode result. Tool calls run through tidev's normal approval and permission checks.\n\n\
             Host functions:\n- {}: {}\n- {}: {}",
            CODE_MODE_SEARCH_TOOL_NAME,
            CODE_MODE_SEARCH_DESCRIPTION,
            CODE_MODE_CALL_TOOL_NAME,
            CODE_MODE_CALL_DESCRIPTION,
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "Python code to run in a fresh Monty sandbox."
                }
            },
            "required": ["code"],
            "additionalProperties": false
        }),
        permission: tidev_tools::types::ToolPermission::Execute,
        origin: tidev_tools::types::ToolOrigin::Local,
    }
}

fn code_mode_search_score(definition: &ToolDefinition, terms: &[String]) -> Option<usize> {
    if terms.is_empty() {
        return Some(0);
    }

    let name = definition.name.to_lowercase();
    let display_name = definition.display_name.to_lowercase();
    let description = definition.description.to_lowercase();
    let schema = definition.parameters.to_string().to_lowercase();
    let (server, tool) = definition
        .mcp_target()
        .map(|(server, tool)| (server.to_lowercase(), tool.to_lowercase()))
        .unwrap_or_default();

    let mut score = 0;
    for term in terms {
        let mut matched = false;
        if name.contains(term) {
            score += 8;
            matched = true;
        }
        if tool.contains(term) {
            score += 8;
            matched = true;
        }
        if server.contains(term) {
            score += 4;
            matched = true;
        }
        if display_name.contains(term) {
            score += 3;
            matched = true;
        }
        if description.contains(term) {
            score += 2;
            matched = true;
        }
        if schema.contains(term) {
            score += 1;
            matched = true;
        }
        if !matched {
            return None;
        }
    }
    Some(score)
}

fn code_mode_tool_record(definition: &ToolDefinition) -> Value {
    let mut record = json!({
        "name": definition.name,
        "display_name": definition.display_name,
        "description": definition.description,
        "input_schema": definition.parameters,
        "permission": format!("{:?}", definition.permission).to_lowercase(),
        "origin": if definition.mcp_target().is_some() { "mcp" } else { "local" },
    });
    if let Some((server, tool)) = definition.mcp_target() {
        record["server"] = Value::String(server.to_string());
        record["tool"] = Value::String(tool.to_string());
    }
    record
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
            disabled: false,
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
            &CodeModeConfig::default(),
        )
    }

    fn test_model() -> tidev_config::auth::ActiveModel {
        tidev_config::auth::ActiveModel {
            provider_id: "test".into(),
            provider_display_name: "Test".into(),
            base_url: String::new(),
            user_agent: None,
            headers: std::collections::BTreeMap::new(),
            session_header: None,
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
        }
    }

    #[test]
    fn test_code_mode_uses_fixed_generic_functions() {
        let mut reg = make_registry_with_mcp();
        reg.code_mode_exposure = CodeModeExposure::Only;

        let functions: Vec<String> = reg
            .code_mode_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert_eq!(
            functions,
            vec![
                CODE_MODE_SEARCH_TOOL_NAME.to_owned(),
                CODE_MODE_CALL_TOOL_NAME.to_owned()
            ]
        );

        let definition = reg
            .definitions()
            .into_iter()
            .find(|definition| definition.name == CODE_MODE_TOOL_NAME)
            .expect("Code Mode definition should be available");
        assert!(definition.description.contains(CODE_MODE_SEARCH_TOOL_NAME));
        assert!(definition.description.contains(CODE_MODE_CALL_TOOL_NAME));
        assert!(!definition.description.contains("mcp__"));
    }

    #[test]
    fn test_code_mode_search_returns_structured_catalog_records() {
        let reg = make_registry_with_mcp();
        let result = reg
            .search_code_mode_tools(
                &test_model(),
                Mode::Build,
                json!({"query": "read", "limit": 5}),
            )
            .unwrap();
        let records = result.as_array().expect("search result should be an array");
        assert!(
            records
                .iter()
                .any(|record| { record.get("name").and_then(Value::as_str) == Some("read") })
        );
        assert!(records.iter().all(|record| {
            record.get("input_schema").is_some()
                && record.get("description").is_some()
                && record.get("origin").is_some()
        }));
    }

    #[test]
    fn test_code_mode_call_resolves_local_tool_name() {
        let reg = make_registry_with_mcp();
        let call = reg
            .prepare_code_mode_call(
                &test_model(),
                Mode::Build,
                json!({
                    "name": "read",
                    "arguments": {"path": "README.md"}
                }),
            )
            .unwrap();
        assert_eq!(call.name, "read");
        assert_eq!(
            serde_json::from_str::<Value>(&call.arguments).unwrap(),
            json!({"path": "README.md"})
        );
    }

    #[test]
    fn test_fixed_mcp_definitions_are_available_without_servers() {
        let reg = make_registry_with_mcp();
        assert!(reg.definition_for(MCP_LIST_TOOL_NAME).is_some());
        assert!(reg.definition_for(MCP_SEARCH_TOOL_NAME).is_some());
        assert!(reg.definition_for(MCP_CALL_TOOL_NAME).is_some());
        assert!(reg.definition_for("mcp__srv__tool").is_none());
    }

    #[test]
    fn test_mcp_call_checks_live_target_availability() {
        let reg = make_registry_with_mcp();
        let call = ToolCall {
            id: "call-1".into(),
            name: MCP_CALL_TOOL_NAME.into(),
            arguments: r#"{"server":"srv","tool":"tool","arguments":{}}"#.into(),
            thought_signature: None,
        };
        assert!(!reg.can_execute_call(&call, Mode::Build).unwrap());
        assert!(!reg.can_execute_call(&call, Mode::Plan).unwrap());
        assert!(!reg.can_execute(MCP_CALL_TOOL_NAME, Mode::Plan));
    }

    #[test]
    fn test_mcp_definitions_for_model_are_connection_independent() {
        let reg = make_registry_with_mcp();
        // Use a model that doesn't apply_patch (the default path).
        let defs = reg.definitions_for_model(&test_model());
        let mcp_names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(
            !mcp_names.contains(&"mcp__srv__tool"),
            "Live MCP tools must not be offered directly to the model: {mcp_names:?}"
        );
        assert!(mcp_names.contains(&MCP_LIST_TOOL_NAME));
        assert!(mcp_names.contains(&MCP_SEARCH_TOOL_NAME));
        assert!(mcp_names.contains(&MCP_CALL_TOOL_NAME));
        assert!(
            !mcp_names.iter().any(|name| name.starts_with("mcp__")),
            "Flattened MCP names must never reach the model: {mcp_names:?}"
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
