//! Generic Model Context Protocol client and tool registry.

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt::{Display, Formatter};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use futures_util::StreamExt;
use http::{HeaderMap, HeaderName, HeaderValue, header};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation, Tool as McpToolModel,
};
use rmcp::service::{RoleClient, RunningService, RxJsonRpcMessage, ServiceExt, TxJsonRpcMessage};
use rmcp::transport::Transport;
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransport;
use serde_json::{Map, Value};
use sse_stream::SseStream;
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;

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
        disabled: bool,
    },
    /// A streamable HTTP server.
    Http {
        url: String,
        headers: BTreeMap<String, String>,
        disabled: bool,
    },
    /// A legacy SSE server using a GET stream and a separate POST message endpoint.
    Sse {
        url: String,
        headers: BTreeMap<String, String>,
        disabled: bool,
    },
}

impl McpServerSpec {
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Stdio { .. } => "stdio",
            Self::Http { .. } => "http",
            Self::Sse { .. } => "sse",
        }
    }

    pub fn is_disabled(&self) -> bool {
        match self {
            Self::Stdio { disabled, .. }
            | Self::Http { disabled, .. }
            | Self::Sse { disabled, .. } => *disabled,
        }
    }

    pub fn set_disabled(&mut self, is_disabled: bool) {
        match self {
            Self::Stdio { disabled, .. }
            | Self::Http { disabled, .. }
            | Self::Sse { disabled, .. } => *disabled = is_disabled,
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
    pub disabled: bool,
}

impl McpServerSummary {
    pub fn status_text(&self) -> String {
        if self.disabled {
            return "disabled".to_string();
        }
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
    disabled: bool,
}

/// A discovered MCP tool implementing the generic agent tool contract.
#[derive(Debug)]
struct McpTool {
    registry: Weak<Mutex<McpRegistryInner>>,
    info: McpToolInfo,
}

const LEGACY_SSE_CHANNEL_CAPACITY: usize = 64;
const LEGACY_SSE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
struct LegacySseTransport {
    shared: Arc<LegacySseShared>,
    messages: mpsc::Receiver<RxJsonRpcMessage<RoleClient>>,
    cancellation: CancellationToken,
    reader: Option<JoinHandle<()>>,
}

#[derive(Debug)]
struct LegacySseShared {
    client: reqwest::Client,
    endpoint: Url,
    headers: HeaderMap,
    closed: AtomicBool,
}

#[derive(Debug)]
struct LegacySseError(anyhow::Error);

impl Display for LegacySseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl StdError for LegacySseError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0.source()
    }
}

impl From<anyhow::Error> for LegacySseError {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}

impl LegacySseTransport {
    async fn connect(url: &str, headers: HeaderMap) -> Result<Self> {
        let base_url =
            Url::parse(url).with_context(|| format!("invalid legacy SSE MCP URL '{url}'"))?;
        let client = reqwest::Client::new();
        let mut get_headers = headers.clone();
        if !get_headers.contains_key(header::ACCEPT) {
            get_headers.insert(
                header::ACCEPT,
                HeaderValue::from_static("text/event-stream"),
            );
        }
        let response = client
            .get(base_url.clone())
            .headers(get_headers)
            .send()
            .await
            .context("failed to open legacy SSE MCP stream")?
            .error_for_status()
            .context("legacy SSE MCP stream returned an error status")?;

        let (message_tx, message_rx) = mpsc::channel(LEGACY_SSE_CHANNEL_CAPACITY);
        let (endpoint_tx, endpoint_rx) = oneshot::channel();
        let cancellation = CancellationToken::new();
        let reader_cancellation = cancellation.clone();
        let reader = tokio::spawn(async move {
            let stream = response.bytes_stream();
            if let Err(error) = run_legacy_sse_reader(
                stream,
                base_url,
                endpoint_tx,
                message_tx,
                reader_cancellation,
            )
            .await
            {
                log::warn!("legacy SSE MCP reader stopped: {error:#}");
            }
        });

        let endpoint = match tokio::time::timeout(LEGACY_SSE_CONNECT_TIMEOUT, endpoint_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(anyhow::anyhow!("legacy SSE endpoint handshake was closed")),
            Err(error) => Err(anyhow::Error::new(error)
                .context("timed out waiting for the legacy SSE endpoint event")),
        };
        let endpoint = match endpoint {
            Ok(endpoint) => endpoint,
            Err(error) => {
                cancellation.cancel();
                let _ = reader.await;
                return Err(error);
            }
        };

        Ok(Self {
            shared: Arc::new(LegacySseShared {
                client,
                endpoint,
                headers,
                closed: AtomicBool::new(false),
            }),
            messages: message_rx,
            cancellation,
            reader: Some(reader),
        })
    }
}

