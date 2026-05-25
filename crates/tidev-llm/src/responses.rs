use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::{types::LlmProviderConfig, types::ToolDefinition};
use tidev_session::session::{BackendEvent, Message, MessageRole, ToolCall};

use log::{debug as log_debug, error as log_error};

use crate::attachments::message_text_with_file_references;
use crate::debug::save_request_for_debugging;
use crate::error::classify_response_status;

/// Responses API endpoint
const RESPONSES_ENDPOINT: &str = "/responses";

#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream_responses(
    http: &Client,
    session_id: Uuid,
    request_id: u64,
    model: LlmProviderConfig,
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    tx: UnboundedSender<BackendEvent>,
    save_request_body: bool,
    max_request_files: usize,
) -> Result<()> {
    let api_key = model
        .api_key
        .clone()
        .with_context(|| format!("missing API key for provider '{}'", model.provider_id))?;

    let request = build_responses_request(&model, messages, true, &tools)?;
    let request_body = serde_json::to_string(&request).unwrap_or_default();
    let request_body_size = request_body.len();
    save_request_for_debugging(&request_body, save_request_body, max_request_files);

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
    let mut current_event_type: Option<String> = None;

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

            // SSE event type line: "event: TYPE"
            if let Some(event) = line.strip_prefix("event:") {
                current_event_type = Some(event.trim().to_string());
                continue;
            }

            // SSE data line: "data: {...}"
            if let Some(payload) = line.strip_prefix("data:") {
                let payload = payload.trim().to_string();
                let _event_type = current_event_type.take();

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

                let event: ResponseStreamEvent =
                    serde_json::from_str(&payload).context("failed to parse responses event")?;

                match event {
                    ResponseStreamEvent::OutputTextDelta {
                        delta,
                        sequence_number: _,
                        output_index: _,
                        content_index: _,
                    } => {
                        if first_delta_time.is_none() {
                            first_delta_time = Some(std::time::Instant::now());
                        }
                        assistant_text.push_str(&delta);
                        let _ = tx.send(BackendEvent::Delta {
                            session_id,
                            request_id,
                            content: delta,
                        });
                    }
                    ResponseStreamEvent::RefusalDelta {
                        delta,
                        sequence_number: _,
                        output_index: _,
                        content_index: _,
                    } => {
                        // Handle refusal text (model declined to respond)
                        if first_delta_time.is_none() {
                            first_delta_time = Some(std::time::Instant::now());
                        }
                        assistant_text.push_str(&delta);
                        let _ = tx.send(BackendEvent::Delta {
                            session_id,
                            request_id,
                            content: delta,
                        });
                    }
                    ResponseStreamEvent::ReasoningDelta {
                        delta,
                        sequence_number: _,
                        output_index: _,
                        content_index: _,
                    } => {
                        if first_delta_time.is_none() {
                            first_delta_time = Some(std::time::Instant::now());
                        }
                        reasoning_text.push_str(&delta);
                        let _ = tx.send(BackendEvent::ReasoningDelta {
                            session_id,
                            request_id,
                            content: delta,
                        });
                    }
                    ResponseStreamEvent::ReasoningTextDelta {
                        delta,
                        sequence_number: _,
                        item_id: _,
                        output_index: _,
                        content_index: _,
                    } => {
                        if first_delta_time.is_none() {
                            first_delta_time = Some(std::time::Instant::now());
                        }
                        reasoning_text.push_str(&delta);
                        let _ = tx.send(BackendEvent::ReasoningDelta {
                            session_id,
                            request_id,
                            content: delta,
                        });
                    }
                    ResponseStreamEvent::ReasoningSummaryTextDelta {
                        summary_delta,
                        sequence_number: _,
                        item_id: _,
                        output_index: _,
                        summary_index: _,
                    } => {
                        reasoning_text.push_str(&summary_delta);
                        let _ = tx.send(BackendEvent::ReasoningDelta {
                            session_id,
                            request_id,
                            content: summary_delta,
                        });
                    }
                    ResponseStreamEvent::ReasoningSummaryTextDone {
                        text,
                        sequence_number: _,
                        item_id: _,
                        output_index: _,
                        summary_index: _,
                    } => {
                        reasoning_text.push_str(&text);
                        let _ = tx.send(BackendEvent::ReasoningDelta {
                            session_id,
                            request_id,
                            content: text,
                        });
                    }
                    ResponseStreamEvent::OutputItemAdded {
                        item,
                        sequence_number: _,
                        output_index: _,
                    } => {
                        // Handle function_call items (Responses API style)
                        if item.item_type == "function_call" {
                            // Use item.id as the key (consistent with item_id in delta events)
                            let key_id = if !item.id.is_empty() {
                                item.id.clone()
                            } else if !item.call_id.is_empty() {
                                item.call_id.clone()
                            } else {
                                continue;
                            };
                            let name = if !item.name.is_empty() {
                                item.name.clone()
                            } else {
                                continue;
                            };
                            let mut builder = ToolCallBuilder::new(key_id.clone(), name.clone());
                            // Add initial arguments if present
                            if !item.arguments.is_empty() {
                                builder.append_arguments(&item.arguments);
                            }
                            tool_calls.insert(key_id.clone(), builder);
                        }
                    }
                    ResponseStreamEvent::OutputItemDone {
                        item,
                        sequence_number: _,
                        output_index: _,
                    } => {
                        // Handle function_call items - send final ToolCallUpdated
                        if item.item_type == "function_call" {
                            let key_id = if !item.id.is_empty() {
                                item.id.clone()
                            } else if !item.call_id.is_empty() {
                                item.call_id.clone()
                            } else {
                                continue;
                            };
                            if let Some(builder) = tool_calls.get(&key_id)
                                && let Some(arguments) = builder.arguments()
                            {
                                let call = tidev_session::session::ToolCall {
                                    id: key_id.clone(),
                                    name: builder.name().to_string(),
                                    arguments: arguments.to_string(),
                                    thought_signature: None,
                                };
                                let _ = tx.send(BackendEvent::ToolCallUpdated {
                                    session_id,
                                    request_id,
                                    tool_call: call,
                                });
                            }
                        }
                        // Extract finish reason from message items
                        if let Some(reason) = &item.finish_reason {
                            finish_reason = Some(reason.clone());
                        }
                    }
                    ResponseStreamEvent::ContentPartAdded {
                        content_part,
                        sequence_number: _,
                        output_index: _,
                        content_index: _,
                    } => {
                        if content_part.part_type.as_str() == "tool_use"
                            && let Some(name) = &content_part.name
                        {
                            let call_id = content_part
                                .id
                                .clone()
                                .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));
                            tool_calls.insert(
                                call_id.clone(),
                                ToolCallBuilder::new(call_id.clone(), name.clone()),
                            );
                        }
                    }
                    ResponseStreamEvent::ReasoningPartAdded {
                        part: _,
                        sequence_number: _,
                        item_id: _,
                        output_index: _,
                        content_index: _,
                    } => {
                        // Reasoning part added, handled by ReasoningTextDelta
                    }
                    ResponseStreamEvent::ReasoningPartDone {
                        sequence_number: _,
                        item_id: _,
                        output_index: _,
                        content_index: _,
                    } => {
                        // Reasoning part done
                    }
                    ResponseStreamEvent::ReasoningSummaryPartAdded {
                        part: _,
                        sequence_number: _,
                        item_id: _,
                        output_index: _,
                        summary_index: _,
                    } => {
                        // Reasoning summary part added
                    }
                    ResponseStreamEvent::FunctionCallArgumentsDelta {
                        call_id,
                        call_name: _,
                        arguments,
                        sequence_number: _,
                        output_index: _,
                        item_id,
                    } => {
                        // Use item_id as the key when call_id is empty
                        let key_id = if call_id.is_empty() {
                            item_id.clone()
                        } else {
                            call_id.clone()
                        };
                        // Only accumulate arguments, don't send ToolCallUpdated here
                        if let Some(builder) = tool_calls.get_mut(&key_id) {
                            builder.append_arguments(&arguments);
                        }
                    }
                    ResponseStreamEvent::FunctionCallArgumentsDone {
                        call_id: _,
                        call_name: _,
                        sequence_number: _,
                        output_index: _,
                        item_id: _,
                    } => {
                        // Only accumulate arguments, final ToolCallUpdated is sent in OutputItemDone
                    }
                    ResponseStreamEvent::ResponseCreated {
                        response: _,
                        sequence_number: _,
                    }
                    | ResponseStreamEvent::ResponseInProgress {
                        response: _,
                        sequence_number: _,
                    }
                    | ResponseStreamEvent::ResponseCompleted {
                        response: _,
                        sequence_number: _,
                    }
                    | ResponseStreamEvent::ResponseQueued {
                        response: _,
                        sequence_number: _,
                    }
                    | ResponseStreamEvent::OutputTextDone {
                        sequence_number: _,
                        output_index: _,
                        content_index: _,
                    }
                    | ResponseStreamEvent::RefusalDone {
                        sequence_number: _,
                        output_index: _,
                        content_index: _,
                    }
                    | ResponseStreamEvent::ReasoningDone {
                        sequence_number: _,
                        output_index: _,
                        content_index: _,
                    }
                    | ResponseStreamEvent::ReasoningTextDone {
                        text: _,
                        sequence_number: _,
                        item_id: _,
                        output_index: _,
                        content_index: _,
                    }
                    | ResponseStreamEvent::ReasoningSummaryPartDone {
                        sequence_number: _,
                        item_id: _,
                        output_index: _,
                        summary_index: _,
                    }
                    | ResponseStreamEvent::ContentPartDone {
                        content_part: _,
                        sequence_number: _,
                        output_index: _,
                        content_index: _,
                    }
                    | ResponseStreamEvent::FileSearchCallInProgress {
                        sequence_number: _,
                        output_index: _,
                        item_id: _,
                    }
                    | ResponseStreamEvent::FileSearchCallSearching {
                        sequence_number: _,
                        output_index: _,
                        item_id: _,
                    }
                    | ResponseStreamEvent::FileSearchCallCompleted {
                        sequence_number: _,
                        output_index: _,
                        item_id: _,
                    }
                    | ResponseStreamEvent::WebSearchCallInProgress {
                        sequence_number: _,
                        output_index: _,
                        item_id: _,
                    }
                    | ResponseStreamEvent::WebSearchCallSearching {
                        sequence_number: _,
                        output_index: _,
                        item_id: _,
                    }
                    | ResponseStreamEvent::WebSearchCallCompleted {
                        sequence_number: _,
                        output_index: _,
                        item_id: _,
                    }
                    | ResponseStreamEvent::ResponseIncomplete {
                        response: _,
                        sequence_number: _,
                    } => {
                        // These events don't need special handling
                    }
                    ResponseStreamEvent::ResponseFailed {
                        response: _,
                        sequence_number: _,
                    } => {
                        log_error!("openai responses stream failed");
                        return Err(anyhow::anyhow!("Response stream failed"));
                    }
                    ResponseStreamEvent::Error { message, code } => {
                        log_error!(
                            "openai responses stream error: code={:?} message={}",
                            code,
                            message
                        );
                        return Err(anyhow::anyhow!("Response stream error: {}", message));
                    }
                }
            }
        }
    }

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
    Ok(())
}

