use anyhow::{Context, Result};
use reqwest::Client;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::debug::{
    save_complete_response_for_debugging, save_raw_response_for_debugging,
    save_request_for_debugging,
};
use crate::error::{NetworkError, classify_response_status};
use crate::event::LlmEvent;
use crate::message::{Message, ToolCall};
use crate::think_parser::strip_think_tags;
use crate::{types::LlmProviderConfig, types::ToolDefinition};

use log::{debug as log_debug, error as log_error};

use self::error::{classify_responses_stream_error, response_error_details};
use self::event::ResponseStreamEvent;
use self::request::{ToolCallBuilder, build_responses_request};
use self::types::ResponsesCompleteResponse;

mod error;
mod event;
mod request;
mod types;

/// Responses API endpoint
const RESPONSES_ENDPOINT: &str = "/responses";
const RESPONSES_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream_responses(
    http: &Client,
    model: LlmProviderConfig,
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    tx: UnboundedSender<LlmEvent>,
    save_request_body: bool,
    max_request_files: usize,
    save_response_body: bool,
    max_response_files: usize,
) -> Result<()> {
    let api_key = model
        .api_key
        .clone()
        .with_context(|| format!("missing API key for provider '{}'", model.provider_id))?;

    let request = build_responses_request(&model, messages, true, &tools, None)?;
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
                let _ = tx.send(LlmEvent::Finished {
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
                    let _ = tx.send(LlmEvent::Delta { content: delta });
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
                    let _ = tx.send(LlmEvent::Delta { content: delta });
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
                        let _ = tx.send(LlmEvent::ReasoningDelta { content: cleaned });
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
                        let _ = tx.send(LlmEvent::ReasoningDelta { content: cleaned });
                    }
                }
                ResponseStreamEvent::ReasoningSummaryTextDelta {
                    summary_delta,
                    sequence_number: _,
                    item_id: _,
                    output_index: _,
                    summary_index,
                } => {
                    let cleaned = strip_think_tags(&summary_delta);
                    if !cleaned.is_empty() {
                        reasoning_text.push_str(&cleaned);
                        let _ = tx.send(LlmEvent::ReasoningSummaryDelta {
                            content: cleaned,
                            summary_index: Some(summary_index),
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
                            let call = crate::message::ToolCall {
                                id: builder.id().to_string(),
                                name: builder.name().to_string(),
                                arguments: arguments.to_string(),
                                thought_signature: None,
                            };
                            let _ = tx.send(LlmEvent::ToolCallUpdated { tool_call: call });
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
                        let _ = tx.send(LlmEvent::UsageStats {
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
                    let _ = tx.send(LlmEvent::Finished {
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

    save_raw_response_for_debugging(&raw_payloads, save_response_body, max_response_files);

    Err(NetworkError::Retryable {
        message: "Responses stream closed before response.completed".to_string(),
    }
    .into())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn complete_responses(
    http: &Client,
    model: LlmProviderConfig,
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    tx: Option<&UnboundedSender<LlmEvent>>,
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
        let _ = tx.send(LlmEvent::UsageStats {
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

fn finalize_turn(
    assistant_text: &str,
    reasoning_text: &str,
    finish_reason: &Option<String>,
    tool_calls: &BTreeMap<String, ToolCallBuilder>,
    responses_output_items: &[serde_json::Value],
) -> crate::message::AssistantTurn {
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

    crate::message::AssistantTurn {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
