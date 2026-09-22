//! tidev's MCP integration layer.
//!
//! Connection management and result formatting live in `tidev-agent`. This
//! module maps tidev configuration and permission metadata onto that generic
//! client while retaining the public API used by the TUI and runtime.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use tidev_agent::{McpRegistry, McpServerSpec, McpToolInfo};
use tidev_config::mcp::McpServerConfig;
use tidev_llm::message::{ToolCall, ToolExecutionResult};
use tidev_tools::types::{ToolDefinition, ToolOrigin, ToolPermission};

use crate::mode::Mode;

pub use tidev_agent::{McpConnectionStatus, McpServerSummary};

/// Stable model-facing entry point for listing the live MCP catalog.
pub const MCP_LIST_TOOL_NAME: &str = "mcp_list";
/// Stable model-facing entry point for searching the live MCP catalog.
pub const MCP_SEARCH_TOOL_NAME: &str = "mcp_search";
/// Stable model-facing entry point for calling a live MCP tool.
pub const MCP_CALL_TOOL_NAME: &str = "mcp_call";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct McpListArguments {
    #[serde(default)]
    server: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct McpSearchArguments {
    query: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct McpCallArguments {
    server: String,
    tool: String,
    arguments: Value,
}

/// Product-facing MCP manager.
#[derive(Clone, Debug)]
pub struct McpManager {
    registry: McpRegistry,
    workspace_root: PathBuf,
    configs: Arc<Mutex<BTreeMap<String, McpServerConfig>>>,
}

impl McpManager {
    /// Create a manager from tidev configuration.
    pub fn new(workspace_root: PathBuf, servers: BTreeMap<String, McpServerConfig>) -> Self {
        let specs = servers
            .iter()
            .map(|(name, config)| (name.clone(), to_agent_spec(&workspace_root, config)))
            .collect();
        Self {
            registry: McpRegistry::new(specs),
            workspace_root,
            configs: Arc::new(Mutex::new(servers)),
        }
    }

    pub fn has_connecting(&self) -> bool {
        self.registry.has_connecting()
    }

    pub async fn wait_until_ready(&self, timeout: std::time::Duration) -> Result<()> {
        self.registry.wait_until_ready(timeout).await
    }

    pub async fn refresh_all(&self) -> Result<()> {
        self.registry.refresh_all().await
    }

    pub async fn refresh_server(&self, name: &str) -> Result<()> {
        self.registry.refresh_server(name).await
    }

    pub async fn upsert_server(&self, name: String, config: McpServerConfig) -> Result<()> {
        let spec = to_agent_spec(&self.workspace_root, &config);
        self.configs.lock().unwrap().insert(name.clone(), config);
        self.registry.upsert_server(name, spec).await
    }

    pub async fn remove_server(&self, name: &str) -> Result<()> {
        self.registry.remove_server(name).await?;
        self.configs.lock().unwrap().remove(name);
        Ok(())
    }

    pub fn server_config(&self, name: &str) -> Option<McpServerConfig> {
        self.configs.lock().unwrap().get(name).cloned()
    }

    pub fn has_server(&self, name: &str) -> bool {
        self.registry.has_server(name)
    }

    pub async fn disconnect_server(&self, name: &str) -> Result<()> {
        self.registry.disconnect_server(name).await
    }

    pub async fn toggle_server(&self, name: &str) -> Result<()> {
        self.registry.toggle_server(name).await
    }

    pub fn summaries(&self) -> Vec<McpServerSummary> {
        self.registry.summaries()
    }

    /// Return the fixed MCP tool definitions exposed to every model request.
    ///
    /// Live server state is deliberately absent from these definitions. The
    /// model must query the current catalog with `mcp_list` or `mcp_search`
    /// before it calls `mcp_call`.
    pub fn model_definitions(&self) -> Vec<ToolDefinition> {
        vec![
            mcp_list_definition(),
            mcp_search_definition(),
            mcp_call_definition(),
        ]
    }

    /// List configured servers or the current tools of one server.
    pub fn execute_list(&self, call: &ToolCall) -> Result<ToolExecutionResult> {
        let args: McpListArguments =
            serde_json::from_str(&call.arguments).context("invalid arguments for mcp_list")?;

        let summaries = self.summaries();
        let mut lines = Vec::new();
        match args
            .server
            .as_deref()
            .map(str::trim)
            .filter(|server| !server.is_empty())
        {
            Some(server) => {
                summaries
                    .iter()
                    .find(|summary| summary.name == server)
                    .with_context(|| format!("MCP server '{server}' is not configured"))?;
                let tools: Vec<McpToolInfo> = self
                    .registry
                    .all_tools()
                    .into_iter()
                    .filter(|tool| tool.server_name == server)
                    .collect();

                lines.extend(
                    tools
                        .into_iter()
                        .map(tool_record)
                        .map(|record| jsonl_line(&record))
                        .collect::<Result<Vec<_>>>()?,
                );
            }
            None => {
                lines.extend(
                    summaries
                        .iter()
                        .map(server_record)
                        .map(|record| jsonl_line(&record))
                        .collect::<Result<Vec<_>>>()?,
                );
            }
        }

        Ok(ToolExecutionResult::new(lines.join("\n")))
    }

    /// Search the live catalog of connected MCP tools.
    pub fn execute_search(&self, call: &ToolCall) -> Result<ToolExecutionResult> {
        let args: McpSearchArguments =
            serde_json::from_str(&call.arguments).context("invalid arguments for mcp_search")?;
        if args.query.trim().is_empty() {
            bail!(
                "mcp_search requires a non-empty query; use mcp_list to enumerate servers or tools"
            );
        }
        let terms: Vec<String> = args
            .query
            .trim()
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();

        let mut matches: Vec<(usize, McpToolInfo)> = self
            .registry
            .all_tools()
            .into_iter()
            .filter_map(|tool| search_score(&tool, &terms).map(|score| (score, tool)))
            .collect();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.server_name.cmp(&right.server_name))
                .then_with(|| left.tool_name.cmp(&right.tool_name))
        });

        let lines = matches
            .into_iter()
            .map(|(_, tool)| tool_record(tool))
            .map(|record| jsonl_line(&record))
            .collect::<Result<Vec<_>>>()?;
        Ok(ToolExecutionResult::new(lines.join("\n")))
    }

    /// Determine whether a specific current MCP target is permitted in a mode.
    pub fn can_execute_mcp_call(&self, call: &ToolCall, mode: Mode) -> Result<bool> {
        let args = parse_mcp_call_arguments(call)?;
        let target = self.registry.tool_info_for_target(&args.server, &args.tool);
        Ok(target.is_some_and(|tool| mode != Mode::Plan || tool.read_only))
    }

    /// Execute a call through the current MCP catalog.
    ///
    /// Target lookup is repeated at execution time, so servers can go online,
    /// offline, or refresh their tool list without changing the model-facing
    /// definition set. The plan-mode check is repeated here to close the gap
    /// between approval and execution.
    pub async fn execute_mcp_call(
        &self,
        call: &ToolCall,
        mode: Mode,
    ) -> Result<ToolExecutionResult> {
        let args = parse_mcp_call_arguments(call)?;
        let target = self
            .registry
            .tool_info_for_target(&args.server, &args.tool)
            .with_context(|| {
                format!(
                    "MCP tool '{}/{}' is not currently available; call mcp_search again",
                    args.server, args.tool
                )
            })?;
        if mode == Mode::Plan && !target.read_only {
            bail!(
                "MCP tool '{}/{}' is not allowed in plan mode",
                args.server,
                args.tool
            );
        }
        let arguments = args
            .arguments
            .as_object()
            .cloned()
            .context("mcp_call arguments must be a JSON object")?;
        self.registry
            .execute_target(&args.server, &args.tool, arguments)
            .await
    }

    pub fn available_definitions(&self, mode: Mode) -> Vec<ToolDefinition> {
        self.registry
            .all_tools()
            .into_iter()
            .filter(|tool| mode != Mode::Plan || tool.read_only)
            .map(|tool| to_host_definition(&tool))
            .collect()
    }

    pub fn all_definitions(&self) -> Vec<ToolDefinition> {
        self.registry
            .all_tools()
            .into_iter()
            .map(|tool| to_host_definition(&tool))
            .collect()
    }

    pub fn definition_for(&self, tool_name: &str) -> Option<ToolDefinition> {
        self.registry
            .tool_info_for(tool_name)
            .map(|tool| to_host_definition(&tool))
    }

    pub fn can_execute(&self, tool_name: &str, mode: Mode) -> bool {
        self.definition_for(tool_name).is_some_and(|definition| {
            definition
                .permission
                .allowed_in_read_only(mode == Mode::Plan)
        })
    }

    pub async fn execute_call(&self, call: &ToolCall) -> Result<ToolExecutionResult> {
        self.registry.execute_call(call).await
    }

    /// Access the generic registry for core-owned adapters.
    #[cfg(test)]
    pub(crate) fn agent_registry(&self) -> &McpRegistry {
        &self.registry
    }
}