async fn run_legacy_sse_reader<S, D, E>(
    stream: S,
    base_url: Url,
    endpoint_tx: oneshot::Sender<Result<Url>>,
    message_tx: mpsc::Sender<RxJsonRpcMessage<RoleClient>>,
    cancellation: CancellationToken,
) -> Result<()>
where
    S: futures_util::Stream<Item = Result<D, E>>,
    D: tokio_util::bytes::Buf,
    E: StdError + Send + Sync + 'static,
{
    let mut stream = Box::pin(SseStream::from_bytes_stream(stream));
    let mut endpoint_tx = Some(endpoint_tx);

    while let Some(event) = tokio::select! {
        _ = cancellation.cancelled() => return Ok(()),
        event = stream.next() => event,
    } {
        let event = event.context("failed to parse a legacy SSE event")?;
        match event.event.as_deref() {
            Some("endpoint") => {
                let data = event
                    .data
                    .as_deref()
                    .map(str::trim)
                    .filter(|data| !data.is_empty())
                    .context("legacy SSE endpoint event has no URL")?;
                let result = base_url
                    .join(data)
                    .context("legacy SSE endpoint event contains an invalid URL")?;
                if let Some(sender) = endpoint_tx.take() {
                    let _ = sender.send(Ok(result));
                }
            }
            None | Some("message") => {
                if endpoint_tx.is_some() {
                    continue;
                }
                let Some(data) = event.data else {
                    continue;
                };
                let message = serde_json::from_str(&data)
                    .context("legacy SSE message event contains invalid JSON-RPC")?;
                message_tx
                    .send(message)
                    .await
                    .context("legacy SSE message receiver was closed")?;
            }
            Some(_) => continue,
        }
    }

    if let Some(sender) = endpoint_tx {
        let _ = sender.send(Err(anyhow::anyhow!(
            "legacy SSE stream closed before the endpoint event"
        )));
    }
    Ok(())
}

impl Transport<RoleClient> for LegacySseTransport {
    type Error = LegacySseError;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let shared = self.shared.clone();
        async move {
            if shared.closed.load(Ordering::Acquire) {
                return Err(LegacySseError(anyhow::anyhow!(
                    "legacy SSE MCP transport is closed"
                )));
            }
            let body =
                serde_json::to_vec(&item).context("failed to encode MCP JSON-RPC message")?;
            let mut headers = shared.headers.clone();
            if !headers.contains_key(header::ACCEPT) {
                headers.insert(
                    header::ACCEPT,
                    HeaderValue::from_static("application/json, text/event-stream"),
                );
            }
            if !headers.contains_key(header::CONTENT_TYPE) {
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                );
            }
            let response = shared
                .client
                .post(shared.endpoint.clone())
                .headers(headers)
                .body(body)
                .send()
                .await
                .context("failed to POST a legacy SSE MCP message")?;
            if !response.status().is_success() {
                return Err(LegacySseError(anyhow::anyhow!(
                    "legacy SSE MCP message endpoint returned {}",
                    response.status()
                )));
            }
            Ok(())
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        self.messages.recv().await
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        if self.shared.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.cancellation.cancel();
        if let Some(reader) = self.reader.take() {
            reader.await.context("legacy SSE MCP reader task failed")?;
        }
        Ok(())
    }
}

impl McpRegistry {
    pub fn new(servers: BTreeMap<String, McpServerSpec>) -> Self {
        let servers = servers
            .into_iter()
            .map(|(name, spec)| {
                let disabled = spec.is_disabled();
                (
                    name,
                    McpServerState {
                        spec,
                        status: McpConnectionStatus::Disconnected,
                        client: None,
                        tools: Vec::new(),
                        disabled,
                    },
                )
            })
            .collect();

        Self {
            inner: Arc::new(Mutex::new(McpRegistryInner { servers })),
        }
    }

    /// Whether any MCP server is currently connecting.
    pub fn has_connecting(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner
            .servers
            .values()
            .any(|state| matches!(state.status, McpConnectionStatus::Connecting))
    }

