//! Generic Model Context Protocol client and tool registry.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation, Tool as McpToolModel,
};
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransport;
use serde_json::{Map, Value};
use tokio::process::Command;

use tidev_llm::ToolDefinition;
use tidev_llm::message::{MessageAttachment, ToolCall, ToolExecutionResult, ToolMetadata};

use crate::{Tool, ToolContext};

type McpClient = RunningService<RoleClient, ClientInfo>;

/// Host-resolved MCP server connection parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpServerSpec {
    /// A subprocess communicating over stdio.
    Stdio {
        command: String,
        args: Vec<String>,
        cwd: Option<std::path::PathBuf>,
        env: BTreeMap<String, String>,
    },
    /// A streamable HTTP server.
    Http { url: String },
    /// An SSE server retained for compatibility with existing hosts.
    Sse { url: String },
}

impl McpServerSpec {
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Stdio { .. } => "stdio",
            Self::Http { .. } => "http",
            Self::Sse { .. } => "sse",
        }
    }
}

/// Connection state for an MCP server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Failed(String),
}

impl McpConnectionStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Failed(_) => "failed",
        }
    }

    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Failed(message) => Some(message.as_str()),
            _ => None,
        }
    }
}

/// A display-oriented snapshot of an MCP server.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Metadata for one discovered MCP tool.
#[derive(Clone, Debug)]
pub struct McpToolInfo {
    pub definition: ToolDefinition,
    pub server_name: String,
    pub tool_name: String,
    pub read_only: bool,
}

#[derive(Clone, Debug)]
pub struct McpRegistry {
    inner: Arc<Mutex<McpRegistryInner>>,
}

#[derive(Debug)]
struct McpRegistryInner {
    servers: BTreeMap<String, McpServerState>,
}

#[derive(Debug)]
struct McpServerState {
    spec: McpServerSpec,
    status: McpConnectionStatus,
    client: Option<McpClient>,
    tools: Vec<Arc<McpTool>>,
}

/// A discovered MCP tool implementing the generic agent tool contract.
#[derive(Debug)]
struct McpTool {
    registry: Weak<Mutex<McpRegistryInner>>,
    info: McpToolInfo,
}

impl McpRegistry {
    pub fn new(servers: BTreeMap<String, McpServerSpec>) -> Self {
        let servers = servers
            .into_iter()
            .map(|(name, spec)| {
                (
                    name,
                    McpServerState {
                        spec,
                        status: McpConnectionStatus::Disconnected,
                        client: None,
                        tools: Vec::new(),
                    },
                )
            })
            .collect();

        Self {
            inner: Arc::new(Mutex::new(McpRegistryInner { servers })),
        }
    }

    /// Connect or refresh all configured servers, retaining best-effort startup.
    pub async fn refresh_all(&self) -> Result<()> {
        let names = {
            let inner = self.inner.lock().unwrap();
            inner.servers.keys().cloned().collect::<Vec<_>>()
        };

        for name in names {
            if let Err(error) = self.refresh_server(&name).await {
                self.mark_failed(&name, error.to_string());
            }
        }
        Ok(())
    }

    /// Connect or reconnect one configured server and discover its tools.
    pub async fn refresh_server(&self, name: &str) -> Result<()> {
        let (spec, existing_client) = {
            let mut inner = self.inner.lock().unwrap();
            let state = inner
                .servers
                .get_mut(name)
                .with_context(|| format!("unknown MCP server '{name}'"))?;
            state.status = McpConnectionStatus::Connecting;
            (state.spec.clone(), state.client.take())
        };

        let client = match existing_client {
            Some(client) if !client.is_closed() => client,
            _ => match Self::connect_client(&spec).await {
                Ok(client) => client,
                Err(error) => {
                    self.mark_failed(name, error.to_string());
                    return Err(error);
                }
            },
        };
        let tools = match Self::load_tools(name, &client, &self.inner).await {
            Ok(tools) => tools,
            Err(error) => {
                let mut client = client;
                let _ = client.close().await;
                self.mark_failed(name, error.to_string());
                return Err(error);
            }
        };
        Self::store_connection(&self.inner, name, client, tools);
        Ok(())
    }

    /// Add or update a server specification and reconnect it.
    pub async fn upsert_server(&self, name: String, spec: McpServerSpec) -> Result<()> {
        let existing_client = {
            let mut inner = self.inner.lock().unwrap();
            let state = inner
                .servers
                .entry(name.clone())
                .or_insert_with(|| McpServerState {
                    spec: spec.clone(),
                    status: McpConnectionStatus::Disconnected,
                    client: None,
                    tools: Vec::new(),
                });
            state.spec = spec;
            state.status = McpConnectionStatus::Disconnected;
            state.tools.clear();
            state.client.take()
        };

        if let Some(mut client) = existing_client {
            let _ = client.close().await;
        }
        self.refresh_server(&name).await
    }

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

