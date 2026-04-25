use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::{
    config::ActiveModel,
    log_debug, log_error,
    session::{BackendEvent, Message, MessageAttachment, MessageRole, ToolCall},
    tooling::ToolDefinition,
};

use super::attachments::{image_attachments, message_text_with_file_references};
use super::error::classify_response_status;

/// Responses API endpoint
const RESPONSES_ENDPOINT: &str = "/v1/responses";

pub(super) async fn stream_responses(
    http: &Client,
    session_id: Uuid,
    request_id: u64,
    model: ActiveModel,
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    tx: UnboundedSender<BackendEvent>,
) -> Result<()> {
    let api_key = model
        .api_key
        .clone()
        .with_context(|| format!("missing API key for provider '{}'", model.provider_id))?;

    let request = build_responses_request(&model, messages, true, &tools)?;
    let request_body_size = serde_json::to_string(&request)
        .map(|s| s.len())
        .unwrap_or(0);

    let endpoint = format!(
        "{}{}",
        model.base_url.trim_end_matches('/'),
        RESPONSES_ENDPOINT
    );

    let send_result = http
        .post(&endpoint)
        .bearer_auth(api_key)
        .json(&request)
        .send()
        .await;

    let response = match send_result {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                resp
            } else {
                let error_body = resp.text().await.unwrap_or_default();
                log_error!(
                    "openai responses request failed: method=POST url={} request_body_size={} status={} error_body={}",
                    endpoint,
                    request_body_size,
                    status,
                    error_body
                );
                return Err(classify_response_status(status, Some(error_body)).into());
            }
        }
        Err(e) => {
            log_error!(
                "openai responses request failed: method=POST url={} request_body_size={} error={}",
                endpoint,
                request_body_size,
                e
            );
            return Err(e.into());
        }
    };

    log_debug!(
        "openai responses request: method=POST url={} request_body_size={} status={}",
        endpoint,
        request_body_size,
        response.status()
    );

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut assistant_text = String::new();
    let mut reasoning_text = String::new();
    let mut finish_reason: Option<String> = None;
    let mut tool_calls: BTreeMap<String, ToolCallBuilder> = BTreeMap::new();
    let mut first_delta_time: Option<std::time::Instant> = None;

    use futures_util::StreamExt;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim_end_matches('\r').to_string();
            buffer.drain(..=line_end);

            if line.is_empty() {
                continue;
            }

            // Parse SSE event format: "event: TYPE\ndata: {...}\n\n"
            let event_type = line.strip_prefix("event:").map(|s| s.trim().to_string());

            let payload = line.strip_prefix("data:").map(|s| s.trim().to_string());

            if let (Some(_event), Some(payload)) = (event_type, payload) {
                if payload == "[DONE]" {
                    let turn = finalize_turn(
                        &assistant_text,
                        &reasoning_text,
                        &finish_reason,
                        &tool_calls,
                    );
                    let _ = tx.send(BackendEvent::Finished {
                        session_id,
                        request_id,
                        turn,
                    });
                    return Ok(());
                }

                let event: ResponsesStreamEvent =
                    serde_json::from_str(&payload).context("failed to parse responses event")?;

                // Handle usage stats
                if let Some(usage) = event.data.usage {
                    let duration_ms =
                        first_delta_time.map(|start| start.elapsed().as_millis() as u64);
                    let _ = tx.send(BackendEvent::UsageStats {
                        session_id,
                        request_id,
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        total_tokens: usage.total_tokens,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                        model_id: model.model_id.clone(),
                        duration_ms,
                    });
                }

                // Process content deltas
                for content in event.data.content {
                    match content.kind.as_str() {
                        "output_text" => {
                            if let Some(text) = content.text {
                                if first_delta_time.is_none() {
                                    first_delta_time = Some(std::time::Instant::now());
                                }
                                assistant_text.push_str(&text);
                                let _ = tx.send(BackendEvent::Delta {
                                    session_id,
                                    request_id,
                                    content: text,
                                });
                            }
                        }
                        "reasoning" => {
                            if let Some(text) = content.text {
                                if first_delta_time.is_none() {
                                    first_delta_time = Some(std::time::Instant::now());
                                }
                                reasoning_text.push_str(&text);
                                let _ = tx.send(BackendEvent::ReasoningDelta {
                                    session_id,
                                    request_id,
                                    content: text,
                                });
                            }
                        }
                        "tool_use" => {
                            if let Some(name) = content.name {
                                let call_id = content
                                    .id
                                    .unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4()));

                                let builder = tool_calls
                                    .entry(call_id.clone())
                                    .or_insert_with(|| ToolCallBuilder::new(call_id.clone(), name));

                                if let Some(arguments) = content.arguments {
                                    builder.append_arguments(&arguments);
                                }

                                if let Some(arguments) = builder.arguments() {
                                    let call = ToolCall {
                                        id: call_id.clone(),
                                        name: builder.name().to_string(),
                                        arguments: arguments.to_string(),
                                    };
                                    let _ = tx.send(BackendEvent::ToolCallUpdated {
                                        session_id,
                                        request_id,
                                        tool_call: call,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Handle finish reason
                if let Some(reason) = event.data.finish_reason {
                    finish_reason = Some(reason);
                }
            }
        }
    }

    Ok(())
}

pub(super) async fn complete_responses(
    http: &Client,
    model: ActiveModel,
    messages: Vec<Message>,
) -> Result<String> {
    let api_key = model
        .api_key
        .clone()
        .with_context(|| format!("missing API key for provider '{}'", model.provider_id))?;

    let request = build_responses_request(&model, messages, false, &[])?;
    let request_body_size = serde_json::to_string(&request)
        .map(|s| s.len())
        .unwrap_or(0);

    let endpoint = format!(
        "{}{}",
        model.base_url.trim_end_matches('/'),
        RESPONSES_ENDPOINT
    );

    let send_result = http
        .post(&endpoint)
        .bearer_auth(api_key)
        .json(&request)
        .send()
        .await;

    let response = match send_result {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                resp
            } else {
                let error_body = resp.text().await.unwrap_or_default();
                log_error!(
                    "openai responses request (complete) failed: method=POST url={} request_body_size={} status={} error_body={}",
                    endpoint,
                    request_body_size,
                    status,
                    error_body
                );
                return Err(classify_response_status(status, Some(error_body)).into());
            }
        }
        Err(e) => {
            log_error!(
                "openai responses request (complete) failed: method=POST url={} request_body_size={} error={}",
                endpoint,
                request_body_size,
                e
            );
            return Err(e.into());
        }
    };

    log_debug!(
        "openai responses request (complete): method=POST url={} request_body_size={} status={}",
        endpoint,
        request_body_size,
        response.status()
    );

    let response: ResponsesCompleteResponse = response.json().await?;

    let content = response
        .output
        .into_iter()
        .find_map(|output| {
            if output.kind == "message" {
                Some(output.content)
            } else {
                None
            }
        })
        .unwrap_or_default();

    Ok(content)
}

