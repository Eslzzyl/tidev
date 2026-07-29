use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::{types::LlmProviderConfig, types::ToolDefinition};
use tidev_types::message::{BackendEvent, Message, MessageAttachment, MessageRole, ToolCall};

use log::{debug as log_debug, error as log_error};

use crate::attachments::{image_attachments, message_text_with_file_references};
use crate::debug::{
    save_complete_response_for_debugging, save_raw_response_for_debugging,
    save_request_for_debugging,
};
use crate::error::{NetworkError, classify_response_status};
use crate::think_parser::strip_think_tags;
use std::time::Duration;

/// Responses API endpoint
const RESPONSES_ENDPOINT: &str = "/responses";
const RESPONSES_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

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
    save_response_body: bool,
    max_response_files: usize,
) -> Result<()> {
    let api_key = model
        .api_key
        .clone()
        .with_context(|| format!("missing API key for provider '{}'", model.provider_id))?;

    let request =
        build_responses_request(&model, messages, true, &tools, Some(session_id.to_string()))?;
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
        .header("session-id", session_id.to_string())
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
    let mut buffer = Vec::new();
    let mut assistant_text = String::new();
    let mut reasoning_text = String::new();
    let mut finish_reason: Option<String> = None;
    let mut tool_calls: BTreeMap<String, ToolCallBuilder> = BTreeMap::new();
    let mut responses_output_items: Vec<serde_json::Value> = Vec::new();
    let mut first_delta_time: Option<std::time::Instant> = None;
    let mut sse_parser = SseParser::default();
    let mut raw_payloads: Vec<String> = Vec::new();

    use futures_util::StreamExt;

    loop {
        let next_chunk = tokio::time::timeout(RESPONSES_STREAM_IDLE_TIMEOUT, stream.next())
            .await
            .map_err(|_| NetworkError::Retryable {
                message: format!(
                    "Responses stream idle timeout after {} seconds",
                    RESPONSES_STREAM_IDLE_TIMEOUT.as_secs()
                ),
            })?;
        let Some(chunk) = next_chunk else {
            break;
        };
        let chunk = chunk?;
        buffer.extend_from_slice(&chunk);

        while let Some(line_end) = buffer.iter().position(|&b| b == b'\n') {
            let tail = &buffer[..line_end];
            let line = if tail.last() == Some(&b'\r') {
                String::from_utf8_lossy(&tail[..tail.len() - 1]).into_owned()
            } else {
                String::from_utf8_lossy(tail).into_owned()
            };
            buffer.drain(..=line_end);

            let Some(payload) = sse_parser.push_line(&line) else {
                continue;
            };
            let payload = payload.trim().to_string();

            raw_payloads.push(payload.clone());

            if payload == "[DONE]" {
                save_raw_response_for_debugging(
                    session_id,
                    request_id,
                    &raw_payloads,
                    save_response_body,
                    max_response_files,
                );
                let turn = finalize_turn(
                    &assistant_text,
                    &reasoning_text,
                    &finish_reason,
                    &tool_calls,
                    &responses_output_items,
                );
                let _ = tx.send(BackendEvent::Finished {
                    session_id,
                    request_id,
                    turn: Box::new(turn),
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
                    let cleaned = strip_think_tags(&delta);
                    if !cleaned.is_empty() {
                        reasoning_text.push_str(&cleaned);
                        let _ = tx.send(BackendEvent::ReasoningDelta {
                            session_id,
                            request_id,
                            content: cleaned,
                        });
                    }
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
                    let cleaned = strip_think_tags(&delta);
                    if !cleaned.is_empty() {
                        reasoning_text.push_str(&cleaned);
                        let _ = tx.send(BackendEvent::ReasoningDelta {
                            session_id,
                            request_id,
                            content: cleaned,
                        });
                    }
                }
                ResponseStreamEvent::ReasoningSummaryTextDelta {
                    summary_delta,
                    sequence_number: _,
                    item_id: _,
                    output_index: _,
                    summary_index: _,
                } => {
                    let cleaned = strip_think_tags(&summary_delta);
                    if !cleaned.is_empty() {
                        reasoning_text.push_str(&cleaned);
                        let _ = tx.send(BackendEvent::ReasoningDelta {
                            session_id,
                            request_id,
                            content: cleaned,
                        });
                    }
                }
                ResponseStreamEvent::ReasoningSummaryTextDone {
                    text: _,
                    sequence_number: _,
                    item_id: _,
                    output_index: _,
                    summary_index: _,
                } => {}
                ResponseStreamEvent::OutputItemAdded {
                    item,
                    sequence_number: _,
                    output_index: _,
                } => {
                    // Handle function_call items (Responses API style)
                    if item.item_type == "function_call" {
                        // Extract the actual call_id for tool call pairing.
                        let call_id = if !item.call_id.is_empty() {
                            item.call_id.clone()
                        } else if !item.id.is_empty() {
                            item.id.clone()
                        } else {
                            continue;
                        };
                        let name = if !item.name.is_empty() {
                            item.name.clone()
                        } else {
                            continue;
                        };
                        // Use item.id as the map key (consistent with
                        // item_id in delta events).
                        let key_id = if !item.id.is_empty() {
                            item.id.clone()
                        } else {
                            call_id.clone()
                        };
                        let mut builder = ToolCallBuilder::new(call_id, name.clone());
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
                        if !item.arguments.is_empty()
                            && let Some(builder) = tool_calls.get_mut(&key_id)
                        {
                            // The done event is authoritative. It may contain
                            // the complete argument string even when a delta
                            // was missed or never emitted.
                            builder.set_arguments(&item.arguments);
                        }
                        if let Some(builder) = tool_calls.get(&key_id) {
                            let arguments = builder.arguments().unwrap_or_default();
                            let call = tidev_types::message::ToolCall {
                                id: builder.id().to_string(),
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
                    if let Ok(raw_item) = serde_json::to_value(&item) {
                        responses_output_items.push(raw_item);
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
                    call_id,
                    call_name: _,
                    arguments,
                    sequence_number: _,
                    output_index: _,
                    item_id,
                } => {
                    let key_id = if call_id.is_empty() { item_id } else { call_id };
                    if !arguments.is_empty()
                        && let Some(builder) = tool_calls.get_mut(&key_id)
                    {
                        builder.set_arguments(&arguments);
                    }
                }
                ResponseStreamEvent::ResponseCompleted {
                    response,
                    sequence_number: _,
                } => {
                    for item in response.output.iter() {
                        if let Ok(raw_item) = serde_json::to_value(item)
                            && !responses_output_items.contains(&raw_item)
                        {
                            responses_output_items.push(raw_item);
                        }
                    }
                    if let Some(usage) = response.usage {
                        let cached_tokens = usage
                            .input_tokens_details
                            .as_ref()
                            .map(|d| d.cached_tokens)
                            .unwrap_or(0);
                        let duration_ms =
                            first_delta_time.map(|start| start.elapsed().as_millis() as u64);
                        let _ = tx.send(BackendEvent::UsageStats {
                            session_id,
                            request_id,
                            input_tokens: usage.input_tokens,
                            output_tokens: usage.output_tokens,
                            total_tokens: usage.total_tokens,
                            cache_read_tokens: cached_tokens,
                            cache_write_tokens: 0,
                            model_id: format!("{}:{}", model.provider_id, model.model_id),
                            duration_ms,
                        });
                    }
                    save_raw_response_for_debugging(
                        session_id,
                        request_id,
                        &raw_payloads,
                        save_response_body,
                        max_response_files,
                    );
                    let turn = finalize_turn(
                        &assistant_text,
                        &reasoning_text,
                        &finish_reason,
                        &tool_calls,
                        &responses_output_items,
                    );
                    let _ = tx.send(BackendEvent::Finished {
                        session_id,
                        request_id,
                        turn: Box::new(turn),
                    });
                    return Ok(());
                }
                ResponseStreamEvent::ResponseCreated {
                    response: _,
                    sequence_number: _,
                }
                | ResponseStreamEvent::ResponseInProgress {
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
                } => {}
                ResponseStreamEvent::ResponseIncomplete {
                    response,
                    sequence_number: _,
                } => {
                    let detail = response
                        .incomplete_details
                        .as_ref()
                        .map(|details| {
                            if details.reason.is_empty() {
                                details.incomplete_type.clone()
                            } else {
                                details.reason.clone()
                            }
                        })
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| "unknown reason".to_string());
                    return Err(NetworkError::NonRetryable {
                        message: format!("Responses response incomplete: {detail}"),
                    }
                    .into());
                }
                ResponseStreamEvent::ResponseFailed {
                    response,
                    sequence_number: _,
                } => {
                    let (message, code) = response_error_details(&response);
                    log_error!(
                        "openai responses stream failed: code={:?} message={}",
                        code,
                        message
                    );
                    return Err(classify_responses_stream_error(message, code).into());
                }
                ResponseStreamEvent::Error { message, code } => {
                    log_error!(
                        "openai responses stream error: code={:?} message={}",
                        code,
                        message
                    );
                    return Err(classify_responses_stream_error(message, code).into());
                }
                ResponseStreamEvent::Unknown { event_type } => {
                    log_debug!("ignoring unknown OpenAI Responses event: {}", event_type);
                }
            }
        }
    }

    save_raw_response_for_debugging(
        session_id,
        request_id,
        &raw_payloads,
        save_response_body,
        max_response_files,
    );

    Err(NetworkError::Retryable {
        message: "Responses stream closed before response.completed".to_string(),
    }
    .into())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn complete_responses(
    http: &Client,
    session_id: Uuid,
    request_id: u64,
    model: LlmProviderConfig,
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    tx: Option<&UnboundedSender<BackendEvent>>,
    save_request_body: bool,
    max_request_files: usize,
    save_response_body: bool,
    max_response_files: usize,
) -> Result<String> {
    let api_key = model
        .api_key
        .clone()
        .with_context(|| format!("missing API key for provider '{}'", model.provider_id))?;

    let request = build_responses_request(&model, messages, false, &tools, None)?;
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

    let body_text = response.text().await?;
    save_complete_response_for_debugging(&body_text, save_response_body, max_response_files);

    let response: ResponsesCompleteResponse = serde_json::from_str(&body_text)?;

    // Check for error in response
    if let Some(error) = response.error {
        let message = if error.message.is_empty() {
            "Responses API returned an error".to_string()
        } else {
            error.message
        };
        let code = (!error.code.is_empty()).then_some(error.code);
        return Err(classify_responses_stream_error(message, code).into());
    }

    // Check for result error
    if let Some(result) = response.result
        && result.result_type == "error"
    {
        return Err(anyhow::anyhow!("API result error"));
    }

    // Preserve all output text parts, including text split across multiple
    // message items. Function calls are intentionally not exposed by this
    // string-only API, but must not hide later assistant text.
    if let Some(usage) = response.usage.as_ref()
        && let Some(tx) = tx
    {
        let cached_tokens = usage
            .input_tokens_details
            .as_ref()
            .map(|details| details.cached_tokens)
            .unwrap_or(0);
        let _ = tx.send(BackendEvent::UsageStats {
            session_id,
            request_id,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            cache_read_tokens: cached_tokens,
            cache_write_tokens: 0,
            model_id: format!("{}:{}", model.provider_id, model.model_id),
            duration_ms: None,
        });
    }

    let content = response
        .output
        .into_iter()
        .filter(|output| output.kind == "message")
        .flat_map(|output| output.content)
        .filter(|part| part.kind == "output_text" || part.kind == "text")
        .filter_map(|part| part.text)
        .collect::<Vec<_>>()
        .join("");

    Ok(content)
}

fn build_responses_request(
    model: &LlmProviderConfig,
    messages: Vec<Message>,
    stream: bool,
    tools: &[ToolDefinition],
    prompt_cache_key: Option<String>,
) -> Result<ResponsesRequest> {
    // System prompt comes from the model config directly.
    // No context summary merging needed — compaction summaries are now
    // User messages inserted at the compression boundary, not System messages.
    let instructions = if model.system_prompt_str().trim().is_empty() {
        None
    } else {
        Some(model.system_prompt.clone().unwrap_or_default())
    };

    // Build conversation history as an array of input items.
    // This supports image attachments in user and tool messages.
    let mut input_items: Vec<serde_json::Value> = Vec::new();
    for message in &messages {
        if message.streaming {
            continue;
        }

        match message.role {
            MessageRole::System => {}
            MessageRole::User => {
                let text = message_text_with_file_references(message);
                let images: Vec<&MessageAttachment> = image_attachments(message).collect();

                let mut content = Vec::new();
                if !text.trim().is_empty() {
                    content.push(serde_json::json!({
                        "type": "input_text",
                        "text": text,
                    }));
                }
                for attachment in images {
                    if let MessageAttachment::Image { mime, data, .. } = attachment {
                        let b64 = BASE64.encode(data);
                        content.push(serde_json::json!({
                            "type": "input_image",
                            "image_url": format!("data:{};base64,{}", mime, b64),
                            "detail": "auto",
                        }));
                    }
                }

                if !content.is_empty() {
                    input_items.push(serde_json::json!({
                        "type": "message",
                        "role": "user",
                        "content": content,
                    }));
                }
            }
            MessageRole::Assistant => {
                // Replay the provider's completed output items whenever they are
                // available. This preserves reasoning items and encrypted content
                // that cannot be reconstructed from the display text alone.
                if !message.metadata.responses_output_items.is_empty() {
                    input_items.extend(message.metadata.responses_output_items.iter().cloned());
                    continue;
                }

                let text = message_text_with_file_references(message);

                // Emit assistant text as a message item (if non-empty).
                if !text.trim().is_empty() {
                    input_items.push(serde_json::json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": text,
                        }],
                    }));
                }

                // Emit each tool call as a separate function_call item.
                // This is required by the Responses API — tool calls must be
                // distinct items so the server can pair them with
                // function_call_output items via call_id.
                for tool_call in &message.tool_calls {
                    input_items.push(serde_json::json!({
                        "type": "function_call",
                        "call_id": tool_call.id,
                        "name": tool_call.name,
                        "arguments": tool_call.arguments,
                    }));
                }
            }
            MessageRole::Tool => {
                let text = message_text_with_file_references(message);
                let call_id = message.tool_call_id.clone().unwrap_or_default();
                let images: Vec<&MessageAttachment> = image_attachments(message).collect();
                let has_images = !images.is_empty();

                // function_call_output.output supports an array of content
                // parts (input_text, input_image, input_file), so embed
                // images inline — no synthetic user message needed.
                let mut output_parts: Vec<serde_json::Value> = Vec::new();
                if !text.trim().is_empty() {
                    output_parts.push(serde_json::json!({
                        "type": "input_text",
                        "text": text,
                    }));
                }
                for attachment in images {
                    if let MessageAttachment::Image { mime, data, .. } = attachment {
                        let b64 = BASE64.encode(data);
                        output_parts.push(serde_json::json!({
                            "type": "input_image",
                            "image_url": format!(
                                "data:{};base64,{}",
                                mime, b64,
                            ),
                            "detail": "auto",
                        }));
                    }
                }
                if output_parts.is_empty() {
                    output_parts.push(serde_json::json!({
                        "type": "input_text",
                        "text": "",
                    }));
                }

                let output = if !has_images {
                    serde_json::Value::String(text)
                } else {
                    serde_json::Value::Array(output_parts)
                };

                input_items.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output,
                }));
            }
            MessageRole::Error => {}
            MessageRole::Shell => {}
        }
    }

    let input = serde_json::Value::Array(input_items);

    let chat_tools = if tools.is_empty() {
        None
    } else {
        Some(tools.iter().map(ResponseTool::from).collect())
    };

    let thinking = model.thinking_config();
    // Always request encrypted reasoning content so reasoning items can be
    // preserved and replayed across multi-turn conversations. Without this,
    // stateless / non-persisted reasoning items (store=false / ZDR) cannot
    // be looked up on subsequent turns.
    let include = Some(vec!["reasoning.encrypted_content".to_string()]);

    let tool_choice = chat_tools.as_ref().map(|_| "auto".to_string());

    Ok(ResponsesRequest {
        model: model.request_model_id.clone().unwrap_or_default(),
        instructions,
        input,
        tools: chat_tools,
        tool_choice,
        parallel_tool_calls: model.supports_parallel_tool_calls && tools.len() > 1,
        temperature: model.temperature,
        max_output_tokens: Some(model.max_output_tokens),
        store: false,
        stream,
        thinking,
        include,
        prompt_cache_key,
        // Responses reasoning is already represented by the dedicated
        // `reasoning` field above. Do not flatten the thinking-level object a
        // second time into the top-level request.
        extra_body: model.extra_body.clone(),
    })
}