    pub fn server_spec(&self, name: &str) -> Option<McpServerSpec> {
        let inner = self.inner.lock().unwrap();
        inner.servers.get(name).map(|state| state.spec.clone())
    }

    pub fn has_server(&self, name: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.servers.contains_key(name)
    }

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

    pub async fn toggle_server(&self, name: &str) -> Result<()> {
        let status = {
            let inner = self.inner.lock().unwrap();
            inner
                .servers
                .get(name)
                .with_context(|| format!("unknown MCP server '{name}'"))?
                .status
                .clone()
        };

        match status {
            McpConnectionStatus::Connected | McpConnectionStatus::Connecting => {
                self.disconnect_server(name).await
            }
            _ => self.refresh_server(name).await,
        }
    }

    pub fn summaries(&self) -> Vec<McpServerSummary> {
        let inner = self.inner.lock().unwrap();
        inner
            .servers
            .iter()
            .map(|(name, state)| McpServerSummary {
                name: name.clone(),
                kind: state.spec.kind_label().to_string(),
                status: state.status.clone(),
                tool_count: state.tools.len(),
            })
            .collect()
    }

    pub fn all_tools(&self) -> Vec<McpToolInfo> {
        let inner = self.inner.lock().unwrap();
        inner
            .servers
            .values()
            .filter(|state| matches!(state.status, McpConnectionStatus::Connected))
            .flat_map(|state| state.tools.iter().map(|tool| tool.info.clone()))
            .collect()
    }

    /// Return connected MCP tools as generic agent tool implementations.
    ///
    /// Hosts can register these values in [`crate::ToolRegistry`] to expose
    /// MCP and built-in tools through one runtime dispatch path.
    pub fn tool_implementations(&self) -> Vec<Arc<dyn Tool>> {
        let inner = self.inner.lock().unwrap();
        inner
            .servers
            .values()
            .filter(|state| matches!(state.status, McpConnectionStatus::Connected))
            .flat_map(|state| {
                state
                    .tools
                    .iter()
                    .cloned()
                    .map(|tool| tool as Arc<dyn Tool>)
            })
            .collect()
    }

    pub fn all_definitions(&self) -> Vec<ToolDefinition> {
        self.all_tools()
            .into_iter()
            .map(|tool| tool.definition)
            .collect()
    }

    pub fn available_definitions(&self, read_only: bool) -> Vec<ToolDefinition> {
        self.all_tools()
            .into_iter()
            .filter(|tool| !read_only || tool.read_only)
            .map(|tool| tool.definition)
            .collect()
    }

    pub fn definition_for(&self, tool_name: &str) -> Option<ToolDefinition> {
        self.tool_info_for(tool_name).map(|tool| tool.definition)
    }

    pub fn tool_info_for(&self, tool_name: &str) -> Option<McpToolInfo> {
        let inner = self.inner.lock().unwrap();
        inner
            .servers
            .values()
            .filter(|state| matches!(state.status, McpConnectionStatus::Connected))
            .flat_map(|state| state.tools.iter())
            .find(|tool| tool.info.definition.name == tool_name)
            .map(|tool| tool.info.clone())
    }

    pub fn read_only_for(&self, tool_name: &str) -> Option<bool> {
        self.tool_info_for(tool_name).map(|tool| tool.read_only)
    }

    pub async fn execute_call(&self, call: &ToolCall) -> Result<ToolExecutionResult> {
        let tool = self
            .find_tool(&call.name)
            .with_context(|| format!("unknown MCP tool '{}'", call.name))?;
        let arguments = parse_arguments(&call.arguments)?;
        tool.execute_arguments(arguments).await
    }

    fn find_tool(&self, tool_name: &str) -> Option<Arc<McpTool>> {
        let inner = self.inner.lock().unwrap();
        inner
            .servers
            .values()
            .filter(|state| matches!(state.status, McpConnectionStatus::Connected))
            .flat_map(|state| state.tools.iter())
            .find(|tool| tool.info.definition.name == tool_name)
            .cloned()
    }

    async fn connect_client(spec: &McpServerSpec) -> Result<McpClient> {
        let client_info = ClientInfo::new(
            ClientCapabilities::builder().build(),
            Implementation::new("tidev", env!("CARGO_PKG_VERSION")),
        );

        match spec {
            McpServerSpec::Stdio {
                command,
                args,
                cwd,
                env,
            } => {
                let mut command = Command::new(command);
                command.args(args);
                if let Some(cwd) = cwd {
                    command.current_dir(cwd);
                }
                for (key, value) in env {
                    command.env(key, value);
                }
                let transport = TokioChildProcess::new(command)
                    .context("failed to start stdio MCP server process")?;
                client_info
                    .serve(transport)
                    .await
                    .context("failed to connect to stdio MCP server")
            }
            McpServerSpec::Http { url } | McpServerSpec::Sse { url } => {
                let transport = StreamableHttpClientTransport::from_uri(url.as_str());
                client_info
                    .serve(transport)
                    .await
                    .context("failed to connect to HTTP MCP server")
            }
        }
    }