fn build_responses_request(
    model: &ActiveModel,
    messages: Vec<Message>,
    stream: bool,
    tools: &[ToolDefinition],
) -> Result<ResponsesRequest> {
    let mut request_messages = Vec::new();

    // Extract context summary from System messages (from context compaction)
    let context_summary: Option<String> = messages
        .iter()
        .filter(|message| !message.streaming)
        .filter(|message| message.role == MessageRole::System)
        .map(message_text_with_file_references)
        .next();

    // Build combined system prompt: model.system_prompt + context summary
    let combined_system_prompt = match (
        model.system_prompt.trim().is_empty(),
        context_summary.as_ref().map(|s| s.trim().is_empty()),
    ) {
        (false, Some(false)) => Some(format!(
            "{}\n\n{}",
            model.system_prompt.trim(),
            context_summary.as_ref().unwrap().trim()
        )),
        (false, _) => Some(model.system_prompt.clone()),
        (true, Some(false)) => context_summary,
        (true, _) => None,
    };

    // Instructions come from system prompt
    let instructions = combined_system_prompt.filter(|s| !s.trim().is_empty());

    // Process only User/Assistant/Tool messages (System messages already handled above)
    for message in &messages {
        if message.streaming {
            continue;
        }

        match message.role {
            MessageRole::System => {}
            MessageRole::User => {
                let content = user_message_content(message)?;
                request_messages.push(ResponseMessage {
                    role: "user".to_string(),
                    content,
                });
            }
            MessageRole::Assistant => {
                let text = message_text_with_file_references(message);
                let has_tool_calls = !message.tool_calls.is_empty();

                let content = if has_tool_calls {
                    let mut content_parts = Vec::new();
                    if !text.is_empty() {
                        content_parts.push(ResponseContentPart::text(text));
                    }
                    for tool_call in &message.tool_calls {
                        content_parts.push(ResponseContentPart::tool_use(
                            tool_call.id.clone(),
                            tool_call.name.clone(),
                            tool_call.arguments.clone(),
                        ));
                    }
                    ResponseContent::Array(content_parts)
                } else if !text.is_empty() {
                    ResponseContent::Text(text)
                } else {
                    continue;
                };

                request_messages.push(ResponseMessage {
                    role: "assistant".to_string(),
                    content,
                });
            }
            MessageRole::Tool => {
                let text = message_text_with_file_references(message);
                let mut content_parts = Vec::new();
                content_parts.push(ResponseContentPart::text(text));
                if let (Some(name), Some(id)) = (&message.tool_name, &message.tool_call_id) {
                    content_parts.push(ResponseContentPart::tool_use(
                        id.clone(),
                        name.clone(),
                        String::new(),
                    ));
                }
                request_messages.push(ResponseMessage {
                    role: "tool".to_string(),
                    content: ResponseContent::Array(content_parts),
                });
            }
            MessageRole::Error => {}
        }
    }

    let chat_tools = if tools.is_empty() {
        None
    } else {
        Some(tools.iter().map(ResponseTool::from).collect())
    };

    Ok(ResponsesRequest {
        model: model.request_model_id.clone(),
        instructions,
        input: request_messages,
        tools: chat_tools,
        temperature: Some(model.temperature),
        max_output_tokens: Some(model.max_output_tokens),
        stream,
        stream_options: if stream {
            Some(StreamOptions {
                include_usage: true,
            })
        } else {
            None
        },
        thinking: model.thinking_config(),
    })
}

