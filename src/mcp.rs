use anyhow::{Context, Result, bail};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation, Tool as McpTool,
    ToolAnnotations,
};
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransport;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::process::Command;

use crate::config::mcp::McpServerConfig;
use crate::prompts::SessionMode;
use crate::session::ToolCall;
use crate::tooling::{ToolDefinition, ToolPermission};

type McpClient = RunningService<RoleClient, ClientInfo>;

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

impl McpManager {
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
            _ => self.connect_client(&config).await?,
        };

        let tools = self.load_tools(name, &client).await?;
        self.store_connection(name, client, tools);
        Ok(())
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
                    .filter(|definition| definition.permission.is_allowed_in(mode))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub fn definition_for(&self, tool_name: &str) -> Option<ToolDefinition> {
        let inner = self.inner.lock().unwrap();
        inner
            .servers
            .values()
            .flat_map(|state| state.tools.iter())
            .find(|definition| definition.name == tool_name)
            .cloned()
    }

    pub fn permission_key_for_call(&self, call: &ToolCall) -> String {
        if let Some(definition) = self.definition_for(&call.name) {
            return definition.permission_key();
        }

        call.name.clone()
    }

    pub fn permission_label_for_call(&self, call: &ToolCall) -> String {
        if let Some(definition) = self.definition_for(&call.name) {
            return definition.permission_label();
        }

        call.name.clone()
    }

    pub fn can_execute(&self, tool_name: &str, mode: SessionMode) -> bool {
        self.definition_for(tool_name)
            .is_some_and(|definition| definition.permission.is_allowed_in(mode))
    }

    pub async fn execute_call(&self, call: &ToolCall) -> Result<String> {
        let definition = self
            .definition_for(&call.name)
            .with_context(|| format!("unknown MCP tool '{}'", call.name))?;

        let (server_name, tool_name) = definition.mcp_target().with_context(|| {
            format!("tool '{}' is not backed by an MCP server", definition.name)
        })?;
        let server_name = server_name.to_string();
        let tool_name = tool_name.to_string();

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
                self.restore_client(server_name.as_str(), client);
                return Err(error)
                    .with_context(|| format!("failed to call MCP tool '{tool_name}'"));
            }
        };

        self.restore_client(server_name.as_str(), client);

        Ok(call_tool_result_text(&result))
    }

    async fn connect_client(&self, config: &McpServerConfig) -> Result<McpClient> {
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
                    command.current_dir(self.resolve_path(cwd));
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
            McpServerConfig::Http { url } => {
                let transport = StreamableHttpClientTransport::from_uri(url.as_str());
                let client: McpClient = client_info
                    .serve(transport)
                    .await
                    .context("failed to connect to HTTP MCP server")?;
                Ok(client)
            }
        }
    }

    async fn load_tools(
        &self,
        server_name: &str,
        client: &McpClient,
    ) -> Result<Vec<ToolDefinition>> {
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

    fn resolve_path(&self, value: &str) -> PathBuf {
        let path = Path::new(value);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.inner.lock().unwrap().workspace_root.join(path)
        }
    }

    fn store_connection(&self, name: &str, client: McpClient, tools: Vec<ToolDefinition>) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(state) = inner.servers.get_mut(name) {
            state.client = Some(client);
            state.tools = tools;
            state.status = McpConnectionStatus::Connected;
        }
    }

    fn restore_client(&self, name: &str, client: McpClient) {
        let mut inner = self.inner.lock().unwrap();
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

fn parse_tool(server_name: &str, tool: McpTool) -> Result<ToolDefinition> {
    let annotations = tool.annotations.unwrap_or_else(ToolAnnotations::default);
    let permission = if annotations.read_only_hint.unwrap_or(false) {
        ToolPermission::Read
    } else {
        ToolPermission::Execute
    };

    let remote_tool_name = tool.name.to_string();
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

fn call_tool_result_text(result: &rmcp::model::CallToolResult) -> String {
    if let Some(structured) = &result.structured_content {
        return serde_json::to_string_pretty(structured).unwrap_or_else(|_| structured.to_string());
    }

    let mut chunks = Vec::new();
    for content in &result.content {
        if let Some(text) = content.raw.as_text() {
            chunks.push(text.text.clone());
            continue;
        }

        if let Some(resource) = content.raw.as_resource_link() {
            chunks.push(format!("[resource:{}]", resource.uri));
            continue;
        }

        chunks.push(format!("[mcp-content:{:?}]", content.raw));
    }

    let joined = chunks.join("\n");
    if joined.trim().is_empty() {
        if result.is_error.unwrap_or(false) {
            "MCP tool returned an empty error".to_string()
        } else {
            "MCP tool returned no content".to_string()
        }
    } else {
        joined
    }
}