    async fn load_tools(
        server_name: &str,
        client: &McpClient,
        registry: &Arc<Mutex<McpRegistryInner>>,
    ) -> Result<Vec<Arc<McpTool>>> {
        let models = client
            .peer()
            .list_all_tools()
            .await
            .with_context(|| format!("failed to list tools for MCP server '{server_name}'"))?;

        models
            .into_iter()
            .map(|model| {
                let info = parse_tool(server_name, model)?;
                Ok(Arc::new(McpTool {
                    registry: Arc::downgrade(registry),
                    info,
                }))
            })
            .collect()
    }

    fn store_connection(
        inner: &Arc<Mutex<McpRegistryInner>>,
        name: &str,
        client: McpClient,
        tools: Vec<Arc<McpTool>>,
    ) {
        let mut inner = inner.lock().unwrap();
        if let Some(state) = inner.servers.get_mut(name) {
            state.client = Some(client);
            state.tools = tools;
            state.status = McpConnectionStatus::Connected;
        }
    }

    async fn restore_client(inner: &Arc<Mutex<McpRegistryInner>>, name: &str, client: McpClient) {
        let mut client = Some(client);
        {
            let mut inner = inner.lock().unwrap();
            if let Some(state) = inner.servers.get_mut(name).filter(|state| {
                matches!(state.status, McpConnectionStatus::Connected) && state.client.is_none()
            }) {
                state.client = client.take();
            }
        }
        if let Some(mut client) = client {
            let _ = client.close().await;
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

impl McpTool {
    async fn execute_arguments(
        &self,
        arguments: Map<String, Value>,
    ) -> Result<ToolExecutionResult> {
        let inner = self
            .registry
            .upgrade()
            .context("MCP registry is no longer available")?;
        let request =
            CallToolRequestParams::new(self.info.tool_name.clone()).with_arguments(arguments);

        let client = {
            let mut registry = inner.lock().unwrap();
            let state = registry
                .servers
                .get_mut(&self.info.server_name)
                .with_context(|| format!("unknown MCP server '{}'", self.info.server_name))?;
            state.client.take().with_context(|| {
                format!("MCP server '{}' is not connected", self.info.server_name)
            })?
        };

        let result = match client.peer().call_tool(request).await {
            Ok(result) => result,
            Err(error) => {
                McpRegistry::restore_client(&inner, &self.info.server_name, client).await;
                return Err(error)
                    .with_context(|| format!("failed to call MCP tool '{}'", self.info.tool_name));
            }
        };

        McpRegistry::restore_client(&inner, &self.info.server_name, client).await;
        Ok(call_tool_result_data(&result, &self.info.tool_name))
    }
}

#[async_trait]
impl Tool for McpTool {
    fn definition(&self) -> ToolDefinition {
        self.info.definition.clone()
    }

    fn read_only(&self) -> bool {
        self.info.read_only
    }

    async fn execute(
        &self,
        args: Value,
        _context: &dyn ToolContext,
    ) -> Result<ToolExecutionResult> {
        match args {
            Value::Object(arguments) => self.execute_arguments(arguments).await,
            Value::Null => self.execute_arguments(Map::new()).await,
            other => bail!("MCP tool arguments must be a JSON object, got {other}"),
        }
    }
}

fn parse_tool(server_name: &str, tool: McpToolModel) -> Result<McpToolInfo> {
    let annotations = tool.annotations.unwrap_or_default();
    let tool_name = tool.name.to_string();
    let read_only = match tool_name.as_str() {
        "websearch" | "webfetch" => true,
        _ => annotations.read_only_hint.unwrap_or(false),
    };
    let name = mcp_name(server_name, &tool_name);
    let display_name = tool
        .title
        .filter(|title| !title.trim().is_empty())
        .map(|title| title.to_string())
        .unwrap_or_else(|| format!("{server_name} / {tool_name}"));
    let description = tool.description.unwrap_or_default().to_string();
    let definition = ToolDefinition {
        name,
        display_name,
        description,
        parameters: Value::Object(tool.input_schema.as_ref().clone()),
    };

    Ok(McpToolInfo {
        definition,
        server_name: server_name.to_string(),
        tool_name,
        read_only,
    })
}

fn mcp_name(server_name: &str, tool_name: &str) -> String {
    format!(
        "mcp__{}__{}",
        sanitize_mcp_name(server_name),
        sanitize_mcp_name(tool_name)
    )
}

fn sanitize_mcp_name(value: &str) -> String {
    let mut sanitized = String::new();
    let mut last_was_separator = false;
    for ch in value.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if matches!(ch, '-' | '_') {
            Some(ch)
        } else {
            None
        };
        match mapped {
            Some(ch) => {
                sanitized.push(ch);
                last_was_separator = false;
            }
            None if !last_was_separator => {
                sanitized.push('_');
                last_was_separator = true;
            }
            None => {}
        }
    }
    if sanitized.trim_matches('_').is_empty() {
        "mcp".to_string()
    } else {
        sanitized.trim_matches('_').to_string()
    }
}

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
        } else if let Some(resource) = content.as_resource_link() {
            chunks.push(format!("[resource:{}]", resource.uri));
        } else if let Some(image) = content.as_image() {
            use base64::Engine as _;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&image.data)
                .unwrap_or_else(|_| image.data.as_bytes().to_vec());
            attachments.push(MessageAttachment::Image {
                filename: image_filename(tool_name, attachments.len(), &image.mime_type),
                mime: image.mime_type.clone(),
                file_size: decoded.len() as u64,
                data: decoded,
            });
        } else {
            chunks.push(format!("[mcp-content:{content:?}]"));
        }
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
    }
}

