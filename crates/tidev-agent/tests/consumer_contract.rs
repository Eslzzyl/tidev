use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use tidev_agent::tidev_llm::message::{Message, MessageRole, ToolCall, ToolExecutionResult};
use tidev_agent::tidev_llm::reasoning::ThinkingLevelType;
use tidev_agent::tidev_llm::{ApiType, LlmClient, LlmProviderConfig, ToolDefinition};
use tidev_agent::{
    AgentContext, AgentEvent, AgentRuntime, ContextManager, McpConnectionStatus, McpRegistry,
    McpServerSpec, MessageStore, Tool, ToolContext, ToolRegistry,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct RecordingStore {
    messages: Mutex<Vec<Message>>,
    saves: Mutex<Vec<Vec<Message>>>,
}

#[async_trait]
impl MessageStore for RecordingStore {
    async fn load_messages(&self, _session_id: Uuid) -> Result<Vec<Message>> {
        Ok(self.messages.lock().unwrap().clone())
    }

    async fn save_messages(&self, _session_id: Uuid, messages: &[Message]) -> Result<()> {
        self.messages.lock().unwrap().extend_from_slice(messages);
        self.saves.lock().unwrap().push(messages.to_vec());
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

fn provider_config(base_url: String) -> LlmProviderConfig {
    provider_config_with_user_agent(base_url, None)
}

fn provider_config_with_user_agent(
    base_url: String,
    user_agent: Option<&str>,
) -> LlmProviderConfig {
    LlmProviderConfig {
        provider_id: "scripted".into(),
        api_type: ApiType::OpenAiChatCompletions,
        api_key: Some("local-test-key".into()),
        base_url,
        user_agent: user_agent.map(str::to_owned),
        model_id: "scripted-model".into(),
        request_model_id: None,
        system_prompt: None,
        thinking_level: ThinkingLevelType::None,
        extra_body: None,
        max_output_tokens: 128,
        context_window: 4096,
        temperature: None,
        supports_images: false,
        supports_parallel_tool_calls: true,
    }
}

async fn read_http_request(stream: &mut TcpStream) -> Result<Vec<u8>> {
    Ok(read_http_request_parts(stream).await?.1)
}

async fn read_http_request_parts(stream: &mut TcpStream) -> Result<(String, Vec<u8>)> {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            anyhow::bail!("fixture closed before sending a request");
        }
        request.extend_from_slice(&chunk[..read]);
        if let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break header_end + 4;
        }
    };

    let header_text = std::str::from_utf8(&request[..header_end - 4])?.to_string();
    let content_length = header_text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.eq_ignore_ascii_case("content-length")).then_some(value.trim())
        })
        .unwrap_or("0")
        .parse::<usize>()?;

    while request.len() < header_end + content_length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            anyhow::bail!("fixture closed in the request body");
        }
        request.extend_from_slice(&chunk[..read]);
    }
    Ok((
        header_text,
        request[header_end..header_end + content_length].to_vec(),
    ))
}

fn tool_call_response() -> String {
    let first = serde_json::json!({
        "id": "scripted-1",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "scripted-model",
        "choices": [{
            "index": 0,
            "delta": {
                "role": "assistant",
                "tool_calls": [{
                    "index": 0,
                    "id": "call-echo",
                    "type": "function",
                    "function": {
                        "name": "echo",
                        "arguments": "{\"text\":\"hello from scripted provider\"}"
                    }
                }]
            },
            "finish_reason": null
        }]
    });
    let second = serde_json::json!({
        "id": "scripted-1",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "scripted-model",
        "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
    });
    format!("data: {first}\n\ndata: {second}\n\ndata: [DONE]\n\n")
}

