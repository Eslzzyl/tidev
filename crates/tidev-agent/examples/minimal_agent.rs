//! Minimal tidev-agent consumer.
//!
//! Set `TIDEV_RUN=1` and the provider variables to run the full loop. Set
//! `TIDEV_MCP_COMMAND` to connect an optional stdio MCP server; its discovered
//! tools are registered beside the built-in echo tool.

use std::collections::BTreeMap;
use std::env;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc::unbounded_channel;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_agent::tidev_llm::message::{Message, ToolCall, ToolExecutionResult};
use tidev_agent::tidev_llm::reasoning::ThinkingLevelType;
use tidev_agent::tidev_llm::{ApiType, LlmClient, LlmProviderConfig, ToolDefinition};
use tidev_agent::{
    AgentEvent, AgentRuntime, ContextManager, McpRegistry, McpServerSpec, MessageStore, Tool,
    ToolContext, ToolRegistry,
};

struct MemoryStore {
    messages: Mutex<Vec<Message>>,
}

#[async_trait]
impl MessageStore for MemoryStore {
    async fn load_messages(&self, _session_id: Uuid) -> Result<Vec<Message>> {
        Ok(self.messages.lock().unwrap().clone())
    }

    async fn save_messages(&self, _session_id: Uuid, messages: &[Message]) -> Result<()> {
        self.messages.lock().unwrap().extend_from_slice(messages);
        Ok(())
    }
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "echo".into(),
            display_name: "Echo".into(),
            description: "Return the supplied text.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }),
        }
    }

    fn read_only(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _context: &dyn ToolContext,
    ) -> Result<ToolExecutionResult> {
        Ok(ToolExecutionResult::new(
            args.get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
        ))
    }
}

struct LengthTool;

#[async_trait]
impl Tool for LengthTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "length".into(),
            display_name: "Length".into(),
            description: "Return the character count of supplied text.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }),
        }
    }

    fn read_only(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _context: &dyn ToolContext,
    ) -> Result<ToolExecutionResult> {
        let length = args
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map_or(0, |text| text.chars().count())
            .to_string();
        Ok(ToolExecutionResult::new(length))
    }
}

fn provider_config() -> LlmProviderConfig {
    LlmProviderConfig {
        provider_id: "example".into(),
        api_type: ApiType::parse(&env::var("TIDEV_API_TYPE").unwrap_or_default()),
        api_key: env::var("TIDEV_API_KEY").ok(),
        base_url: env::var("TIDEV_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080/v1".into()),
        user_agent: None,
        model_id: env::var("TIDEV_MODEL").unwrap_or_else(|_| "example-model".into()),
        request_model_id: None,
        system_prompt: None,
        thinking_level: ThinkingLevelType::None,
        extra_body: None,
        max_output_tokens: 1024,
        context_window: 16_000,
        temperature: None,
        supports_images: false,
        supports_parallel_tool_calls: true,
    }
}

fn mcp_spec_from_environment() -> Option<McpServerSpec> {
    let command = env::var("TIDEV_MCP_COMMAND").ok()?;
    let args = env::var("TIDEV_MCP_ARGS")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    Some(McpServerSpec::Stdio {
        command,
        args,
        cwd: env::var("TIDEV_MCP_CWD").ok().map(Into::into),
        env: BTreeMap::new(),
        disabled: false,
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let session_id = Uuid::new_v4();
    let (event_tx, mut event_rx) = unbounded_channel::<AgentEvent>();

    let mut tool_registry = ToolRegistry::new(64 * 1024);
    tool_registry.register(EchoTool);
    tool_registry.register(LengthTool);

    let mcp = mcp_spec_from_environment()
        .map(|spec| McpRegistry::new(BTreeMap::from([("example".to_string(), spec)])));
    if let Some(mcp) = &mcp {
        mcp.refresh_all().await?;
        for tool in mcp.tool_implementations() {
            tool_registry.register_shared(tool);
        }
        println!("connected MCP tools: {}", mcp.all_tools().len());
    }

    let store = Arc::new(MemoryStore {
        messages: Mutex::new(vec![Message::new(
            tidev_agent::tidev_llm::message::MessageRole::User,
            env::var("TIDEV_PROMPT").unwrap_or_else(|_| "Use echo to repeat hello".into()),
        )]),
    });
    let initial_messages = store.messages.lock().unwrap().clone();
    let llm = LlmClient::new(false, 0, false, 0)?;
    let cancel = CancellationToken::new();
    let runtime = AgentRuntime::new(
        session_id,
        llm,
        provider_config(),
        Arc::new(tool_registry),
        ContextManager::new(),
        initial_messages,
        store,
        env::current_dir()?,
        event_tx,
        cancel,
    );

    let echo_call = ToolCall {
        id: "example-echo".into(),
        name: "echo".into(),
        arguments: r#"{"text":"hello from tidev-agent"}"#.into(),
        thought_signature: None,
    };
    let result = runtime.execute_tool(&echo_call).await?;
    println!("echo result: {}", result.output);

    if let Ok(tool_name) = env::var("TIDEV_MCP_TOOL") {
        let mcp_call = ToolCall {
            id: "example-mcp".into(),
            name: tool_name,
            arguments: env::var("TIDEV_MCP_ARGS_JSON").unwrap_or_else(|_| "{}".into()),
            thought_signature: None,
        };
        let result = runtime.execute_tool(&mcp_call).await?;
        println!("MCP result: {}", result.output);
    }

    if env::var("TIDEV_RUN").as_deref() == Ok("1") {
        let steer_signal = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let run = runtime.run(
            "You are a concise tool-using assistant.".into(),
            ThinkingLevelType::None,
            steer_signal,
        );
        tokio::pin!(run);
        loop {
            tokio::select! {
                result = &mut run => {
                    result?;
                    break;
                }
                event = event_rx.recv() => {
                    if let Some(event) = event {
                        println!("event: {event:?}");
                    }
                }
            }
        }
    }

    Ok(())
}