pub(crate) async fn complete_responses(
    http: &Client,
    model: LlmProviderConfig,
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    save_request_body: bool,
    max_request_files: usize,
) -> Result<String> {
    let api_key = model
        .api_key
        .clone()
        .with_context(|| format!("missing API key for provider '{}'", model.provider_id))?;

    let request = build_responses_request(&model, messages, false, &tools)?;
    let request_body = serde_json::to_string(&request).unwrap_or_default();
    let request_body_size = request_body.len();
    save_request_for_debugging(&request_body, save_request_body, max_request_files);

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

    // Check for error in response
    if let Some(error) = response.error {
        return Err(anyhow::anyhow!(
            "API error: {} - {}",
            error.code,
            error.message
        ));
    }

    // Check for result error
    if let Some(result) = response.result
        && result.result_type == "error"
    {
        return Err(anyhow::anyhow!("API result error"));
    }

    // Extract text content from message output items
    let content = response
        .output
        .into_iter()
        .find_map(|output| {
            if output.kind == "message" {
                output.content.into_iter().find_map(|part| {
                    if part.kind == "output_text" || part.kind == "text" {
                        Some(part.text.unwrap_or_default())
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        })
        .unwrap_or_default();

    Ok(content)
}

fn build_responses_request(
    model: &LlmProviderConfig,
    messages: Vec<Message>,
    stream: bool,
    tools: &[ToolDefinition],
) -> Result<ResponsesRequest> {
    // System prompt comes from the model config directly.
    // No context summary merging needed — compaction summaries are now
    // User messages inserted at the compression boundary, not System messages.
    let instructions = if model.system_prompt_str().trim().is_empty() {
        None
    } else {
        Some(model.system_prompt.clone().unwrap_or_default())
    };

    // Build conversation history as a string (this backend only supports string input)
    let mut conversation_parts: Vec<String> = Vec::new();
    for message in &messages {
        if message.streaming {
            continue;
        }

        match message.role {
            MessageRole::System => {}
            MessageRole::User => {
                let text = message_text_with_file_references(message);
                if !text.is_empty() {
                    conversation_parts.push(format!("User: {}", text));
                }
            }
            MessageRole::Assistant => {
                let text = message_text_with_file_references(message);
                let has_tool_calls = !message.tool_calls.is_empty();

                if has_tool_calls {
                    let mut combined = text;
                    for tool_call in &message.tool_calls {
                        combined.push_str(&format!(
                            "\n[Tool: {}]\nArguments: {}",
                            tool_call.name, tool_call.arguments
                        ));
                    }
                    if !combined.is_empty() {
                        conversation_parts.push(format!("Assistant: {}", combined));
                    }
                } else if !text.is_empty() {
                    conversation_parts.push(format!("Assistant: {}", text));
                }
            }
            MessageRole::Tool => {
                let text = message_text_with_file_references(message);
                if !text.is_empty() {
                    conversation_parts.push(format!("Tool: {}", text));
                }
            }
            MessageRole::Error => {}
            MessageRole::Shell => {}
        }
    }

    let input = conversation_parts.join("\n\n");

    let chat_tools = if tools.is_empty() {
        None
    } else {
        Some(tools.iter().map(ResponseTool::from).collect())
    };

    Ok(ResponsesRequest {
        model: model.request_model_id.clone().unwrap_or_default(),
        instructions,
        input,
        tools: chat_tools,
        temperature: model.temperature,
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

fn finalize_turn(
    assistant_text: &str,
    reasoning_text: &str,
    finish_reason: &Option<String>,
    tool_calls: &BTreeMap<String, ToolCallBuilder>,
) -> tidev_session::session::AssistantTurn {
    let tool_calls = tool_calls
        .values()
        .map(|builder| ToolCall {
            id: builder.id().to_string(),
            name: builder.name().to_string(),
            arguments: builder.arguments().unwrap_or_default().to_string(),
            thought_signature: None,
        })
        .collect::<Vec<_>>();

    let final_finish_reason = finish_reason.clone().unwrap_or_else(|| {
        if tool_calls.is_empty() {
            "stop".to_string()
        } else {
            "tool_calls".to_string()
        }
    });

    tidev_session::session::AssistantTurn {
        content: assistant_text.to_string(),
        reasoning: reasoning_text.to_string(),
        tool_calls,
        finish_reason: Some(final_finish_reason),
        ..Default::default()
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

// ============================================================================
// Response data structures
// ============================================================================

#[derive(Clone, Debug, Serialize)]
struct ResponsesRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    input: String,
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

// ============================================================================
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

/// SSE event types for Responses API streaming
#[derive(Clone, Debug, Deserialize)]
#[serde(from = "ResponseStreamEventRaw")]
#[allow(dead_code)]
enum ResponseStreamEvent {
    ResponseCreated {
        #[serde(default)]
        response: ResponseStreamResponse,
        #[serde(default)]
        sequence_number: u64,
    },
    ResponseInProgress {
        #[serde(default)]
        response: ResponseStreamResponse,
        #[serde(default)]
        sequence_number: u64,
    },
    ResponseCompleted {
        #[serde(default)]
        response: ResponseStreamResponse,
        #[serde(default)]
        sequence_number: u64,
    },
    ResponseFailed {
        #[serde(default)]
        response: ResponseStreamResponse,
        #[serde(default)]
        sequence_number: u64,
    },
    ResponseIncomplete {
        #[serde(default)]
        response: ResponseStreamResponse,
        #[serde(default)]
        sequence_number: u64,
    },
    ResponseQueued {
        #[serde(default)]
        response: ResponseStreamResponse,
        #[serde(default)]
        sequence_number: u64,
    },
    OutputItemAdded {
        item: ResponseStreamItem,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
    },
    OutputItemDone {
        #[serde(default)]
        item: ResponseStreamItem,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
    },
    ContentPartAdded {
        #[serde(default)]
        content_part: ResponseStreamContentPart,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ContentPartDone {
        #[serde(default)]
        content_part: ResponseStreamContentPart,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    OutputTextDelta {
        delta: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    OutputTextDone {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    RefusalDelta {
        delta: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    RefusalDone {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningDelta {
        delta: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningDone {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningTextDelta {
        delta: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningTextDone {
        #[serde(default)]
        text: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningPartAdded {
        #[serde(default)]
        part: ResponseStreamReasoningPart,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningPartDone {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningSummaryTextDelta {
        #[serde(rename = "summary")]
        summary_delta: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        summary_index: u32,
    },
    ReasoningSummaryTextDone {
        #[serde(default)]
        text: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        summary_index: u32,
    },
    ReasoningSummaryPartAdded {
        #[serde(default)]
        part: ResponseStreamReasoningPart,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        summary_index: u32,
    },
    ReasoningSummaryPartDone {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        summary_index: u32,
    },
    FunctionCallArgumentsDelta {
        #[serde(rename = "id")]
        call_id: String,
        #[serde(rename = "name")]
        call_name: Option<String>,
        arguments: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    FunctionCallArgumentsDone {
        #[serde(rename = "id")]
        call_id: String,
        #[serde(rename = "name")]
        call_name: Option<String>,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    FileSearchCallInProgress {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    FileSearchCallSearching {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    FileSearchCallCompleted {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    WebSearchCallInProgress {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    WebSearchCallSearching {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    WebSearchCallCompleted {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    Error {
        message: String,
        #[serde(default)]
        code: Option<String>,
    },
}

/// Raw JSON structure for parsing SSE events
#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamEventRaw {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    response: ResponseStreamResponse,
    #[serde(default)]
    item: ResponseStreamItem,
    #[serde(default)]
    content_part: ResponseStreamContentPart,
    #[serde(default)]
    part: ResponseStreamReasoningPart,
    #[serde(default)]
    delta: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    index: u32,
    #[serde(default)]
    content_index: u32,
    #[serde(default)]
    summary_index: u32,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
    #[serde(default)]
    item_id: String,
    #[serde(default)]
    output_index: u32,
    #[serde(default)]
    sequence_number: u64,
    #[serde(default)]
    output: Vec<ResponseStreamItem>,
    #[serde(default)]
    error: ResponseStreamError,
    #[serde(default)]
    incomplete_details: ResponseStreamIncompleteDetails,
    #[serde(default)]
    message: String,
    #[serde(default)]
    code: Option<String>,
}

impl From<ResponseStreamEventRaw> for ResponseStreamEvent {
    fn from(raw: ResponseStreamEventRaw) -> Self {
        match raw.event_type.as_str() {
            "response.created" => ResponseStreamEvent::ResponseCreated {
                response: raw.response,
                sequence_number: raw.sequence_number,
            },
            "response.in_progress" => ResponseStreamEvent::ResponseInProgress {
                response: raw.response,
                sequence_number: raw.sequence_number,
            },
            "response.completed" => ResponseStreamEvent::ResponseCompleted {
                response: raw.response,
                sequence_number: raw.sequence_number,
            },
            "response.failed" => ResponseStreamEvent::ResponseFailed {
                response: raw.response,
                sequence_number: raw.sequence_number,
            },
            "response.incomplete" => ResponseStreamEvent::ResponseIncomplete {
                response: raw.response,
                sequence_number: raw.sequence_number,
            },
            "response.queued" => ResponseStreamEvent::ResponseQueued {
                response: raw.response,
                sequence_number: raw.sequence_number,
            },
            "response.output_item.added" => ResponseStreamEvent::OutputItemAdded {
                item: raw.item,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
            },
            "response.output_item.done" => ResponseStreamEvent::OutputItemDone {
                item: raw.item,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
            },
            "response.content_part.added" => ResponseStreamEvent::ContentPartAdded {
                content_part: raw.content_part,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.content_part.done" => ResponseStreamEvent::ContentPartDone {
                content_part: raw.content_part,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.output_text.delta" => ResponseStreamEvent::OutputTextDelta {
                delta: raw.delta,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.output_text.done" => ResponseStreamEvent::OutputTextDone {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.refusal.delta" => ResponseStreamEvent::RefusalDelta {
                delta: raw.delta,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.refusal.done" => ResponseStreamEvent::RefusalDone {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning.delta" => ResponseStreamEvent::ReasoningDelta {
                delta: raw.delta,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning.done" => ResponseStreamEvent::ReasoningDone {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning_text.delta" => ResponseStreamEvent::ReasoningTextDelta {
                delta: raw.delta,
                sequence_number: raw.sequence_number,
                item_id: raw.item_id,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning_text.done" => ResponseStreamEvent::ReasoningTextDone {
                text: raw.text,
                sequence_number: raw.sequence_number,
                item_id: raw.item_id,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning_part.added" => ResponseStreamEvent::ReasoningPartAdded {
                part: raw.part,
                sequence_number: raw.sequence_number,
                item_id: raw.item_id,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning_part.done" => ResponseStreamEvent::ReasoningPartDone {
                sequence_number: raw.sequence_number,
                item_id: raw.item_id,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning_summary_text.delta" => {
                ResponseStreamEvent::ReasoningSummaryTextDelta {
                    summary_delta: raw.summary,
                    sequence_number: raw.sequence_number,
                    item_id: raw.item_id,
                    output_index: raw.output_index,
                    summary_index: raw.summary_index,
                }
            }
            "response.reasoning_summary_text.done" => {
                ResponseStreamEvent::ReasoningSummaryTextDone {
                    text: raw.text,
                    sequence_number: raw.sequence_number,
                    item_id: raw.item_id,
                    output_index: raw.output_index,
                    summary_index: raw.summary_index,
                }
            }
            "response.reasoning_summary_part.added" => {
                ResponseStreamEvent::ReasoningSummaryPartAdded {
                    part: raw.part,
                    sequence_number: raw.sequence_number,
                    item_id: raw.item_id,
                    output_index: raw.output_index,
                    summary_index: raw.summary_index,
                }
            }
            "response.reasoning_summary_part.done" => {
                ResponseStreamEvent::ReasoningSummaryPartDone {
                    sequence_number: raw.sequence_number,
                    item_id: raw.item_id,
                    output_index: raw.output_index,
                    summary_index: raw.summary_index,
                }
            }
            "response.function_call_arguments.delta" => {
                ResponseStreamEvent::FunctionCallArgumentsDelta {
                    call_id: raw.id,
                    call_name: if raw.name.is_empty() {
                        None
                    } else {
                        Some(raw.name)
                    },
                    arguments: raw.delta,
                    sequence_number: raw.sequence_number,
                    output_index: raw.output_index,
                    item_id: raw.item_id,
                }
            }
            "response.function_call_arguments.done" => {
                ResponseStreamEvent::FunctionCallArgumentsDone {
                    call_id: raw.id,
                    call_name: if raw.name.is_empty() {
                        None
                    } else {
                        Some(raw.name)
                    },
                    sequence_number: raw.sequence_number,
                    output_index: raw.output_index,
                    item_id: raw.item_id,
                }
            }
            "response.file_search_call.in_progress" => {
                ResponseStreamEvent::FileSearchCallInProgress {
                    sequence_number: raw.sequence_number,
                    output_index: raw.output_index,
                    item_id: raw.item_id,
                }
            }
            "response.file_search_call.searching" => ResponseStreamEvent::FileSearchCallSearching {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                item_id: raw.item_id,
            },
            "response.file_search_call.completed" => ResponseStreamEvent::FileSearchCallCompleted {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                item_id: raw.item_id,
            },
            "response.web_search_call.in_progress" => {
                ResponseStreamEvent::WebSearchCallInProgress {
                    sequence_number: raw.sequence_number,
                    output_index: raw.output_index,
                    item_id: raw.item_id,
                }
            }
            "response.web_search_call.searching" => ResponseStreamEvent::WebSearchCallSearching {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                item_id: raw.item_id,
            },
            "response.web_search_call.completed" => ResponseStreamEvent::WebSearchCallCompleted {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                item_id: raw.item_id,
            },
            "error" => ResponseStreamEvent::Error {
                message: raw.message,
                code: raw.code,
            },
            _ => ResponseStreamEvent::Error {
                message: format!("Unknown event type: {}", raw.event_type),
                code: None,
            },
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamResponse {
    #[serde(default)]
    id: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    created_at: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamItem {
    #[serde(rename = "type")]
    #[serde(default)]
    item_type: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    call_id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
    #[serde(default)]
    content: Option<Vec<ResponseStreamContentPart>>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamContentPart {
    #[serde(rename = "type")]
    #[serde(default)]
    part_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    index: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamReasoningPart {
    #[serde(rename = "type")]
    #[serde(default)]
    part_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    summary: Option<Vec<ResponseStreamReasoningStep>>,
    #[serde(default)]
    last_summary: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamReasoningStep {
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    param: Option<String>,
    #[serde(default)]
    error: ResponseStreamErrorDetail,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamErrorDetail {
    #[serde(rename = "type", default)]
    r#type: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamIncompleteDetails {
    #[serde(rename = "type")]
    #[serde(default)]
    incomplete_type: String,
    #[serde(default)]
    reason: String,
}

/// Usage stats from streaming response
#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamUsage {
    #[serde(rename = "input_tokens")]
    input_tokens: u32,
    #[serde(rename = "completion_tokens")]
    output_tokens: u32,
    #[serde(rename = "total_tokens")]
    total_tokens: u32,
    #[serde(rename = "response_tokens")]
    response_tokens: Option<u32>,
    #[serde(rename = "thinking_tokens")]
    thinking_tokens: Option<u32>,
}

/// Non-streaming response structures
#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
struct ResponsesCompleteResponse {
    #[serde(default)]
    id: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    created_at: u64,
    #[serde(default)]
    error: Option<ResponseStreamError>,
    #[serde(default)]
    result: Option<ResponseResult>,
    #[serde(default)]
    output: Vec<ResponseOutputItem>,
    #[serde(default)]
    usage: ResponseStreamUsage,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct ResponseResult {
    #[serde(rename = "type")]
    #[serde(default)]
    result_type: String,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
struct ResponseOutputItem {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    content: Vec<ResponseOutputContent>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
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
    call_id: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
    #[serde(default)]
    index: Option<u32>,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ApiType;
    use tidev_session::session::{Message, MessageRole};

    #[test]
    fn test_responses_request_basic() {
        let model = LlmProviderConfig {
            provider_id: "openai-responses".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: true,
            system_prompt: Some("You are helpful.".to_string()),
            api_key: None,
            extra_body: None,
            thinking_level: tidev_types::reasoning::ThinkingLevelType::None,
        };

        let messages = vec![Message::new(MessageRole::User, "Hello")];

        let request = build_responses_request(&model, messages, true, &[]).unwrap();

        assert_eq!(request.model, "gpt-4.5");
        assert_eq!(request.instructions, Some("You are helpful.".to_string()));
        assert!(request.stream);
        assert_eq!(request.input, "User: Hello");
    }

    #[test]
    fn test_responses_request_with_system_prompt() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: false,
            system_prompt: Some("Base system prompt".to_string()),
            api_key: None,
            extra_body: None,
            thinking_level: tidev_types::reasoning::ThinkingLevelType::None,
        };

        // System messages in the list are now skipped; only model.system_prompt is used
        let messages = vec![
            Message::new(MessageRole::System, "Context summary"),
            Message::new(MessageRole::User, "Hello"),
        ];

        let request = build_responses_request(&model, messages, false, &[]).unwrap();

        assert!(request.instructions.is_some());
        let instructions = request.instructions.unwrap();
        assert!(instructions.contains("Base system prompt"));
        assert!(
            !instructions.contains("Context summary"),
            "System messages should no longer be merged into instructions"
        );
    }

    #[test]
    fn test_responses_request_assistant_and_tool_messages() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: false,
            system_prompt: Some("You are helpful.".to_string()),
            api_key: None,
            extra_body: None,
            thinking_level: tidev_types::reasoning::ThinkingLevelType::None,
        };

        let messages = vec![
            Message::new(MessageRole::User, "Run command"),
            Message::new(MessageRole::Assistant, "I'll help you run a command."),
            Message::new(MessageRole::Tool, "Tool result: success"),
        ];

        let request = build_responses_request(&model, messages, false, &[]).unwrap();

        // Input should be a string with conversation history
        assert!(request.input.contains("User: Run command"));
        assert!(
            request
                .input
                .contains("Assistant: I'll help you run a command.")
        );
        assert!(request.input.contains("Tool: Tool result: success"));
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
        };

        let response_tool = ResponseTool::from(&tool);

        assert_eq!(response_tool.kind, "function");
        assert_eq!(response_tool.name, "bash");
        assert!(!response_tool.description.is_empty());
    }
}