fn final_response() -> String {
    let chunk = serde_json::json!({
        "id": "scripted-2",
        "object": "chat.completion.chunk",
        "created": 2,
        "model": "scripted-model",
        "choices": [{
            "index": 0,
            "delta": {"content": "tool result received"},
            "finish_reason": "stop"
        }]
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

fn second_tool_call_response() -> String {
    let first = serde_json::json!({
        "id": "scripted-2",
        "object": "chat.completion.chunk",
        "created": 2,
        "model": "scripted-model",
        "choices": [{
            "index": 0,
            "delta": {
                "role": "assistant",
                "tool_calls": [{
                    "index": 0,
                    "id": "call-echo-2",
                    "type": "function",
                    "function": {
                        "name": "echo",
                        "arguments": "{\"text\":\"second scripted call\"}"
                    }
                }]
            },
            "finish_reason": null
        }]
    });
    let second = serde_json::json!({
        "id": "scripted-2",
        "object": "chat.completion.chunk",
        "created": 2,
        "model": "scripted-model",
        "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
    });
    format!("data: {first}\n\ndata: {second}\n\ndata: [DONE]\n\n")
}

#[derive(Deserialize)]
struct RawChatRequest<'a> {
    #[serde(borrow)]
    messages: &'a serde_json::value::RawValue,
    #[serde(borrow)]
    tools: &'a serde_json::value::RawValue,
}

async fn scripted_provider(listener: TcpListener) -> Result<Vec<Vec<u8>>> {
    let mut requests = Vec::new();
    for response in [
        tool_call_response(),
        second_tool_call_response(),
        final_response(),
        final_response(),
    ] {
        let (mut stream, _) = listener.accept().await?;
        let (headers, body) = read_http_request_parts(&mut stream).await?;
        let expected_user_agent = format!("tidev/{}", env!("CARGO_PKG_VERSION"));
        assert!(has_header(&headers, "user-agent", &expected_user_agent));
        requests.push(body);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        );
        stream.write_all(response.as_bytes()).await?;
    }
    Ok(requests)
}

async fn complete_provider(listener: TcpListener) -> Result<String> {
    let (mut stream, _) = listener.accept().await?;
    let (headers, _body) = read_http_request_parts(&mut stream).await?;
    let response = serde_json::json!({
        "id": "complete-1",
        "choices": [{"message": {"content": "complete response"}}]
    })
    .to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.len(),
        response
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(headers)
}

#[tokio::test]
async fn custom_user_agent_overrides_default_for_completion() -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let base_url = format!("http://{}/v1", listener.local_addr()?);
    let provider = tokio::spawn(complete_provider(listener));

    let response = LlmClient::new(false, 0, false, 0)?
        .complete_with_messages(
            provider_config_with_user_agent(base_url, Some("gateway-client/1.0")),
            vec![Message::new(MessageRole::User, "Hello")],
            vec![],
            None,
        )
        .await?;

    assert_eq!(response, "complete response");
    let headers = provider.await??;
    assert!(has_header(&headers, "user-agent", "gateway-client/1.0"));
    Ok(())
}

async fn compaction_provider(listener: TcpListener) -> Result<Vec<u8>> {
    let (mut stream, _) = listener.accept().await?;
    let body = read_http_request(&mut stream).await?;
    let response = serde_json::json!({
        "id": "compaction-1",
        "choices": [{"message": {"content": "compacted conversation summary"}}]
    })
    .to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.len(),
        response
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(body)
}