fn user_message_content(message: &Message) -> Result<ResponseContent> {
    let mut parts = Vec::new();
    let text = message_text_with_file_references(message);

    if !text.is_empty() {
        parts.push(ResponseContentPart::text(text));
    }

    // Handle image attachments
    for attachment in image_attachments(message) {
        if let MessageAttachment::Image { data_url, .. } = attachment {
            parts.push(ResponseContentPart::image(data_url.clone()));
        }
    }

    if parts.len() == 1 && !parts[0].has_image() {
        Ok(ResponseContent::Text(parts.pop().unwrap().unwrap_text()))
    } else {
        Ok(ResponseContent::Array(parts))
    }
}

fn finalize_turn(
    assistant_text: &str,
    reasoning_text: &str,
    finish_reason: &Option<String>,
    tool_calls: &BTreeMap<String, ToolCallBuilder>,
) -> crate::session::AssistantTurn {
    let tool_calls = tool_calls.values().map(|builder| ToolCall {
            id: builder.id().to_string(),
            name: builder.name().to_string(),
            arguments: builder.arguments().unwrap_or_default().to_string(),
        })
        .collect::<Vec<_>>();

    let final_finish_reason = finish_reason.clone().unwrap_or_else(|| {
        if tool_calls.is_empty() {
            "stop".to_string()
        } else {
            "tool_calls".to_string()
        }
    });

    crate::session::AssistantTurn {
        content: assistant_text.to_string(),
        reasoning: reasoning_text.to_string(),
        tool_calls,
        finish_reason: Some(final_finish_reason),
    }
}

// ToolCallBuilder for Responses API
struct ToolCallBuilder {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallBuilder {
    fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            arguments: String::new(),
        }
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn arguments(&self) -> Option<&str> {
        if self.arguments.is_empty() {
            None
        } else {
            Some(&self.arguments)
        }
    }

    fn append_arguments(&mut self, args: &str) {
        self.arguments.push_str(args);
    }
}

// Response Content Part helpers
impl ResponseContentPart {
    fn text(content: String) -> Self {
        Self {
            kind: "text".to_string(),
            text: Some(content),
            image: None,
            id: None,
            name: None,
            arguments: None,
        }
    }

    fn image(url: String) -> Self {
        Self {
            kind: "image".to_string(),
            text: None,
            image: Some(ResponseImage { url }),
            id: None,
            name: None,
            arguments: None,
        }
    }

    fn tool_use(id: String, name: String, arguments: String) -> Self {
        Self {
            kind: "tool_use".to_string(),
            text: None,
            image: None,
            id: Some(id),
            name: Some(name),
            arguments: Some(arguments),
        }
    }

    fn has_image(&self) -> bool {
        self.image.is_some()
    }

    fn unwrap_text(self) -> String {
        self.text.unwrap_or_default()
    }
}

// ============================================================================
// Response data structures
// ============================================================================

#[derive(Clone, Debug, Serialize)]
struct ResponsesRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(rename = "input")]
    input: Vec<ResponseMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ResponseTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<usize>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize)]
struct StreamOptions {
    #[serde(rename = "include_usage")]
    include_usage: bool,
}