fn finalize_turn(
    assistant_text: &str,
    reasoning_text: &str,
    finish_reason: &Option<String>,
    tool_calls: &BTreeMap<String, ToolCallBuilder>,
    responses_output_items: &[serde_json::Value],
) -> tidev_types::message::AssistantTurn {
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

    tidev_types::message::AssistantTurn {
        content: assistant_text.to_string(),
        reasoning: reasoning_text.to_string(),
        tool_calls,
        finish_reason: Some(final_finish_reason),
        responses_output_items: responses_output_items.to_vec(),
        ..Default::default()
    }
}

#[derive(Default)]
struct SseParser {
    data_lines: Vec<String>,
    event_type: Option<String>,
}

impl SseParser {
    /// Parse one SSE line and return a payload only at an event boundary.
    fn push_line(&mut self, line: &str) -> Option<String> {
        if line.is_empty() {
            let payload = (!self.data_lines.is_empty()).then(|| self.data_lines.join("\n"));
            self.data_lines.clear();
            let _event_type = self.event_type.take();
            return payload;
        }

        if line.starts_with(':') {
            return None;
        }

        if let Some(event_type) = line.strip_prefix("event:") {
            self.event_type = Some(event_type.trim().to_string());
            return None;
        }

        if let Some(data) = line.strip_prefix("data:") {
            self.data_lines
                .push(data.strip_prefix(' ').unwrap_or(data).to_string());
        }
        None
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

    fn set_arguments(&mut self, args: &str) {
        self.arguments.clear();
        self.arguments.push_str(args);
    }
}

fn response_error_details(response: &ResponseStreamResponse) -> (String, Option<String>) {
    response
        .error
        .as_ref()
        .map(|error| {
            let message = if error.message.is_empty() {
                "Responses API returned a failed response".to_string()
            } else {
                error.message.clone()
            };
            let code = if error.code.is_empty() {
                (!error.r#type.is_empty()).then(|| error.r#type.clone())
            } else {
                Some(error.code.clone())
            };
            (message, code)
        })
        .unwrap_or_else(|| ("Responses API returned a failed response".to_string(), None))
}

fn classify_responses_stream_error(message: String, code: Option<String>) -> NetworkError {
    let searchable = format!(
        "{} {}",
        code.as_deref().unwrap_or_default().to_ascii_lowercase(),
        message.to_ascii_lowercase()
    );
    let retryable = [
        "rate_limit",
        "rate limit",
        "server_error",
        "server error",
        "overloaded",
        "temporarily unavailable",
        "timeout",
        "timed out",
        "try again",
    ]
    .iter()
    .any(|marker| searchable.contains(marker));

    if retryable {
        NetworkError::Retryable { message }
    } else {
        NetworkError::NonRetryable { message }
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
    input: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ResponseTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    parallel_tool_calls: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<usize>,
    stream: bool,
    store: bool,
    #[serde(rename = "reasoning", skip_serializing_if = "Option::is_none")]
    thinking: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
    #[serde(flatten)]
    extra_body: Option<serde_json::Value>,
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
        arguments: String,
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
    Unknown {
        event_type: String,
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
                    summary_delta: raw.delta,
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
                    arguments: raw.arguments,
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
            "error" => {
                let message = if !raw.message.is_empty() {
                    raw.message
                } else if !raw.error.message.is_empty() {
                    raw.error.message.clone()
                } else {
                    raw.error.error.message.clone()
                };
                let code = raw
                    .code
                    .or_else(|| (!raw.error.code.is_empty()).then(|| raw.error.code.clone()));
                let code = code.or_else(|| {
                    (!raw.error.error.code.is_empty()).then(|| raw.error.error.code.clone())
                });
                ResponseStreamEvent::Error { message, code }
            }
            _ => ResponseStreamEvent::Unknown {
                event_type: raw.event_type,
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
    #[serde(default)]
    usage: Option<ResponseStreamUsage>,
    #[serde(default)]
    error: Option<ResponseStreamErrorDetail>,
    #[serde(default)]
    incomplete_details: Option<ResponseStreamIncompleteDetails>,
    #[serde(default)]
    output: Vec<ResponseStreamItem>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamItem {
    #[serde(rename = "type", skip_serializing_if = "String::is_empty")]
    #[serde(default)]
    item_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    call_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    arguments: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<Vec<ResponseStreamContentPart>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finish_reason: Option<String>,
    #[serde(flatten, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    extra: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamContentPart {
    #[serde(rename = "type", skip_serializing_if = "String::is_empty")]
    #[serde(default)]
    part_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    index: Option<u32>,
    #[serde(flatten, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    extra: std::collections::BTreeMap<String, serde_json::Value>,
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
    #[serde(rename = "output_tokens")]
    output_tokens: u32,
    #[serde(rename = "total_tokens")]
    total_tokens: u32,
    #[serde(default)]
    input_tokens_details: Option<ResponseStreamUsageInputDetails>,
}

/// Input token details (cached tokens)
#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamUsageInputDetails {
    #[serde(default)]
    cached_tokens: u32,
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
    usage: Option<ResponseStreamUsage>,
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
    use tidev_types::message::{Message, MessageRole};

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
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: Some("You are helpful.".to_string()),
            api_key: None,
            extra_body: None,
            thinking_level: tidev_types::reasoning::ThinkingLevelType::None,
        };

        let messages = vec![Message::new(MessageRole::User, "Hello")];

        let request = build_responses_request(&model, messages, true, &[], None).unwrap();

        assert_eq!(request.model, "gpt-4.5");
        assert_eq!(request.instructions, Some("You are helpful.".to_string()));
        assert!(request.stream);
        // Input should be an array with a single user message item
        let input = request.input.as_array().expect("input should be an array");
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "Hello");
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
            supports_parallel_tool_calls: true,
            context_window: 128000,
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

        let request = build_responses_request(&model, messages, false, &[], None).unwrap();

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
            supports_parallel_tool_calls: true,
            context_window: 128000,
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

        let request = build_responses_request(&model, messages, false, &[], None).unwrap();

        // Input should be an array with 3 items: user message, assistant message, tool result
        let input = request.input.as_array().expect("input should be an array");
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["text"], "Run command");
        assert_eq!(input[1]["type"], "message");
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"][0]["type"], "output_text");
        assert_eq!(
            input[1]["content"][0]["text"],
            "I'll help you run a command."
        );
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["output"], "Tool result: success");
    }