    /// Wait until all MCP servers currently in `Connecting` state transition
    /// to `Connected`, `Failed`, or `Disconnected` (or until `timeout` expires).
    pub async fn wait_until_ready(&self, timeout: Duration) -> Result<()> {
        if !self.has_connecting() {
            return Ok(());
        }

        let start = tokio::time::Instant::now();
        let interval = Duration::from_millis(50);

        while start.elapsed() < timeout {
            if !self.has_connecting() {
                return Ok(());
            }
            tokio::time::sleep(interval).await;
        }

        Ok(())
    }

    /// Connect or refresh all configured servers concurrently, retaining best-effort startup.
    /// Servers marked as disabled are skipped.
    pub async fn refresh_all(&self) -> Result<()> {
        let names = {
            let inner = self.inner.lock().unwrap();
            inner
                .servers
                .iter()
                .filter(|(_, state)| !state.disabled)
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>()
        };

        let futures = names.into_iter().map(|name| async move {
            if let Err(error) = self.refresh_server(&name).await {
                self.mark_failed(&name, error.to_string());
            }
        });
        futures_util::future::join_all(futures).await;
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
            state.disabled = false;
            state.spec.set_disabled(false);
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

    /// Add or update a server specification and reconnect it if enabled.
    pub async fn upsert_server(&self, name: String, spec: McpServerSpec) -> Result<()> {
        let disabled = spec.is_disabled();
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
                    disabled,
                });
            state.spec = spec;
            state.disabled = disabled;
            state.status = McpConnectionStatus::Disconnected;
            state.tools.clear();
            state.client.take()
        };

        if let Some(mut client) = existing_client {
            let _ = client.close().await;
        }

        if !disabled {
            self.refresh_server(&name).await
        } else {
            Ok(())
        }
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
                disabled: state.disabled,
            })
            .collect()
    }

    pub fn all_tools(&self) -> Vec<McpToolInfo> {
        let inner = self.inner.lock().unwrap();
        let mut tools: Vec<McpToolInfo> = inner
            .servers
            .values()
            .filter(|state| matches!(state.status, McpConnectionStatus::Connected))
            .flat_map(|state| state.tools.iter().map(|tool| tool.info.clone()))
            .collect();
        tools.sort_by(|a, b| a.definition.name.cmp(&b.definition.name));
        tools
    }

    /// Return connected MCP tools as generic agent tool implementations.
    ///
    /// Hosts can register these values in [`crate::ToolRegistry`] to expose
    /// MCP and built-in tools through one runtime dispatch path.
    pub fn tool_implementations(&self) -> Vec<Arc<dyn Tool>> {
        let inner = self.inner.lock().unwrap();
        let mut tools: Vec<Arc<McpTool>> = inner
            .servers
            .values()
            .filter(|state| matches!(state.status, McpConnectionStatus::Connected))
            .flat_map(|state| state.tools.iter().cloned())
            .collect();
        tools.sort_by(|a, b| a.info.definition.name.cmp(&b.info.definition.name));
        tools
            .into_iter()
            .map(|tool| tool as Arc<dyn Tool>)
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
                ..
            } => {
                let mut command = Command::new(command);
                command.args(args);
                if let Some(cwd) = cwd {
                    command.current_dir(cwd);
                }
                for (key, value) in env {
                    command.env(key, value);
                }
                let (transport, _) = TokioChildProcess::builder(command)
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .context("failed to start stdio MCP server process")?;
                client_info
                    .serve(transport)
                    .await
                    .context("failed to connect to stdio MCP server")
            }
            McpServerSpec::Http { url, headers, .. } => {
                let custom_headers = Self::to_http_headers(headers)?;
                let transport = StreamableHttpClientTransport::from_config(
                    rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(url.as_str())
                        .custom_headers(custom_headers),
                );
                client_info
                    .serve(transport)
                    .await
                    .context("failed to connect to HTTP MCP server")
            }
            McpServerSpec::Sse { url, headers, .. } => {
                let custom_headers = Self::to_http_headers(headers)?;
                let custom_headers = Self::to_reqwest_headers(custom_headers);
                let transport = LegacySseTransport::connect(url, custom_headers).await?;
                client_info
                    .serve(transport)
                    .await
                    .context("failed to connect to legacy SSE MCP server")
            }
        }
    }

    fn to_http_headers(
        headers: &BTreeMap<String, String>,
    ) -> Result<std::collections::HashMap<HeaderName, HeaderValue>> {
        headers
            .iter()
            .map(|(name, value)| {
                let header_name = HeaderName::from_bytes(name.as_bytes())
                    .with_context(|| format!("invalid MCP HTTP header name '{name}'"))?;
                let header_value = HeaderValue::from_str(value)
                    .with_context(|| format!("invalid MCP HTTP header value for '{name}'"))?;
                Ok((header_name, header_value))
            })
            .collect()
    }

    fn to_reqwest_headers(
        headers: std::collections::HashMap<HeaderName, HeaderValue>,
    ) -> HeaderMap {
        headers.into_iter().collect()
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
    fn http_headers_are_converted_for_rmcp() {
        let headers = BTreeMap::from([
            ("Authorization".to_string(), "Bearer token".to_string()),
            ("X-Trace".to_string(), "trace-1".to_string()),
        ]);
        let converted = McpRegistry::to_http_headers(&headers).unwrap();

        assert_eq!(
            converted
                .get(&HeaderName::from_static("authorization"))
                .unwrap(),
            "Bearer token"
        );
        assert_eq!(
            converted.get(&HeaderName::from_static("x-trace")).unwrap(),
            "trace-1"
        );
    }

    #[test]
    fn invalid_http_headers_are_rejected_before_connecting() {
        let headers = BTreeMap::from([("invalid header".to_string(), "value".to_string())]);
        assert!(McpRegistry::to_http_headers(&headers).is_err());
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
            disabled: false,
        };
        let registry = McpRegistry::new(BTreeMap::from([
            ("b".to_string(), spec.clone()),
            ("a".to_string(), spec),
        ]));
        let summaries = registry.summaries();
        assert_eq!(summaries[0].name, "a");
        assert_eq!(summaries[1].status, McpConnectionStatus::Disconnected);
        assert!(!summaries[0].disabled);
    }

    #[tokio::test]
    async fn refresh_failure_marks_server_failed() {
        let registry = McpRegistry::new(BTreeMap::from([(
            "broken".to_string(),
            McpServerSpec::Stdio {
                command: "/definitely/missing-mcp-server".into(),
                args: Vec::new(),
                cwd: None,
                env: BTreeMap::new(),
                disabled: false,
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

    #[tokio::test]
    async fn wait_until_ready_returns_immediately_when_no_connecting() {
        let registry = McpRegistry::new(BTreeMap::new());
        assert!(!registry.has_connecting());
        assert!(
            registry
                .wait_until_ready(Duration::from_millis(50))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn refresh_all_skips_disabled_servers() {
        let registry = McpRegistry::new(BTreeMap::from([
            (
                "disabled_broken".to_string(),
                McpServerSpec::Stdio {
                    command: "/definitely/missing-1".into(),
                    args: Vec::new(),
                    cwd: None,
                    env: BTreeMap::new(),
                    disabled: true,
                },
            ),
            (
                "enabled_broken".to_string(),
                McpServerSpec::Stdio {
                    command: "/definitely/missing-2".into(),
                    args: Vec::new(),
                    cwd: None,
                    env: BTreeMap::new(),
                    disabled: false,
                },
            ),
        ]));

        assert!(registry.refresh_all().await.is_ok());
        let summaries = registry.summaries();
        let disabled_summary = summaries
            .iter()
            .find(|s| s.name == "disabled_broken")
            .unwrap();
        let enabled_summary = summaries
            .iter()
            .find(|s| s.name == "enabled_broken")
            .unwrap();

        assert_eq!(disabled_summary.status, McpConnectionStatus::Disconnected);
        assert!(disabled_summary.disabled);
        assert_eq!(disabled_summary.status_text(), "disabled");
        assert!(matches!(
            enabled_summary.status,
            McpConnectionStatus::Failed(_)
        ));
    }

    #[tokio::test]
    async fn refresh_all_runs_and_marks_failed_without_blocking() {
        let registry = McpRegistry::new(BTreeMap::from([
            (
                "broken1".to_string(),
                McpServerSpec::Stdio {
                    command: "/definitely/missing-1".into(),
                    args: Vec::new(),
                    cwd: None,
                    env: BTreeMap::new(),
                    disabled: false,
                },
            ),
            (
                "broken2".to_string(),
                McpServerSpec::Stdio {
                    command: "/definitely/missing-2".into(),
                    args: Vec::new(),
                    cwd: None,
                    env: BTreeMap::new(),
                    disabled: false,
                },
            ),
        ]));

        assert!(registry.refresh_all().await.is_ok());
        assert!(!registry.has_connecting());
        let summaries = registry.summaries();
        assert_eq!(summaries.len(), 2);
        assert!(matches!(
            summaries[0].status,
            McpConnectionStatus::Failed(_)
        ));
        assert!(matches!(
            summaries[1].status,
            McpConnectionStatus::Failed(_)
        ));
    }
}