#[tokio::test]
async fn independent_consumer_runs_full_loop_against_scripted_provider() -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let base_url = format!("http://{}/v1", listener.local_addr()?);
    let provider = tokio::spawn(scripted_provider(listener));

    let session_id = Uuid::from_u128(0x11);
    let store = Arc::new(RecordingStore {
        messages: Mutex::new(vec![Message::new(MessageRole::User, "Please use echo.")]),
        saves: Mutex::new(Vec::new()),
    });
    let mut registry = ToolRegistry::new(64 * 1024);
    registry.register(EchoTool);
    let registry = Arc::new(registry);
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let runtime = AgentRuntime::new(
        session_id,
        LlmClient::new(false, 0, false, 0)?,
        provider_config(base_url.clone()),
        registry.clone(),
        ContextManager::new(),
        store.messages.lock().unwrap().clone(),
        store.clone(),
        PathBuf::from("/tmp/tidev-agent-fixture"),
        event_tx,
        CancellationToken::new(),
    );

    runtime
        .run(
            "You are a test assistant.".into(),
            ThinkingLevelType::None,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .await?;

    let stored = runtime.stored_messages();
    assert_eq!(stored.len(), 6);
    assert_eq!(stored[2].role, MessageRole::Tool);
    assert_eq!(stored[2].content, "hello from scripted provider");
    assert_eq!(stored[4].content, "second scripted call");
    assert_eq!(stored[5].content, "tool result received");
    assert_eq!(store.saves.lock().unwrap().len(), 5);

    let mut events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        events.push(event);
    }
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolStarting { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolCompleted { .. }))
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, AgentEvent::Finished { .. }))
            .count(),
        3
    );

    let (reload_event_tx, _reload_event_rx) = tokio::sync::mpsc::unbounded_channel();
    let reloaded = AgentRuntime::from_store(
        LlmClient::new(false, 0, false, 0)?,
        provider_config(base_url),
        registry,
        ContextManager::new(),
        store.clone(),
        session_id,
        PathBuf::from("/tmp/tidev-agent-fixture"),
        reload_event_tx,
        CancellationToken::new(),
    )
    .await?;
    reloaded
        .run(
            "You are a test assistant.".into(),
            ThinkingLevelType::None,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .await?;
    assert_eq!(reloaded.stored_messages().len(), 7);
    assert_eq!(store.saves.lock().unwrap().len(), 6);

    let requests = provider.await??;
    assert_eq!(requests.len(), 4);
    let parsed_requests: Vec<serde_json::Value> = requests
        .iter()
        .map(|body| serde_json::from_slice(body))
        .collect::<serde_json::Result<_>>()?;
    assert_eq!(
        parsed_requests[0]["messages"][1]["content"],
        "Please use echo."
    );
    let second_messages = parsed_requests[1]["messages"].as_array().unwrap();
    assert!(second_messages.iter().any(|message| {
        message["role"] == "assistant" && message["tool_calls"][0]["function"]["name"] == "echo"
    }));
    assert!(
        second_messages
            .iter()
            .any(|message| { message["role"] == "tool" && message["tool_call_id"] == "call-echo" })
    );
    let third_messages = parsed_requests[2]["messages"].as_array().unwrap();
    assert!(
        third_messages.iter().any(|message| {
            message["role"] == "tool" && message["tool_call_id"] == "call-echo-2"
        })
    );

    let raw_requests: Vec<RawChatRequest<'_>> = requests
        .iter()
        .map(|body| serde_json::from_slice(body))
        .collect::<serde_json::Result<_>>()?;
    let first_messages: Vec<&serde_json::value::RawValue> =
        serde_json::from_str(raw_requests[0].messages.get())?;
    let second_messages: Vec<&serde_json::value::RawValue> =
        serde_json::from_str(raw_requests[1].messages.get())?;
    let third_messages: Vec<&serde_json::value::RawValue> =
        serde_json::from_str(raw_requests[2].messages.get())?;
    let fourth_messages: Vec<&serde_json::value::RawValue> =
        serde_json::from_str(raw_requests[3].messages.get())?;
    assert_eq!(
        first_messages
            .iter()
            .map(|message| message.get())
            .collect::<Vec<_>>(),
        second_messages[..2]
            .iter()
            .map(|message| message.get())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        second_messages
            .iter()
            .map(|message| message.get())
            .collect::<Vec<_>>(),
        third_messages[..second_messages.len()]
            .iter()
            .map(|message| message.get())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        third_messages
            .iter()
            .map(|message| message.get())
            .collect::<Vec<_>>(),
        fourth_messages[..third_messages.len()]
            .iter()
            .map(|message| message.get())
            .collect::<Vec<_>>()
    );
    assert_eq!(raw_requests[0].tools.get(), raw_requests[1].tools.get());
    assert_eq!(raw_requests[1].tools.get(), raw_requests[2].tools.get());
    assert_eq!(raw_requests[2].tools.get(), raw_requests[3].tools.get());
    Ok(())
}

#[tokio::test]
async fn independent_consumer_sends_compaction_request_through_provider() -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let base_url = format!("http://{}/v1", listener.local_addr()?);
    let provider = tokio::spawn(compaction_provider(listener));

    let session_id = Uuid::from_u128(0x12);
    let mut old_message = Message::new(MessageRole::User, "conversation before compaction");
    old_message.input_tokens = Some(6_000);
    let store = Arc::new(RecordingStore {
        messages: Mutex::new(vec![old_message]),
        saves: Mutex::new(Vec::new()),
    });
    let model = {
        let mut model = provider_config(base_url);
        model.context_window = 10_000;
        model
    };
    let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
    let runtime = AgentRuntime::new(
        session_id,
        LlmClient::new(false, 0, false, 0)?,
        model,
        Arc::new(ToolRegistry::new(64 * 1024)),
        ContextManager::new(),
        store.messages.lock().unwrap().clone(),
        store.clone(),
        PathBuf::from("/tmp/tidev-agent-fixture"),
        event_tx,
        CancellationToken::new(),
    );

    let prepared = runtime.load_messages(session_id).await?;
    assert_eq!(prepared.len(), 1);
    assert!(
        prepared[0]
            .content
            .contains("compacted conversation summary")
    );
    assert_eq!(runtime.stored_messages().len(), 2);

    let body = provider.await??;
    let request: serde_json::Value = serde_json::from_slice(&body)?;
    let messages = request["messages"].as_array().unwrap();
    assert!(messages.iter().any(|message| {
        message["content"]
            .as_str()
            .is_some_and(|content| content.contains("conversation before compaction"))
    }));
    assert!(messages.iter().any(|message| {
        message["content"].as_str().is_some_and(|content| {
            content.contains("Please provide a detailed summary of the conversation history above")
        })
    }));
    Ok(())
}