fn mcp_list_definition() -> ToolDefinition {
    ToolDefinition {
        name: MCP_LIST_TOOL_NAME.to_string(),
        display_name: "MCP list".to_string(),
        description: "List configured MCP servers, or list the current tools of one server. Omit server to list servers; provide server to list that server's tools. Returns JSON Lines.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "Optional configured MCP server name. When omitted or empty, lists servers; when provided, lists that server's current tools."
                }
            },
            "additionalProperties": false
        }),
        permission: ToolPermission::Search,
        origin: ToolOrigin::Local,
    }
}

fn mcp_search_definition() -> ToolDefinition {
    ToolDefinition {
        name: MCP_SEARCH_TOOL_NAME.to_string(),
        display_name: "MCP search".to_string(),
        description: "Search currently connected MCP tools by tool name, server name, display name, description, and input schema. Returns every matching tool as JSON Lines.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Non-empty terms describing the MCP capability to find. Every term must match the same tool."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        permission: ToolPermission::Search,
        origin: ToolOrigin::Local,
    }
}

fn mcp_call_definition() -> ToolDefinition {
    ToolDefinition {
        name: MCP_CALL_TOOL_NAME.to_string(),
        display_name: "MCP call".to_string(),
        description: "Call a currently available MCP tool found with mcp_list or mcp_search. The server and tool values must match a catalog result, and arguments must follow that result's input schema.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "MCP server name returned by mcp_list or mcp_search."
                },
                "tool": {
                    "type": "string",
                    "description": "MCP tool name returned by mcp_list or mcp_search."
                },
                "arguments": {
                    "type": "object",
                    "description": "Arguments for the selected MCP tool."
                }
            },
            "required": ["server", "tool", "arguments"],
            "additionalProperties": false
        }),
        permission: ToolPermission::Execute,
        origin: ToolOrigin::Local,
    }
}