fn image_filename(tool_name: &str, index: usize, mime_type: &str) -> String {
    let extension = match mime_type {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        _ => "img",
    };
    let sanitized = tool_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("{sanitized}-attachment-{}.{}", index + 1, extension)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{CallToolResult, ContentBlock, Resource};

    #[test]
    fn result_text_is_preserved() {
        let result = CallToolResult::success(vec![ContentBlock::text("hello")]);
        assert_eq!(call_tool_result_data(&result, "tool").output, "hello");
    }

    #[test]
    fn result_empty_error_has_legacy_message() {
        let result = CallToolResult::error(vec![]);
        assert_eq!(
            call_tool_result_data(&result, "tool").output,
            "MCP tool returned an empty error"
        );
    }

    #[test]
    fn result_empty_success_has_legacy_message() {
        let result = CallToolResult::success(vec![]);
        assert_eq!(
            call_tool_result_data(&result, "tool").output,
            "MCP tool returned no content"
        );
    }

    #[test]
    fn result_image_is_converted_to_attachment() {
        let result = CallToolResult::success(vec![ContentBlock::image("aGVsbG8=", "image/png")]);
        let converted = call_tool_result_data(&result, "img-tool");
        assert_eq!(converted.output, "MCP tool returned image attachment(s)");
        assert_eq!(converted.attachments.len(), 1);
    }

    #[test]
    fn result_resource_link_is_text() {
        let result = CallToolResult::success(vec![ContentBlock::resource_link(Resource::new(
            "file:///tmp/x.txt",
            "x.txt",
        ))]);
        assert_eq!(
            call_tool_result_data(&result, "tool").output,
            "[resource:file:///tmp/x.txt]"
        );
    }

    #[test]
    fn mcp_names_match_host_sanitization() {
        assert_eq!(
            mcp_name("My Server", "Read.File"),
            "mcp__my_server__read_file"
        );
        assert_eq!(mcp_name("!!!", "???"), "mcp__mcp__mcp");
    }

    #[test]
    fn disconnected_registry_has_no_tools() {
        let registry = McpRegistry::new(BTreeMap::new());
        assert!(registry.summaries().is_empty());
        assert!(registry.all_definitions().is_empty());
        assert!(registry.tool_implementations().is_empty());
        assert!(registry.definition_for("missing").is_none());
    }

    #[test]
    fn server_summaries_are_sorted_and_disconnected() {
        let spec = McpServerSpec::Stdio {
            command: "node".into(),
            args: vec!["server.js".into()],
            cwd: None,
            env: BTreeMap::new(),
        };
        let registry = McpRegistry::new(BTreeMap::from([
            ("b".to_string(), spec.clone()),
            ("a".to_string(), spec),
        ]));
        let summaries = registry.summaries();
        assert_eq!(summaries[0].name, "a");
        assert_eq!(summaries[1].status, McpConnectionStatus::Disconnected);
    }

    #[tokio::test]
    async fn refresh_failure_marks_server_failed() {
        let registry = McpRegistry::new(BTreeMap::from([(
            "broken".to_string(),
            McpServerSpec::Stdio {
                command: "/definitely/missing/tidev-mcp-server".into(),
                args: Vec::new(),
                cwd: None,
                env: BTreeMap::new(),
            },
        )]));

        assert!(registry.refresh_server("broken").await.is_err());
        let summaries = registry.summaries();
        assert!(matches!(
            summaries[0].status,
            McpConnectionStatus::Failed(_)
        ));
        assert_eq!(summaries[0].tool_count, 0);
        assert!(registry.all_definitions().is_empty());
    }
}