    #[test]
    fn test_responses_tool_message_images_embedded_inline() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: true,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: None,
            api_key: None,
            extra_body: None,
            thinking_level: tidev_types::reasoning::ThinkingLevelType::None,
        };

        // Build a tool result message that carries an image attachment.
        use tidev_types::message::ToolExecutionResult;
        let mut result = ToolExecutionResult::new("Image read successfully.");
        result.attachments.push(MessageAttachment::Image {
            filename: "icon.png".to_string(),
            mime: "image/png".to_string(),
            data: vec![0x89, 0x50, 0x4E, 0x47],
            file_size: 4,
        });
        let tool_msg = Message::tool_result("call_xyz", "read", result);

        let mut assistant = Message::new(MessageRole::Assistant, "");
        assistant.tool_calls.push(tidev_types::message::ToolCall {
            id: "call_xyz".to_string(),
            name: "read".to_string(),
            arguments: r#"{"file_path":"icon.png"}"#.to_string(),
            thought_signature: None,
        });

        let messages = vec![
            Message::new(MessageRole::User, "describe the icon"),
            assistant,
            tool_msg,
        ];

        let request = build_responses_request(&model, messages, false, &[], None).unwrap();
        let input = request.input.as_array().expect("input should be an array");

        // user message, function_call, function_call_output — no synthetic
        // user message and no assistant message (empty text).
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_xyz");
        assert_eq!(input[1]["name"], "read");
        assert_eq!(input[1]["arguments"], r#"{"file_path":"icon.png"}"#);
        assert_eq!(input[2]["type"], "function_call_output");

        let output = input[2]["output"]
            .as_array()
            .expect("output should be an array with text + image");
        assert_eq!(output.len(), 2);
        assert_eq!(output[0]["type"], "input_text");
        assert_eq!(output[1]["type"], "input_image");
        assert!(
            output[1]["image_url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }

    #[test]
    fn test_responses_assistant_with_text_and_tool_calls() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: None,
            api_key: None,
            extra_body: None,
            thinking_level: tidev_types::reasoning::ThinkingLevelType::None,
        };

        let mut assistant = Message::new(MessageRole::Assistant, "Let me read that file.");
        assistant.tool_calls.push(tidev_types::message::ToolCall {
            id: "call_abc".to_string(),
            name: "read".to_string(),
            arguments: r#"{"file_path":"/tmp/test.txt"}"#.to_string(),
            thought_signature: None,
        });

        let messages = vec![
            Message::new(MessageRole::User, "read /tmp/test.txt"),
            assistant,
            Message::tool_result(
                "call_abc",
                "read",
                tidev_types::message::ToolExecutionResult::new("file content"),
            ),
        ];

        let request = build_responses_request(&model, messages, true, &[], None).unwrap();
        let input = request.input.as_array().expect("input should be an array");

        // user message, assistant text message, function_call, function_call_output
        assert_eq!(input.len(), 4);
        // [0] user
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        // [1] assistant text
        assert_eq!(input[1]["type"], "message");
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"][0]["text"], "Let me read that file.");
        // [2] function_call
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[2]["call_id"], "call_abc");
        assert_eq!(input[2]["name"], "read");
        assert_eq!(input[2]["arguments"], r#"{"file_path":"/tmp/test.txt"}"#);
        // [3] function_call_output
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_abc");
    }

    #[test]
    fn test_responses_assistant_multiple_tool_calls() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: None,
            api_key: None,
            extra_body: None,
            thinking_level: tidev_types::reasoning::ThinkingLevelType::None,
        };

        let mut assistant = Message::new(MessageRole::Assistant, "");
        assistant.tool_calls.push(tidev_types::message::ToolCall {
            id: "call_1".to_string(),
            name: "read".to_string(),
            arguments: r#"{"path":"a.txt"}"#.to_string(),
            thought_signature: None,
        });
        assistant.tool_calls.push(tidev_types::message::ToolCall {
            id: "call_2".to_string(),
            name: "grep".to_string(),
            arguments: r#"{"pattern":"fn main"}"#.to_string(),
            thought_signature: None,
        });

        let messages = vec![
            Message::new(MessageRole::User, "find and read"),
            assistant,
            Message::tool_result(
                "call_1",
                "read",
                tidev_types::message::ToolExecutionResult::new("file content"),
            ),
            Message::tool_result(
                "call_2",
                "grep",
                tidev_types::message::ToolExecutionResult::new("found matches"),
            ),
        ];

        let request = build_responses_request(&model, messages, true, &[], None).unwrap();
        let input = request.input.as_array().expect("input should be an array");

        // user message, function_call x2, function_call_output x2 — no
        // assistant message (empty text).
        assert_eq!(input.len(), 5);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_1");
        assert_eq!(input[1]["name"], "read");
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[2]["call_id"], "call_2");
        assert_eq!(input[2]["name"], "grep");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_1");
        assert_eq!(input[4]["type"], "function_call_output");
        assert_eq!(input[4]["call_id"], "call_2");
    }

    #[test]
    fn test_responses_request_fields() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: Some("You are helpful.".to_string()),
            api_key: None,
            extra_body: None,
            thinking_level: tidev_types::reasoning::ThinkingLevelType::None,
        };

        let messages = vec![Message::new(MessageRole::User, "Hello")];

        // With tools — tool_choice should be "auto"
        let tools = vec![ToolDefinition {
            name: "read".to_string(),
            display_name: "Read".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }];
        let request =
            build_responses_request(&model, messages.clone(), true, &tools, None).unwrap();
        assert_eq!(request.parallel_tool_calls, false);
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["tool_choice"], "auto");
        assert_eq!(json["parallel_tool_calls"], false);

        // Without tools — tool_choice should be absent
        let request = build_responses_request(&model, messages, true, &[], None).unwrap();
        let json = serde_json::to_value(&request).unwrap();
        assert!(!json.as_object().unwrap().contains_key("tool_choice"));
        assert_eq!(json["parallel_tool_calls"], false);
    }

    #[test]
    fn test_response_tool_spec() {
        let tool = ToolDefinition {
            name: "shell".to_string(),
            display_name: "Shell".to_string(),
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
        assert_eq!(response_tool.name, "shell");
        assert!(!response_tool.description.is_empty());
    }

    #[test]
    fn responses_assistant_replays_raw_output_items() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-5".to_string(),
            request_model_id: Some("gpt-5".to_string()),
            max_output_tokens: 4096,
            temperature: None,
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: None,
            api_key: None,
            extra_body: None,
            thinking_level: tidev_types::reasoning::ThinkingLevelType::None,
        };
        let mut assistant = Message::new(MessageRole::Assistant, "visible text");
        assistant.metadata.responses_output_items = vec![
            serde_json::json!({
                "type": "reasoning",
                "id": "rs_1",
                "encrypted_content": "opaque"
            }),
            serde_json::json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "visible text"}]
            }),
        ];

        let request = build_responses_request(&model, vec![assistant], false, &[], None).unwrap();
        assert_eq!(
            request.input,
            serde_json::json!([
                {
                    "type": "reasoning",
                    "id": "rs_1",
                    "encrypted_content": "opaque"
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "visible text"}]
                }
            ])
        );
    }

    #[test]
    fn reasoning_summary_delta_reads_delta_field() {
        let event: ResponseStreamEvent = serde_json::from_value(serde_json::json!({
            "type": "response.reasoning_summary_text.delta",
            "delta": "summary chunk",
            "item_id": "rs_1",
            "output_index": 0,
            "summary_index": 0
        }))
        .unwrap();

        match event {
            ResponseStreamEvent::ReasoningSummaryTextDelta { summary_delta, .. } => {
                assert_eq!(summary_delta, "summary chunk");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn function_call_done_preserves_complete_arguments() {
        let event: ResponseStreamEvent = serde_json::from_value(serde_json::json!({
            "type": "response.function_call_arguments.done",
            "id": "call_1",
            "name": "read",
            "arguments": "{\"path\":\"a.txt\"}",
            "item_id": "fc_1"
        }))
        .unwrap();

        match event {
            ResponseStreamEvent::FunctionCallArgumentsDone { arguments, .. } => {
                assert_eq!(arguments, r#"{"path":"a.txt"}"#);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn output_item_serialization_preserves_encrypted_reasoning_content() {
        let event: ResponseStreamEvent = serde_json::from_value(serde_json::json!({
            "type": "response.output_item.done",
            "item": {
                "type": "reasoning",
                "id": "rs_1",
                "status": "completed",
                "encrypted_content": "opaque",
                "summary": []
            }
        }))
        .unwrap();

        let ResponseStreamEvent::OutputItemDone { item, .. } = event else {
            panic!("expected output item done");
        };
        let serialized = serde_json::to_value(item).unwrap();
        assert_eq!(serialized["type"], "reasoning");
        assert_eq!(serialized["encrypted_content"], "opaque");
        assert_eq!(serialized["summary"], serde_json::json!([]));
    }

    #[test]
    fn unknown_responses_event_is_ignored() {
        let event: ResponseStreamEvent = serde_json::from_value(serde_json::json!({
            "type": "response.future_event",
            "foo": "bar"
        }))
        .unwrap();
        assert!(matches!(event, ResponseStreamEvent::Unknown { .. }));
    }

    #[test]
    fn stream_error_classification_retries_transient_errors() {
        assert!(
            classify_responses_stream_error(
                "server overloaded".to_string(),
                Some("server_error".to_string())
            )
            .is_retryable()
        );
        assert!(
            !classify_responses_stream_error(
                "invalid prompt".to_string(),
                Some("invalid_request_error".to_string())
            )
            .is_retryable()
        );
    }

    #[test]
    fn sse_parser_combines_multiline_data_at_event_boundary() {
        let mut parser = SseParser::default();
        assert!(
            parser
                .push_line("event: response.output_text.delta")
                .is_none()
        );
        assert!(parser.push_line("data: {\"type\":").is_none());
        assert!(
            parser
                .push_line("data: \"response.output_text.delta\"}")
                .is_none()
        );
        assert_eq!(
            parser.push_line(""),
            Some("{\"type\":\n\"response.output_text.delta\"}".to_string())
        );
    }

    #[test]
    fn responses_extra_body_is_flattened_without_thinking_duplicates() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-5".to_string(),
            request_model_id: Some("gpt-5".to_string()),
            max_output_tokens: 4096,
            temperature: None,
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: None,
            api_key: None,
            extra_body: Some(serde_json::json!({"service_tier": "flex"})),
            thinking_level: tidev_types::reasoning::ThinkingLevelType::Gpt5(
                tidev_types::reasoning::Gpt5ThinkingLevel::High,
            ),
        };

        let request = build_responses_request(
            &model,
            vec![Message::new(MessageRole::User, "Hello")],
            true,
            &[],
            None,
        )
        .unwrap();
        let json = serde_json::to_value(request).unwrap();
        assert_eq!(json["service_tier"], "flex");
        assert_eq!(json["reasoning"]["effort"], "high");
        assert_eq!(json["reasoning"]["summary"], "auto");
        assert!(json.get("effort").is_none());
        assert!(json.get("summary").is_none());
    }
}