fn server_status(summary: &McpServerSummary) -> &str {
    if summary.disabled {
        "disabled"
    } else {
        summary.status.label()
    }
}

fn server_record(summary: &McpServerSummary) -> Value {
    let mut record = json!({
        "server": summary.name,
        "kind": summary.kind,
        "status": server_status(summary),
        "tool_count": summary.tool_count,
    });
    if !summary.disabled
        && let Some(error) = summary.status.detail()
    {
        record["error"] = Value::String(error.to_string());
    }
    record
}

fn tool_record(tool: McpToolInfo) -> Value {
    json!({
        "server": tool.server_name,
        "tool": tool.tool_name,
        "description": tool.definition.description,
        "input_schema": tool.definition.parameters,
        "read_only": tool.read_only,
    })
}

fn jsonl_line(value: &Value) -> Result<String> {
    serde_json::to_string(value).context("failed to serialize MCP catalog record")
}

fn parse_mcp_call_arguments(call: &ToolCall) -> Result<McpCallArguments> {
    let args: McpCallArguments =
        serde_json::from_str(&call.arguments).context("invalid arguments for mcp_call")?;
    if args.server.trim().is_empty() {
        bail!("mcp_call requires a non-empty server name");
    }
    if args.tool.trim().is_empty() {
        bail!("mcp_call requires a non-empty tool name");
    }
    if !args.arguments.is_object() {
        bail!("mcp_call arguments must be a JSON object");
    }
    Ok(args)
}

