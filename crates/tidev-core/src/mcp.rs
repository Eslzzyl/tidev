//! MCP (Model Context Protocol) server management.
//!
//! This module provides [`McpManager`] for connecting to MCP servers,
//! discovering their tools, and executing tool calls.  It supports:
//!
//! - **stdio** servers launched as child processes
//! - **HTTP** servers (POST-based)
//! - **SSE** servers (Server-Sent Events)
//!
//! Each server exposes a set of tools that are merged into the global
//! tool registry alongside built-in tidev tools.

use anyhow::{Context, Result, bail};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation, Tool as McpTool,
};
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransport;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::process::Command;

use tidev_config::mcp::McpServerConfig;
use tidev_llm::message::{MessageAttachment, ToolCall, ToolExecutionResult, ToolMetadata};
use tidev_llm::mode::SessionMode;
use tidev_tools::types::{ToolDefinition, ToolPermission};

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

type McpClient = RunningService<RoleClient, ClientInfo>;

// ---------------------------------------------------------------------------
// McpConnectionStatus
// ---------------------------------------------------------------------------

/// The connection status of an MCP server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Failed(String),
}

impl McpConnectionStatus {
    /// Short label for display.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Failed(_) => "failed",
        }
    }

    /// Extended detail (error message for Failed, `None` otherwise).
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Failed(message) => Some(message.as_str()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// McpServerSummary
// ---------------------------------------------------------------------------

/// Summary of an MCP server for display in the TUI.
#[derive(Clone, Debug)]
pub struct McpServerSummary {
    pub name: String,
    pub kind: String,
    pub status: McpConnectionStatus,
    pub tool_count: usize,
}

impl McpServerSummary {
    pub fn status_text(&self) -> String {
        match &self.status {
            McpConnectionStatus::Failed(message) => format!("failed: {message}"),
            status => status.label().to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct McpManager {
    inner: Arc<Mutex<McpManagerInner>>,
}

#[derive(Debug)]
struct McpManagerInner {
    workspace_root: PathBuf,
    servers: BTreeMap<String, McpServerState>,
}

#[derive(Debug)]
struct McpServerState {
    config: McpServerConfig,
    status: McpConnectionStatus,
    client: Option<McpClient>,
    tools: Vec<ToolDefinition>,
}

// ---------------------------------------------------------------------------
// McpManager — public API
// ---------------------------------------------------------------------------

impl McpManager {
    /// Create a new [`McpManager`] from a map of server configurations.
    ///
    /// All servers start in the [`McpConnectionStatus::Disconnected`] state.
    /// Call [`refresh_all`](Self::refresh_all) to connect.
    pub fn new(workspace_root: PathBuf, servers: BTreeMap<String, McpServerConfig>) -> Self {
        let servers = servers
            .into_iter()
            .map(|(name, config)| {
                (
                    name,
                    McpServerState {
                        config,
                        status: McpConnectionStatus::Disconnected,
                        client: None,
                        tools: Vec::new(),
                    },
                )
            })
            .collect();

        Self {
            inner: Arc::new(Mutex::new(McpManagerInner {
                workspace_root,
                servers,
            })),
        }
    }

    // ── Connection lifecycle ────────────────────────────────────────────

    /// Connect / refresh all configured servers (best-effort).
    pub async fn refresh_all(&self) -> Result<()> {
        let server_names = {
            let inner = self.inner.lock().unwrap();
            inner.servers.keys().cloned().collect::<Vec<_>>()
        };

        for name in server_names {
            if let Err(error) = self.refresh_server(&name).await {
                self.mark_failed(&name, error.to_string());
            }
        }

        Ok(())
    }

    /// Connect (or reconnect) a single server, discovering its tools.
    pub async fn refresh_server(&self, name: &str) -> Result<()> {
        let (config, existing_client) = {
            let mut inner = self.inner.lock().unwrap();
            let state = inner
                .servers
                .get_mut(name)
                .with_context(|| format!("unknown MCP server '{name}'"))?;
            state.status = McpConnectionStatus::Connecting;
            (state.config.clone(), state.client.take())
        };

        let client = match existing_client {
            Some(client) if !client.is_closed() => client,
            _ => Self::connect_client(&config, &self.inner).await?,
        };

        let tools = Self::load_tools(name, &client).await?;
        Self::store_connection(&self.inner, name, client, tools);
        Ok(())
    }

    /// Add or update a server configuration and (re)connect it.
    pub async fn upsert_server(&self, name: String, config: McpServerConfig) -> Result<()> {
        let existing_client = {
            let mut inner = self.inner.lock().unwrap();
            let state = inner
                .servers
                .entry(name.clone())
                .or_insert_with(|| McpServerState {
                    config: config.clone(),
                    status: McpConnectionStatus::Disconnected,
                    client: None,
                    tools: Vec::new(),
                });

            state.config = config;
            state.status = McpConnectionStatus::Disconnected;
            state.tools.clear();
            state.client.take()
        };

        if let Some(mut client) = existing_client {
            let _ = client.close().await;
        }

        self.refresh_server(&name).await
    }

    /// Remove a server and close its connection.
    pub async fn remove_server(&self, name: &str) -> Result<()> {
        let client = {
            let mut inner = self.inner.lock().unwrap();
            inner
                .servers
                .remove(name)
                .with_context(|| format!("unknown MCP server '{name}'"))?
                .client
        };

        if let Some(mut client) = client {
            let _ = client.close().await;
        }

        Ok(())
    }

    /// Return the configuration for a named server, if it exists.
    pub fn server_config(&self, name: &str) -> Option<McpServerConfig> {
        let inner = self.inner.lock().unwrap();
        inner.servers.get(name).map(|state| state.config.clone())
    }

    /// Check if a server with the given name exists.
    pub fn has_server(&self, name: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.servers.contains_key(name)
    }

    /// Disconnect a server without removing its configuration.
    pub async fn disconnect_server(&self, name: &str) -> Result<()> {
        let client = {
            let mut inner = self.inner.lock().unwrap();
            let state = inner
                .servers
                .get_mut(name)
                .with_context(|| format!("unknown MCP server '{name}'"))?;
            state.status = McpConnectionStatus::Disconnected;
            state.tools.clear();
            state.client.take()
        };

        if let Some(mut client) = client {
            let _ = client.close().await;
        }

        Ok(())
    }

    /// Toggle a server between connected and disconnected.
    pub async fn toggle_server(&self, name: &str) -> Result<()> {
        let status = {
            let inner = self.inner.lock().unwrap();
            let state = inner
                .servers
                .get(name)
                .with_context(|| format!("unknown MCP server '{name}'"))?;
            state.status.clone()
        };

        match status {
            McpConnectionStatus::Connected | McpConnectionStatus::Connecting => {
                self.disconnect_server(name).await
            }
            _ => self.refresh_server(name).await,
        }
    }

    // ── Queries ─────────────────────────────────────────────────────────

    /// Return summaries for all configured servers.
    pub fn summaries(&self) -> Vec<McpServerSummary> {
        let inner = self.inner.lock().unwrap();
        inner
            .servers
            .iter()
            .map(|(name, state)| McpServerSummary {
                name: name.clone(),
                kind: state.config.kind_label().to_string(),
                status: state.status.clone(),
                tool_count: state.tools.len(),
            })
            .collect()
    }

    /// Return tool definitions from all connected servers, filtered by mode.
    pub fn available_definitions(&self, mode: SessionMode) -> Vec<ToolDefinition> {
        let inner = self.inner.lock().unwrap();
        inner
            .servers
            .values()
            .filter(|state| matches!(state.status, McpConnectionStatus::Connected))
            .flat_map(|state| {
                state
                    .tools
                    .iter()
                    .filter(|definition| definition.permission.allowed_in_mode(mode))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Return all tool definitions from all connected servers (unfiltered).
    pub fn all_definitions(&self) -> Vec<ToolDefinition> {
        let inner = self.inner.lock().unwrap();
        inner
            .servers
            .values()
            .filter(|state| matches!(state.status, McpConnectionStatus::Connected))
            .flat_map(|state| state.tools.iter().cloned())
            .collect()
    }

    /// Look up a tool definition by its full name (mcp__server__tool).
    pub fn definition_for(&self, tool_name: &str) -> Option<ToolDefinition> {
        let inner = self.inner.lock().unwrap();
        inner
            .servers
            .values()
            .flat_map(|state| state.tools.iter())
            .find(|definition| definition.name == tool_name)
            .cloned()
    }

    // ── Permission helpers ──────────────────────────────────────────────

    pub fn can_execute(&self, tool_name: &str, mode: SessionMode) -> bool {
        self.definition_for(tool_name)
            .is_some_and(|definition| definition.permission.allowed_in_mode(mode))
    }

    // ── Execution ───────────────────────────────────────────────────────

    /// Execute an MCP tool call.
    pub async fn execute_call(&self, call: &ToolCall) -> Result<ToolExecutionResult> {
        let definition = self
            .definition_for(&call.name)
            .with_context(|| format!("unknown MCP tool '{}'", call.name))?;

        let target = definition.mcp_target().with_context(|| {
            format!("tool '{}' is not backed by an MCP server", definition.name)
        })?;
        let (server_name, tool_name) = (target.0.to_string(), target.1.to_string());

        let arguments = parse_arguments(&call.arguments)?;
        let request = CallToolRequestParams::new(tool_name.clone()).with_arguments(arguments);

        let client = {
            let mut inner = self.inner.lock().unwrap();
            let state = inner
                .servers
                .get_mut(server_name.as_str())
                .with_context(|| format!("unknown MCP server '{server_name}'"))?;
            state
                .client
                .take()
                .with_context(|| format!("MCP server '{server_name}' is not connected"))?
        };

        let result = match client.peer().call_tool(request).await {
            Ok(result) => result,
            Err(error) => {
                Self::restore_client(&self.inner, server_name.as_str(), client);
                return Err(error)
                    .with_context(|| format!("failed to call MCP tool '{tool_name}'"));
            }
        };

        Self::restore_client(&self.inner, server_name.as_str(), client);
        Ok(call_tool_result_data(&result, &tool_name))
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

impl McpManager {
    async fn connect_client(
        config: &McpServerConfig,
        inner: &Arc<Mutex<McpManagerInner>>,
    ) -> Result<McpClient> {
        let client_info = ClientInfo::new(
            ClientCapabilities::builder().build(),
            Implementation::new("tidev", env!("CARGO_PKG_VERSION")),
        );

        match config {
            McpServerConfig::Stdio {
                command,
                args,
                cwd,
                env,
            } => {
                let mut command = Command::new(command);
                command.args(args);

                if let Some(cwd) = cwd {
                    let resolved = Self::resolve_path_inner(inner, cwd);
                    command.current_dir(resolved);
                }

                for (key, value) in env {
                    command.env(key, value);
                }

                let transport = TokioChildProcess::new(command)
                    .context("failed to start stdio MCP server process")?;
                let client: McpClient = client_info
                    .serve(transport)
                    .await
                    .context("failed to connect to stdio MCP server")?;
                Ok(client)
            }
            McpServerConfig::Http { url } | McpServerConfig::Sse { url } => {
                let transport = StreamableHttpClientTransport::from_uri(url.as_str());
                let client: McpClient = client_info
                    .serve(transport)
                    .await
                    .context("failed to connect to HTTP/SSE MCP server")?;
                Ok(client)
            }
        }
    }

    async fn load_tools(server_name: &str, client: &McpClient) -> Result<Vec<ToolDefinition>> {
        let tools = client
            .peer()
            .list_all_tools()
            .await
            .with_context(|| format!("failed to list tools for MCP server '{server_name}'"))?;

        let mut definitions = Vec::new();
        for tool in tools {
            definitions.push(parse_tool(server_name, tool)?);
        }

        Ok(definitions)
    }

    fn resolve_path_inner(inner: &Arc<Mutex<McpManagerInner>>, value: &str) -> PathBuf {
        let path = Path::new(value);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            inner.lock().unwrap().workspace_root.join(path)
        }
    }

    fn store_connection(
        inner: &Arc<Mutex<McpManagerInner>>,
        name: &str,
        client: McpClient,
        tools: Vec<ToolDefinition>,
    ) {
        let mut inner = inner.lock().unwrap();
        if let Some(state) = inner.servers.get_mut(name) {
            state.client = Some(client);
            state.tools = tools;
            state.status = McpConnectionStatus::Connected;
        }
    }

    fn restore_client(inner: &Arc<Mutex<McpManagerInner>>, name: &str, client: McpClient) {
        let mut inner = inner.lock().unwrap();
        if let Some(state) = inner.servers.get_mut(name) {
            state.client = Some(client);
            state.status = McpConnectionStatus::Connected;
        }
    }

    fn mark_failed(&self, name: &str, error: String) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(state) = inner.servers.get_mut(name) {
            state.status = McpConnectionStatus::Failed(error);
            state.tools.clear();
            state.client = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Convert an [`rmcp`] tool into a [`ToolDefinition`].
fn parse_tool(server_name: &str, tool: McpTool) -> Result<ToolDefinition> {
    let annotations = tool.annotations.unwrap_or_default();
    let remote_tool_name = tool.name.to_string();
    let permission = match remote_tool_name.as_str() {
        "websearch" => ToolPermission::Search,
        "webfetch" => ToolPermission::Read,
        _ if annotations.read_only_hint.unwrap_or(false) => ToolPermission::Read,
        _ => ToolPermission::Execute,
    };

    let name = ToolDefinition::mcp_name(server_name, &remote_tool_name);
    let display_name = if let Some(title) = tool.title {
        if title.trim().is_empty() {
            format!("{server_name} / {remote_tool_name}")
        } else {
            title.to_string()
        }
    } else {
        format!("{server_name} / {remote_tool_name}")
    };

    let description = tool.description.unwrap_or_default().to_string();
    let parameters = Value::Object(tool.input_schema.as_ref().clone());

    Ok(ToolDefinition::mcp(
        name,
        display_name,
        description,
        parameters,
        permission,
        server_name.to_string(),
        remote_tool_name,
    ))
}

/// Parse JSON arguments string into a [`Map`].
fn parse_arguments(arguments: &str) -> Result<Map<String, Value>> {
    if arguments.trim().is_empty() {
        return Ok(Map::new());
    }

    let value: Value = serde_json::from_str(arguments)
        .with_context(|| "failed to parse MCP tool arguments as JSON")?;

    match value {
        Value::Object(map) => Ok(map),
        Value::Null => Ok(Map::new()),
        other => bail!("MCP tool arguments must be a JSON object, got {other}"),
    }
}

/// Convert an MCP [`CallToolResult`](rmcp::model::CallToolResult) into a [`ToolExecutionResult`].
fn call_tool_result_data(
    result: &rmcp::model::CallToolResult,
    tool_name: &str,
) -> ToolExecutionResult {
    if let Some(structured) = &result.structured_content {
        return ToolExecutionResult::new(
            serde_json::to_string_pretty(structured).unwrap_or_else(|_| structured.to_string()),
        );
    }

    let mut chunks = Vec::new();
    let mut attachments = Vec::new();
    for content in &result.content {
        if let Some(text) = content.as_text() {
            chunks.push(text.text.clone());
            continue;
        }

        if let Some(resource) = content.as_resource_link() {
            chunks.push(format!("[resource:{}]", resource.uri));
            continue;
        }

        if let Some(image) = content.as_image() {
            // Decode base64 data into raw bytes.
            use base64::Engine as _;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&image.data)
                .unwrap_or_else(|_| image.data.as_bytes().to_vec());
            let file_size = decoded.len() as u64;
            attachments.push(MessageAttachment::Image {
                filename: image_filename(tool_name, attachments.len(), &image.mime_type),
                mime: image.mime_type.clone(),
                data: decoded,
                file_size,
            });
            continue;
        }

        chunks.push(format!("[mcp-content:{:?}]", content));
    }

    let joined = chunks.join("\n");
    let output = if joined.trim().is_empty() {
        if result.is_error.unwrap_or(false) {
            "MCP tool returned an empty error".to_string()
        } else if !attachments.is_empty() {
            "MCP tool returned image attachment(s)".to_string()
        } else {
            "MCP tool returned no content".to_string()
        }
    } else {
        joined
    };

    ToolExecutionResult {
        output,
        attachments,
        metadata: ToolMetadata::default(),
        instruction_sources: Vec::new(),
        snapshot_hash: None,
        patch_files: None,
    }
}

/// Generate an image filename from a tool name and MIME type.
fn image_filename(tool_name: &str, index: usize, mime_type: &str) -> String {
    let extension = match mime_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        _ => "img",
    };

    let sanitized = tool_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();

    format!("{sanitized}-attachment-{}.{}", index + 1, extension)
}

// ---------------------------------------------------------------------------
// Test helpers (available to other crate modules under cfg(test))
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) fn insert_mock_tool(
    mgr: &McpManager,
    server_name: &str,
    server_config: McpServerConfig,
    tool: ToolDefinition,
) {
    let mut inner = mgr.inner.lock().unwrap();
    let state = inner
        .servers
        .entry(server_name.to_string())
        .or_insert_with(|| McpServerState {
            config: server_config,
            status: McpConnectionStatus::Disconnected,
            client: None,
            tools: Vec::new(),
        });
    state.status = McpConnectionStatus::Connected;
    state.client = None; // no real connection needed for tests
    state.tools.push(tool);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{CallToolResult, ContentBlock, Resource};

    // ── call_tool_result_data tests ────────────────────────────────────
    #[test]
    fn test_call_tool_result_text_only() {
        let result = CallToolResult::success(vec![ContentBlock::text("hello")]);
        let converted = call_tool_result_data(&result, "tool");
        assert_eq!(converted.output, "hello");
        assert!(converted.attachments.is_empty());
    }

    #[test]
    fn test_call_tool_result_error_empty() {
        let result = CallToolResult::error(vec![]);
        let converted = call_tool_result_data(&result, "tool");
        assert_eq!(converted.output, "MCP tool returned an empty error");
    }

    #[test]
    fn test_call_tool_result_no_content() {
        let result = CallToolResult::success(vec![]);
        let converted = call_tool_result_data(&result, "tool");
        assert_eq!(converted.output, "MCP tool returned no content");
    }

    #[test]
    fn test_call_tool_result_image_attachment_only() {
        let result = CallToolResult::success(vec![ContentBlock::image("aGVsbG8=", "image/png")]);
        let converted = call_tool_result_data(&result, "img-tool");
        assert_eq!(converted.output, "MCP tool returned image attachment(s)");
        assert_eq!(converted.attachments.len(), 1);
    }

    #[test]
    fn test_call_tool_result_mixed_text_and_resource() {
        let result = CallToolResult::success(vec![
            ContentBlock::text("Done"),
            ContentBlock::resource_link(Resource::new("file:///tmp/x.txt", "x.txt")),
        ]);
        let converted = call_tool_result_data(&result, "tool");
        assert!(converted.output.contains("Done"));
        assert!(converted.output.contains("file:///tmp/x.txt"));
    }

    // ── McpManager state tests (no real connections) ───────────────────

    fn make_stdio_config() -> McpServerConfig {
        McpServerConfig::Stdio {
            command: "node".into(),
            args: vec!["server.js".into()],
            cwd: None,
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn test_mcp_manager_new_empty() {
        let mgr = McpManager::new(PathBuf::from("/tmp"), BTreeMap::new());
        assert!(mgr.summaries().is_empty());
    }

    #[test]
    fn test_mcp_manager_new_with_config() {
        let mut servers = BTreeMap::new();
        servers.insert("srv1".into(), make_stdio_config());
        servers.insert("srv2".into(), make_stdio_config());
        let mgr = McpManager::new(PathBuf::from("/tmp"), servers);

        let summaries = mgr.summaries();
        assert_eq!(summaries.len(), 2);
        for s in &summaries {
            assert_eq!(s.status, McpConnectionStatus::Disconnected);
            assert_eq!(s.kind, "stdio");
            assert_eq!(s.tool_count, 0);
        }
        // Names should be sorted (BTreeMap).
        assert_eq!(summaries[0].name, "srv1");
        assert_eq!(summaries[1].name, "srv2");
    }

    #[test]
    fn test_mcp_manager_has_server() {
        let mut servers = BTreeMap::new();
        servers.insert("exists".into(), make_stdio_config());
        let mgr = McpManager::new(PathBuf::from("/tmp"), servers);

        assert!(mgr.has_server("exists"));
        assert!(!mgr.has_server("nonexistent"));
    }

    #[test]
    fn test_mcp_manager_server_config() {
        let mut servers = BTreeMap::new();
        servers.insert("srv".into(), make_stdio_config());
        let mgr = McpManager::new(PathBuf::from("/tmp"), servers);

        let cfg = mgr.server_config("srv");
        assert!(cfg.is_some());
        assert_eq!(cfg.unwrap().kind_label(), "stdio");

        assert!(mgr.server_config("nonexistent").is_none());
    }

    #[test]
    fn test_mcp_manager_remove_server() {
        let mut servers = BTreeMap::new();
        servers.insert("srv".into(), make_stdio_config());
        let mgr = McpManager::new(PathBuf::from("/tmp"), servers);

        assert_eq!(mgr.summaries().len(), 1);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(mgr.remove_server("srv")).unwrap();
        assert!(mgr.summaries().is_empty());
    }

    #[test]
    fn test_mcp_manager_remove_nonexistent_fails() {
        let mgr = McpManager::new(PathBuf::from("/tmp"), BTreeMap::new());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(mgr.remove_server("nope"));
        assert!(result.is_err());
    }

    #[test]
    fn test_mcp_manager_definitions_empty_when_disconnected() {
        let mgr = McpManager::new(PathBuf::from("/tmp"), BTreeMap::new());
        assert!(mgr.all_definitions().is_empty());
        assert!(mgr.definition_for("anything").is_none());
        assert!(mgr.available_definitions(SessionMode::Build).is_empty());
    }

    #[test]
    fn test_mcp_manager_definitions_with_mock_tool() {
        let mgr = McpManager::new(PathBuf::from("/tmp"), BTreeMap::new());
        let tool = ToolDefinition::new::<tidev_tools::types::ReadArgs>(
            "read",
            "Read a file",
            ToolPermission::Read,
        );

        insert_mock_tool(&mgr, "my-server", make_stdio_config(), tool);

        let all = mgr.all_definitions();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "read");

        let found = mgr.definition_for("read");
        assert!(found.is_some());
    }

    #[test]
    fn test_mcp_manager_definition_for_mcp_name() {
        let mgr = McpManager::new(PathBuf::from("/tmp"), BTreeMap::new());
        let tool = ToolDefinition::mcp(
            "mcp__srv__tool".into(),
            "Srv Tool".into(),
            "desc".into(),
            serde_json::json!({}),
            ToolPermission::Execute,
            "srv".into(),
            "tool".into(),
        );

        insert_mock_tool(&mgr, "srv", make_stdio_config(), tool);

        let found = mgr.definition_for("mcp__srv__tool");
        assert!(found.is_some());
        assert_eq!(found.unwrap().mcp_target(), Some(("srv", "tool")));
    }

    #[test]
    fn test_mcp_manager_available_definitions_filters_by_mode() {
        let mgr = McpManager::new(PathBuf::from("/tmp"), BTreeMap::new());

        let write_tool = ToolDefinition::new::<tidev_tools::types::WriteArgs>(
            "write",
            "Write file",
            ToolPermission::Write,
        );
        let read_tool = ToolDefinition::new::<tidev_tools::types::ReadArgs>(
            "read",
            "Read file",
            ToolPermission::Read,
        );

        insert_mock_tool(&mgr, "srv", make_stdio_config(), write_tool);
        insert_mock_tool(&mgr, "srv", make_stdio_config(), read_tool);

        let plan_tools = mgr.available_definitions(SessionMode::Plan);
        assert_eq!(plan_tools.len(), 1);
        assert_eq!(plan_tools[0].name, "read");

        let build_tools = mgr.available_definitions(SessionMode::Build);
        assert_eq!(build_tools.len(), 2);
    }

    #[test]
    fn test_mcp_manager_can_execute_for_unknown() {
        let mgr = McpManager::new(PathBuf::from("/tmp"), BTreeMap::new());
        assert!(!mgr.can_execute("nonexistent", SessionMode::Build));
    }

    #[test]
    fn test_mcp_server_summary_ordering() {
        let mut servers = BTreeMap::new();
        servers.insert("b-server".into(), make_stdio_config());
        servers.insert("a-server".into(), make_stdio_config());
        let mgr = McpManager::new(PathBuf::from("/tmp"), servers);

        let summaries = mgr.summaries();
        assert_eq!(summaries[0].name, "a-server");
        assert_eq!(summaries[1].name, "b-server");
    }
}