#[tokio::test]
async fn independent_consumer_connects_stdio_mcp_and_calls_discovered_tool() -> Result<()> {
    let fixture = std::env::var("CARGO_BIN_EXE_mcp_fixture")
        .context("Cargo did not provide the mcp_fixture binary path")?;
    let registry = McpRegistry::new(BTreeMap::from([(
        "fixture".into(),
        McpServerSpec::Stdio {
            command: fixture,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            disabled: false,
        },
    )]));

    registry.refresh_server("fixture").await?;
    let summary = &registry.summaries()[0];
    assert_eq!(summary.status, McpConnectionStatus::Connected);
    assert_eq!(summary.tool_count, 1);
    let definition = registry
        .definition_for("mcp__fixture__fixture_echo")
        .context("fixture tool was not discovered")?;
    assert_eq!(definition.name, "mcp__fixture__fixture_echo");

    let result = registry
        .execute_call(&ToolCall {
            id: "mcp-call".into(),
            name: definition.name,
            arguments: r#"{"text":"hello"}"#.into(),
            thought_signature: None,
        })
        .await?;
    assert_eq!(result.output, "{\n  \"value\": \"fixture:hello\"\n}");
    registry.disconnect_server("fixture").await?;
    Ok(())
}

fn has_fixture_header(headers: &str) -> bool {
    headers.lines().any(|line| {
        let Some((name, value)) = line.split_once(':') else {
            return false;
        };
        name.eq_ignore_ascii_case("x-fixture-token") && value.trim() == "legacy-token"
    })
}

fn has_header(headers: &str, expected_name: &str, expected_value: &str) -> bool {
    headers.lines().any(|line| {
        let Some((name, value)) = line.split_once(':') else {
            return false;
        };
        name.eq_ignore_ascii_case(expected_name) && value.trim() == expected_value
    })
}

async fn write_sse_message(stream: &mut TcpStream, message: &serde_json::Value) -> Result<()> {
    let payload = format!(
        "event: message\ndata: {}\n\n",
        serde_json::to_string(message)?
    );
    stream.write_all(payload.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

async fn legacy_sse_fixture(listener: TcpListener) -> Result<Vec<serde_json::Value>> {
    let (mut sse_stream, _) = listener.accept().await?;
    let (sse_headers, sse_body) = read_http_request_parts(&mut sse_stream).await?;
    assert!(sse_headers.starts_with("GET /sse HTTP/1.1"));
    assert!(sse_body.is_empty());
    assert!(has_fixture_header(&sse_headers));
    sse_stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n",
        )
        .await?;
    sse_stream
        .write_all(b"event: endpoint\ndata: /messages?session=legacy-fixture\n\n")
        .await?;
    sse_stream.flush().await?;

    let mut requests = Vec::new();
    loop {
        let (mut post_stream, _) = listener.accept().await?;
        let (headers, body) = read_http_request_parts(&mut post_stream).await?;
        assert!(headers.starts_with("POST /messages?session=legacy-fixture HTTP/1.1"));
        assert!(has_fixture_header(&headers));
        assert!(has_header(
            &headers,
            "accept",
            "application/json, text/event-stream"
        ));
        let request: serde_json::Value = serde_json::from_slice(&body)?;
        requests.push(request.clone());
        post_stream
            .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await?;
        post_stream.shutdown().await?;

        match request["method"].as_str() {
            Some("initialize") => {
                write_sse_message(
                    &mut sse_stream,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": request["id"],
                        "result": {
                            "protocolVersion": "2025-11-25",
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "legacy-fixture", "version": "1.0.0"}
                        }
                    }),
                )
                .await?;
            }
            Some("notifications/initialized") => {}
            Some("tools/list") => {
                write_sse_message(
                    &mut sse_stream,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": request["id"],
                        "result": {
                            "tools": [{
                                "name": "legacy_echo",
                                "description": "Return a deterministic legacy SSE value.",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {"text": {"type": "string"}},
                                    "required": ["text"]
                                }
                            }]
                        }
                    }),
                )
                .await?;
            }
            Some("tools/call") => {
                assert_eq!(request["params"]["name"], "legacy_echo");
                assert_eq!(request["params"]["arguments"]["text"], "hello");
                write_sse_message(
                    &mut sse_stream,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": request["id"],
                        "result": {
                            "content": [{"type": "text", "text": "legacy:hello"}],
                            "isError": false
                        }
                    }),
                )
                .await?;
                break;
            }
            method => anyhow::bail!("unexpected legacy SSE method: {method:?}"),
        }
    }
    Ok(requests)
}