fn search_score(tool: &McpToolInfo, terms: &[String]) -> Option<usize> {
    let server = tool.server_name.to_lowercase();
    let name = tool.tool_name.to_lowercase();
    let display_name = tool.definition.display_name.to_lowercase();
    let description = tool.definition.description.to_lowercase();
    let schema = tool.definition.parameters.to_string().to_lowercase();

    let mut score = 0;
    for term in terms {
        let mut matched = false;
        if name.contains(term) {
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

fn to_agent_spec(workspace_root: &Path, config: &McpServerConfig) -> McpServerSpec {
    match config {
        McpServerConfig::Stdio {
            command,
            args,
            cwd,
            env,
            disabled,
        } => McpServerSpec::Stdio {
            command: resolve_stdio_command(workspace_root, command, cwd.as_deref()),
            args: args.clone(),
            cwd: cwd
                .as_deref()
                .map(|cwd| resolve_workspace_path(workspace_root, cwd)),
            env: env.clone(),
            disabled: *disabled,
        },
        McpServerConfig::Http {
            url,
            headers,
            disabled,
        } => McpServerSpec::Http {
            url: url.clone(),
            headers: headers.clone(),
            disabled: *disabled,
        },
        McpServerConfig::Sse {
            url,
            headers,
            disabled,
        } => McpServerSpec::Sse {
            url: url.clone(),
            headers: headers.clone(),
            disabled: *disabled,
        },
    }
}

fn resolve_workspace_path(workspace_root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

fn resolve_stdio_command(workspace_root: &Path, command: &str, cwd: Option<&str>) -> String {
    let path = Path::new(command);
    if path.is_absolute() || (!command.contains('/') && !command.contains('\\')) {
        return command.to_string();
    }

    let base = cwd
        .map(|value| resolve_workspace_path(workspace_root, value))
        .unwrap_or_else(|| workspace_root.to_path_buf());
    base.join(path).to_string_lossy().into_owned()
}

fn to_host_definition(tool: &McpToolInfo) -> ToolDefinition {
    let permission = match tool.tool_name.as_str() {
        "websearch" => ToolPermission::Search,
        "webfetch" => ToolPermission::Read,
        _ if tool.read_only => ToolPermission::Read,
        _ => ToolPermission::Execute,
    };
    ToolDefinition::mcp(
        tool.definition.name.clone(),
        tool.definition.display_name.clone(),
        tool.definition.description.clone(),
        tool.definition.parameters.clone(),
        permission,
        tool.server_name.clone(),
        tool.tool_name.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio_config() -> McpServerConfig {
        McpServerConfig::Stdio {
            command: "node".into(),
            args: vec!["server.js".into()],
            cwd: None,
            env: BTreeMap::new(),
            disabled: false,
        }
    }

    #[test]
    fn manager_preserves_config_and_summary_order() {
        let manager = McpManager::new(
            PathBuf::from("/workspace"),
            BTreeMap::from([
                ("b".to_string(), stdio_config()),
                ("a".to_string(), stdio_config()),
            ]),
        );
        assert_eq!(manager.summaries()[0].name, "a");
        assert_eq!(
            manager.summaries()[1].status,
            McpConnectionStatus::Disconnected
        );
        assert_eq!(manager.server_config("a").unwrap().kind_label(), "stdio");
    }

    #[test]
    fn model_definitions_are_fixed_without_connected_servers() {
        let manager = McpManager::new(PathBuf::from("/workspace"), BTreeMap::new());
        let definitions = manager.model_definitions();
        let names: Vec<&str> = definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();

        assert_eq!(
            names,
            vec![MCP_LIST_TOOL_NAME, MCP_SEARCH_TOOL_NAME, MCP_CALL_TOOL_NAME]
        );
        assert_eq!(definitions[0].permission, ToolPermission::Search);
        assert_eq!(definitions[1].permission, ToolPermission::Search);
        assert_eq!(definitions[2].permission, ToolPermission::Execute);
    }

    #[test]
    fn list_servers_returns_jsonl_records() {
        let manager = McpManager::new(
            PathBuf::from("/workspace"),
            BTreeMap::from([
                ("b".to_string(), stdio_config()),
                ("a".to_string(), stdio_config()),
            ]),
        );
        let result = manager
            .execute_list(&ToolCall {
                id: "call-1".into(),
                name: MCP_LIST_TOOL_NAME.into(),
                arguments: "{}".into(),
                thought_signature: None,
            })
            .unwrap();
        let lines: Vec<Value> = result
            .output
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()
            .unwrap();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["server"], "a");
        assert_eq!(lines[0]["status"], "disconnected");
        assert_eq!(lines[1]["server"], "b");
    }

    #[test]
    fn list_server_returns_empty_jsonl_when_disconnected() {
        let manager = McpManager::new(
            PathBuf::from("/workspace"),
            BTreeMap::from([("srv".to_string(), stdio_config())]),
        );
        let result = manager
            .execute_list(&ToolCall {
                id: "call-1".into(),
                name: MCP_LIST_TOOL_NAME.into(),
                arguments: r#"{"server":"srv"}"#.into(),
                thought_signature: None,
            })
            .unwrap();
        assert!(result.output.is_empty());
    }

    #[test]
    fn list_treats_an_empty_server_as_omitted_and_rejects_unknown_servers() {
        let manager = McpManager::new(PathBuf::from("/workspace"), BTreeMap::new());
        let empty = manager
            .execute_list(&ToolCall {
                id: "call-1".into(),
                name: MCP_LIST_TOOL_NAME.into(),
                arguments: r#"{"server":" "}"#.into(),
                thought_signature: None,
            })
            .unwrap();
        assert!(empty.output.is_empty());

        let unknown = manager
            .execute_list(&ToolCall {
                id: "call-2".into(),
                name: MCP_LIST_TOOL_NAME.into(),
                arguments: r#"{"server":"unknown"}"#.into(),
                thought_signature: None,
            })
            .unwrap_err();
        assert!(unknown.to_string().contains("not configured"));
    }

    #[test]
    fn search_empty_catalog_returns_empty_jsonl() {
        let manager = McpManager::new(PathBuf::from("/workspace"), BTreeMap::new());
        let result = manager
            .execute_search(&ToolCall {
                id: "call-1".into(),
                name: MCP_SEARCH_TOOL_NAME.into(),
                arguments: r#"{"query":"issue"}"#.into(),
                thought_signature: None,
            })
            .unwrap();
        assert!(result.output.is_empty());
    }

    #[test]
    fn search_requires_a_non_empty_query() {
        let manager = McpManager::new(PathBuf::from("/workspace"), BTreeMap::new());
        let missing = manager
            .execute_search(&ToolCall {
                id: "call-1".into(),
                name: MCP_SEARCH_TOOL_NAME.into(),
                arguments: "{}".into(),
                thought_signature: None,
            })
            .unwrap_err();
        assert!(
            missing
                .to_string()
                .contains("invalid arguments for mcp_search")
        );

        let empty = manager
            .execute_search(&ToolCall {
                id: "call-2".into(),
                name: MCP_SEARCH_TOOL_NAME.into(),
                arguments: r#"{"query":" "}"#.into(),
                thought_signature: None,
            })
            .unwrap_err();
        assert!(empty.to_string().contains("non-empty query"));
    }

    #[test]
    fn tool_records_are_single_jsonl_lines() {
        let record = tool_record(McpToolInfo {
            definition: tidev_llm::ToolDefinition {
                name: "mcp__srv__create_issue".into(),
                display_name: "Create issue".into(),
                description: "Create an issue\nwith a title".into(),
                parameters: json!({"type": "object", "properties": {"title": {"type": "string"}}}),
            },
            server_name: "srv".into(),
            tool_name: "create_issue".into(),
            read_only: false,
        });
        let line = jsonl_line(&record).unwrap();

        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), record);
    }

    #[test]
    fn mcp_call_rejects_non_object_arguments_before_permission_check() {
        let manager = McpManager::new(PathBuf::from("/workspace"), BTreeMap::new());
        let error = manager
            .can_execute_mcp_call(
                &ToolCall {
                    id: "call-1".into(),
                    name: MCP_CALL_TOOL_NAME.into(),
                    arguments: r#"{"server":"srv","tool":"tool","arguments":null}"#.into(),
                    thought_signature: None,
                },
                Mode::Build,
            )
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("arguments must be a JSON object")
        );
    }

    #[test]
    fn relative_stdio_cwd_is_resolved_for_agent() {
        let manager = McpManager::new(
            PathBuf::from("/workspace"),
            BTreeMap::from([("srv".to_string(), stdio_config())]),
        );
        let spec = manager.agent_registry().server_spec("srv").unwrap();
        assert!(matches!(spec, McpServerSpec::Stdio { .. }));
    }

    #[test]
    fn http_headers_round_trip_to_agent_spec() {
        let config = McpServerConfig::Http {
            url: "https://example.com/mcp".into(),
            headers: BTreeMap::from([("Authorization".into(), "Bearer token".into())]),
            disabled: true,
        };
        let spec = to_agent_spec(Path::new("/workspace"), &config);

        assert!(matches!(
            spec,
            McpServerSpec::Http { url, headers, disabled }
                if url == "https://example.com/mcp"
                    && headers.get("Authorization") == Some(&"Bearer token".to_string())
                    && disabled
        ));
    }

    #[test]
    fn relative_stdio_command_is_resolved_from_workspace_cwd() {
        let workspace = PathBuf::from("workspace");
        let command = resolve_stdio_command(&workspace, "bin/server", Some(".mcp"));
        assert_eq!(
            command,
            workspace
                .join(".mcp")
                .join("bin/server")
                .to_string_lossy()
                .into_owned()
        );
    }

    #[test]
    fn bare_stdio_command_is_left_for_path_resolution() {
        assert_eq!(
            resolve_stdio_command(Path::new("workspace"), "dbx-mcp-server", None),
            "dbx-mcp-server"
        );
    }
}