#[derive(Clone, Debug, Serialize)]
struct ResponseMessage {
    role: String,
    content: ResponseContent,
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
enum ResponseContent {
    Text(String),
    Array(Vec<ResponseContentPart>),
}

#[derive(Clone, Debug, Serialize)]
struct ResponseContentPart {
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<ResponseImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    arguments: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct ResponseImage {
    url: String,
}

#[derive(Clone, Debug, Serialize)]
struct ResponseTool {
    #[serde(rename = "type")]
    kind: String,
    name: String,
    description: String,
    parameters: serde_json::Value,
}

impl From<&ToolDefinition> for ResponseTool {
    fn from(def: &ToolDefinition) -> Self {
        Self {
            kind: "function".to_string(),
            name: def.name.to_string(),
            description: def.description.clone(),
            parameters: def.parameters.clone(),
        }
    }
}

// ============================================================================
// Response parsing structures
// ============================================================================

#[derive(Clone, Debug, Deserialize)]
struct ResponsesStreamEvent {
    #[serde(default)]
    data: ResponseEventData,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ResponseEventData {
    #[serde(default)]
    content: Vec<ResponseOutputContent>,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    usage: Option<ResponseUsage>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ResponseOutputContent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ResponseUsage {
    #[serde(rename = "input_tokens")]
    input_tokens: u32,
    #[serde(rename = "completion_tokens")]
    output_tokens: u32,
    #[serde(rename = "total_tokens")]
    total_tokens: u32,
}

#[derive(Clone, Debug, Deserialize)]
struct ResponsesCompleteResponse {
    #[serde(default)]
    output: Vec<ResponseOutput>,
}

#[derive(Clone, Debug, Deserialize)]
struct ResponseOutput {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    content: String,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ApiType;

    #[test]
    fn test_responses_request_basic() {
        let model = ActiveModel {
            provider_id: "openai-responses".to_string(),
            provider_display_name: "OpenAI Responses".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: "gpt-4.5".to_string(),
            display_name: "GPT-4.5".to_string(),
            context_window: 128000,
            max_output_tokens: 4096,
            temperature: 0.7,
            supports_images: true,
            system_prompt: "You are helpful.".to_string(),
            api_key: None,
            extra_body: None,
            thinking_level: crate::config::reasoning::ThinkingLevelType::None,
        };

        let messages = vec![Message::new(MessageRole::User, "Hello")];

        let request = build_responses_request(&model, messages, true, &[]).unwrap();

        assert_eq!(request.model, "gpt-4.5");
        assert_eq!(request.instructions, Some("You are helpful.".to_string()));
        assert!(request.stream);
        assert_eq!(request.input.len(), 1);
        assert_eq!(request.input[0].role, "user");
    }

    #[test]
    fn test_responses_request_with_system_prompt() {
        let model = ActiveModel {
            provider_id: "test".to_string(),
            provider_display_name: "Test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: "gpt-4.5".to_string(),
            display_name: "GPT-4.5".to_string(),
            context_window: 128000,
            max_output_tokens: 4096,
            temperature: 0.7,
            supports_images: false,
            system_prompt: "Base system prompt".to_string(),
            api_key: None,
            extra_body: None,
            thinking_level: crate::config::reasoning::ThinkingLevelType::None,
        };

        let messages = vec![
            Message::new(MessageRole::System, "Context summary"),
            Message::new(MessageRole::User, "Hello"),
        ];

        let request = build_responses_request(&model, messages, false, &[]).unwrap();

        assert!(request.instructions.is_some());
        let instructions = request.instructions.unwrap();
        assert!(instructions.contains("Base system prompt"));
        assert!(instructions.contains("Context summary"));
    }

    #[test]
    fn test_responses_request_tool_calls() {
        let model = ActiveModel {
            provider_id: "test".to_string(),
            provider_display_name: "Test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: "gpt-4.5".to_string(),
            display_name: "GPT-4.5".to_string(),
            context_window: 128000,
            max_output_tokens: 4096,
            temperature: 0.7,
            supports_images: false,
            system_prompt: "You are helpful.".to_string(),
            api_key: None,
            extra_body: None,
            thinking_level: crate::config::reasoning::ThinkingLevelType::None,
        };

        let mut message = Message::new(MessageRole::User, "Run command");
        message.tool_calls.push(ToolCall {
            id: "call_123".to_string(),
            name: "bash".to_string(),
            arguments: "{\"command\":\"ls\"}".to_string(),
        });

        let request = build_responses_request(&model, vec![message], false, &[]).unwrap();

        assert!(request.tools.is_none()); // No tools provided to the request
    }

    #[test]
    fn test_response_tool_spec() {
        let tool = ToolDefinition {
            name: "bash".to_string(),
            display_name: "Bash".to_string(),
            description: "Execute shell command".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                }
            }),
            permission: crate::tooling::ToolPermission::Execute,
            origin: crate::tooling::ToolOrigin::Local,
        };

        let response_tool = ResponseTool::from(&tool);

        assert_eq!(response_tool.kind, "function");
        assert_eq!(response_tool.name, "bash");
        assert!(!response_tool.description.is_empty());
    }
}