#[tokio::test]
async fn independent_consumer_connects_legacy_sse_and_calls_discovered_tool() -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let url = format!("http://{}/sse", listener.local_addr()?);
    let fixture = tokio::spawn(legacy_sse_fixture(listener));
    let registry = McpRegistry::new(BTreeMap::from([(
        "legacy".into(),
        McpServerSpec::Sse {
            url,
            headers: BTreeMap::from([("x-fixture-token".into(), "legacy-token".into())]),
            disabled: false,
        },
    )]));

    registry.refresh_server("legacy").await?;
    let summary = &registry.summaries()[0];
    assert_eq!(summary.status, McpConnectionStatus::Connected);
    assert_eq!(summary.tool_count, 1);
    let definition = registry
        .definition_for("mcp__legacy__legacy_echo")
        .context("legacy SSE tool was not discovered")?;
    let result = registry
        .execute_call(&ToolCall {
            id: "legacy-call".into(),
            name: definition.name,
            arguments: r#"{"text":"hello"}"#.into(),
            thought_signature: None,
        })
        .await?;
    assert_eq!(result.output, "legacy:hello");
    registry.disconnect_server("legacy").await?;

    let requests = fixture.await??;
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0]["method"], "initialize");
    assert_eq!(requests[1]["method"], "notifications/initialized");
    assert_eq!(requests[2]["method"], "tools/list");
    assert_eq!(requests[3]["method"], "tools/call");
    Ok(())
}

/// A scripted provider that serves: tool call → final response → final response.
async fn steer_provider(listener: TcpListener) -> Result<Vec<Vec<u8>>> {
    let mut requests = Vec::new();
    for response in [tool_call_response(), final_response(), final_response()] {
        let (mut stream, _) = listener.accept().await?;
        let body = read_http_request(&mut stream).await?;
        requests.push(body);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        );
        stream.write_all(response.as_bytes()).await?;
    }
    Ok(requests)
}

/// Setting the steering signal while the loop is running must keep the loop
/// alive after the model stops responding, so the next iteration picks up
/// the persisted steer message.
#[tokio::test]
async fn steer_signal_keeps_loop_alive_for_next_turn() -> Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};

    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let base_url = format!("http://{}/v1", listener.local_addr()?);
    let provider = tokio::spawn(steer_provider(listener));

    let session_id = Uuid::from_u128(0x21);
    let store = Arc::new(RecordingStore {
        messages: Mutex::new(vec![Message::new(MessageRole::User, "Please use echo.")]),
        saves: Mutex::new(Vec::new()),
    });
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let steer_signal = Arc::new(AtomicBool::new(false));
    let runtime = AgentRuntime::new(
        session_id,
        LlmClient::new(false, 0, false, 0)?,
        provider_config(base_url),
        Arc::new({
            let mut registry = ToolRegistry::new(64 * 1024);
            registry.register(EchoTool);
            registry
        }),
        ContextManager::new(),
        store.messages.lock().unwrap().clone(),
        store.clone(),
        PathBuf::from("/tmp/tidev-agent-fixture"),
        event_tx,
        CancellationToken::new(),
    );

    let run_task = tokio::spawn({
        let steer_signal = steer_signal.clone();
        async move {
            runtime
                .run(
                    "You are a test assistant.".into(),
                    ThinkingLevelType::None,
                    steer_signal,
                )
                .await
        }
    });

    // Wait for the first tool execution, then steer a message in: the
    // model's second response (no tool calls) must keep the loop alive.
    loop {
        match event_rx.recv().await {
            Some(AgentEvent::ToolCompleted { .. }) => break,
            Some(_) => continue,
            None => anyhow::bail!("event channel closed before tool completion"),
        }
    }
    steer_signal.store(true, Ordering::SeqCst);

    run_task.await??;

    // Three requests: tool call, final response (steer keeps the loop
    // alive), and the follow-up request for the steered message.
    let requests = provider.await??;
    assert_eq!(requests.len(), 3, "steer must trigger a follow-up request");

    // The follow-up request must carry the tool result from the previous
    // turn (the persisted history is replayed verbatim).
    let third: serde_json::Value = serde_json::from_slice(&requests[2])?;
    let messages = third["messages"].as_array().unwrap();
    assert!(
        messages.iter().any(|m| m["content"]
            .as_str()
            .is_some_and(|c| c.contains("hello from scripted provider"))),
        "follow-up request must include the persisted tool result"
    );
    Ok(())
}
