//! Google Gemini API provider implementation.
//!
//! Implements both streaming (`streamGenerateContent` with SSE) and
//! non-streaming (`generateContent`) interactions with the Gemini API.
//!
//! # API Reference
//!
//! - Endpoints: `{base}/models/{model}:generateContent` and `:streamGenerateContent?alt=sse`
//! - Auth: `x-goog-api-key` header
//! - Streaming: Server-Sent Events (SSE) with full JSON per `data:` line
//!
//! # Key Differences from Other Providers
//!
//! - Uses `"model"` role instead of `"assistant"`
//! - System instruction is a top-level `system_instruction` field, not a message
//! - Tool calls (`functionCall`) arrive as complete objects, not incremental deltas
//! - Each SSE chunk contains a complete `GenerateContentResponse` JSON
//! - Usage metadata (`usageMetadata`) appears in every chunk
//! - Parts carry an optional `thought` flag to distinguish reasoning from visible text
//! - Parts may carry a `thoughtSignature` that must be echoed back (Gemini 3+)

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
use crate::error::classify_response_status;
use crate::think_parser::ThinkParser;
use crate::tool_call_format::ToolCallBuilder;
use crate::turn::finalize_turn;

// ============================================================================
// Public API
// ============================================================================

/// Stream a chat completion via Gemini's `streamGenerateContent` (SSE).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream_gemini(
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

    let endpoint = model.gemini_stream_endpoint();
    let request = build_gemini_request(&model, messages, &tools)?;
    let request_body = serde_json::to_string(&request).unwrap_or_default();
    let request_body_size = request_body.len();
    save_request_for_debugging(&request_body, save_request_body, max_request_files);

    let send_result = http
        .post(&endpoint)
        .header("x-goog-api-key", &api_key)
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
                    "gemini request failed: method=POST url={} request_body_size={} status={} error_body={}",
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
                "gemini request failed: method=POST url={} request_body_size={} error={}",
                endpoint,
                request_body_size,
                e
            );
            return Err(e.into());
        }
    };

    log_debug!(
        "gemini request: method=POST url={} request_body_size={} status={}",
        endpoint,
        request_body_size,
        response.status()
    );

    // ── SSE stream parsing ────────────────────────────────────────────────
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();
    let mut assistant_text = String::new();
    let mut reasoning_text = String::new();
    let mut finish_reason: Option<String> = None;
    let mut tool_calls: BTreeMap<usize, ToolCallBuilder> = BTreeMap::new();
    let mut think_parser = ThinkParser::default();
    let mut first_delta_time: Option<std::time::Instant> = None;
    // Track the last usage metadata received (Gemini sends it in every chunk)
    let mut last_usage: Option<GeminiUsage> = None;
    let mut raw_payloads: Vec<String> = Vec::new();

    use futures_util::StreamExt;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buffer.extend_from_slice(&chunk);

        // Process complete SSE lines
        while let Some(line_end) = buffer.iter().position(|&b| b == b'\n') {
            let tail = &buffer[..line_end];
            let line = String::from_utf8_lossy(
                if tail.last() == Some(&b'\r') { &tail[..tail.len() - 1] } else { tail }
            ).into_owned().trim().to_string();
            buffer.drain(..=line_end);

            if line.is_empty() {
                continue;
            }

            // SSE data lines start with "data: "
            let payload = if let Some(p) = line.strip_prefix("data: ") {
                p.to_string()
            } else {
                continue;
            };

            raw_payloads.push(payload.clone());

            if payload.trim().is_empty() {
                continue;
            }

            // Parse the chunk JSON
            let chunk_response: GeminiChunkResponse = match serde_json::from_str(&payload) {
                Ok(c) => c,
                Err(e) => {
                    log_debug!("gemini: failed to parse SSE chunk: {e}, payload={payload}");
                    continue;
                }
            };

            // Process candidates
            if let Some(candidates) = chunk_response.candidates {
                for candidate in candidates {
                    // Track finish reason
                    if let Some(reason) = &candidate.finish_reason
                        && reason != "FINISH_REASON_UNSPECIFIED"
                        && reason != "finish_reason_unspecified"
                    {
                        finish_reason = Some(reason.clone());
                    }

                    let Some(content) = candidate.content else {
                        continue;
                    };

                    let mut tool_call_index = tool_calls.len();

                    for part in content.parts {
                        // ── Text part ──────────────────────────────────────
                        if let Some(text) = part.text {
                            if first_delta_time.is_none() {
                                first_delta_time = Some(std::time::Instant::now());
                            }

                            let was_thinking = part.thought.unwrap_or(false);

                            if was_thinking {
                                // Native Gemini thought/reasoning
                                let _ = tx.send(BackendEvent::ReasoningDelta {
                                    session_id,
                                    request_id,
                                    content: text.clone(),
                                });
                                reasoning_text.push_str(&text);
                            } else {
                                // Regular text — run through ThinkParser for <think> tags too
                                let (visible, reasoning) = think_parser.push(&text);
                                if !visible.is_empty() {
                                    let _ = tx.send(BackendEvent::Delta {
                                        session_id,
                                        request_id,
                                        content: visible.clone(),
                                    });
                                    assistant_text.push_str(&visible);
                                }
                                if !reasoning.is_empty() {
                                    let _ = tx.send(BackendEvent::ReasoningDelta {
                                        session_id,
                                        request_id,
                                        content: reasoning.clone(),
                                    });
                                    reasoning_text.push_str(&reasoning);
                                }
                            }
                        }

                        // ── Function call part ─────────────────────────────
                        if let Some(fcall) = part.function_call {
                            let tc_id = fcall
                                .id
                                .clone()
                                .unwrap_or_else(|| format!("gc-{}-{}", fcall.name, request_id));

                            let tc = ToolCall {
                                id: tc_id.clone(),
                                name: fcall.name.clone(),
                                arguments: serde_json::to_string(&fcall.args)
                                    .unwrap_or_else(|_| "{}".to_string()),
                                thought_signature: part.thought_signature.clone(),
                            };

                            let _ = tx.send(BackendEvent::ToolCallUpdated {
                                session_id,
                                request_id,
                                tool_call: tc,
                            });

                            tool_calls.insert(
                                tool_call_index,
                                ToolCallBuilder {
                                    id: tc_id,
                                    name: fcall.name,
                                    arguments: serde_json::to_string(&fcall.args)
                                        .unwrap_or_else(|_| "{}".to_string()),
                                },
                            );
                            tool_call_index += 1;
                        }

                        // ── ExecutableCode / CodeExecutionResult ───────────
                        // These are server-side code execution built-in tools.
                        // For now, we just include the text output as content.
                        if let Some(code_result) = part.code_execution_result
                            && let Some(output) = code_result.output
                        {
                            let (visible, reasoning) = think_parser.push(&output);
                            if !visible.is_empty() {
                                let _ = tx.send(BackendEvent::Delta {
                                    session_id,
                                    request_id,
                                    content: visible.clone(),
                                });
                                assistant_text.push_str(&visible);
                            }
                            if !reasoning.is_empty() {
                                let _ = tx.send(BackendEvent::ReasoningDelta {
                                    session_id,
                                    request_id,
                                    content: reasoning.clone(),
                                });
                                reasoning_text.push_str(&reasoning);
                            }
                        }
                    }

                    // ── Citations ─────────────────────────────────────────
                    // Optionally handle citation metadata
                }
            }

            // Track usage metadata (Gemini sends it in every chunk; keep the last one)
            if let Some(usage) = chunk_response.usage_metadata {
                last_usage = Some(usage);
            }
        }
    }

    // Drain any remaining think_parser buffer
    let (remaining_visible, remaining_reasoning) = think_parser.finish();
    if !remaining_visible.is_empty() {
        let _ = tx.send(BackendEvent::Delta {
            session_id,
            request_id,
            content: remaining_visible.clone(),
        });
        assistant_text.push_str(&remaining_visible);
    }
    if !remaining_reasoning.is_empty() {
        let _ = tx.send(BackendEvent::ReasoningDelta {
            session_id,
            request_id,
            content: remaining_reasoning.clone(),
        });
        reasoning_text.push_str(&remaining_reasoning);
    }

    // ── Save raw response payloads ────────────────────────────────────────
    save_raw_response_for_debugging(
        session_id,
        request_id,
        &raw_payloads,
        save_response_body,
        max_response_files,
    );

    // ── Send usage stats ──────────────────────────────────────────────────
    if let Some(usage) = last_usage {
        let _ = tx.send(BackendEvent::UsageStats {
            session_id,
            request_id,
            input_tokens: usage.prompt_token_count.unwrap_or(0),
            output_tokens: usage.candidates_token_count.unwrap_or(0),
            total_tokens: usage.total_token_count.unwrap_or(0),
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            model_id: model.model_id.clone(),
            duration_ms: first_delta_time.map(|t| t.elapsed().as_millis() as u64),
        });
    }

    // ── Finalize turn ─────────────────────────────────────────────────────
    let turn = finalize_turn(
        assistant_text,
        reasoning_text,
        finish_reason,
        &tool_calls,
        &mut think_parser,
    );

    let _ = tx.send(BackendEvent::Finished {
        session_id,
        request_id,
        turn,
    });

    Ok(())
}

/// Non-streaming completion via Gemini's `generateContent`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn complete_gemini(
    http: &Client,
    model: LlmProviderConfig,
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    save_request_body: bool,
    max_request_files: usize,
    save_response_body: bool,
    max_response_files: usize,
) -> Result<String> {
    let api_key = model
        .api_key
        .clone()
        .with_context(|| format!("missing API key for provider '{}'", model.provider_id))?;

    let endpoint = model.endpoint(); // uses :generateContent for non-streaming
    let request = build_gemini_request(&model, messages, &tools)?;
    let request_body = serde_json::to_string(&request).unwrap_or_default();
    let request_body_size = request_body.len();
    save_request_for_debugging(&request_body, save_request_body, max_request_files);

    let send_result = http
        .post(&endpoint)
        .header("x-goog-api-key", &api_key)
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
                    "gemini (complete) request failed: method=POST url={} request_body_size={} status={} error_body={}",
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
                "gemini (complete) request failed: method=POST url={} request_body_size={} error={}",
                endpoint,
                request_body_size,
                e
            );
            return Err(e.into());
        }
    };

    log_debug!(
        "gemini (complete) request: method=POST url={} request_body_size={} status={}",
        endpoint,
        request_body_size,
        response.status()
    );

    let body_text = response.text().await?;
    save_complete_response_for_debugging(&body_text, save_response_body, max_response_files);

    let gemini_response: GeminiResponse = serde_json::from_str(&body_text)?;

    // Extract text from the first candidate's parts
    let text = gemini_response
        .candidates
        .and_then(|candidates| candidates.into_iter().next())
        .and_then(|c| c.content)
        .map(|content| {
            content
                .parts
                .into_iter()
                .filter_map(|part| part.text)
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    Ok(text)
}

// ============================================================================
// Request Building
// ============================================================================

fn build_gemini_request(
    model: &LlmProviderConfig,
    messages: Vec<Message>,
    tools: &[ToolDefinition],
) -> Result<GeminiRequest> {
    // ── System instruction ────────────────────────────────────────────────
    let system_instruction = if model.system_prompt_str().trim().is_empty() {
        None
    } else {
        Some(GeminiContent {
            role: None,
            parts: vec![GeminiPart {
                text: Some(model.system_prompt.clone().unwrap_or_default()),
                inline_data: None,
                file_data: None,
                function_call: None,
                function_response: None,
                executable_code: None,
                code_execution_result: None,
                thought: None,
                thought_signature: None,
            }],
        })
    };

    // ── Convert messages ──────────────────────────────────────────────────
    let mut contents: Vec<GeminiContent> = Vec::new();

    for message in messages {
        if message.streaming {
            continue;
        }

        match message.role {
            MessageRole::System => {
                // System messages are handled via system_instruction above;
                // skip any message-level system role entries.
            }
            MessageRole::User => {
                let parts = user_message_parts(model, &message)?;
                if !parts.is_empty() {
                    contents.push(GeminiContent {
                        role: Some("user".to_string()),
                        parts,
                    });
                }
            }
            MessageRole::Assistant => {
                let mut parts: Vec<GeminiPart> = Vec::new();

                // Text content
                let text = message_text_with_file_references(&message);
                if !text.is_empty() {
                    parts.push(GeminiPart {
                        text: Some(text),
                        inline_data: None,
                        file_data: None,
                        function_call: None,
                        function_response: None,
                        executable_code: None,
                        code_execution_result: None,
                        thought: None,
                        thought_signature: None,
                    });
                }

                // Reasoning content — send back as a thought part if present
                if !message.reasoning.is_empty() {
                    parts.push(GeminiPart {
                        text: Some(message.reasoning.clone()),
                        inline_data: None,
                        file_data: None,
                        function_call: None,
                        function_response: None,
                        executable_code: None,
                        code_execution_result: None,
                        thought: Some(true),
                        thought_signature: None,
                    });
                }

                // Tool calls (functionCall parts)
                for tc in &message.tool_calls {
                    let args: serde_json::Value = serde_json::from_str(&tc.arguments)
                        .unwrap_or(serde_json::Value::Object(Default::default()));

                    parts.push(GeminiPart {
                        text: None,
                        inline_data: None,
                        file_data: None,
                        function_call: Some(GeminiFunctionCall {
                            name: tc.name.clone(),
                            args,
                            id: Some(tc.id.clone()),
                        }),
                        function_response: None,
                        executable_code: None,
                        code_execution_result: None,
                        thought: None,
                        thought_signature: tc.thought_signature.clone(),
                    });
                }

                if !parts.is_empty() {
                    contents.push(GeminiContent {
                        role: Some("model".to_string()),
                        parts,
                    });
                }
            }
            MessageRole::Tool => {
                // Tool results are sent as functionResponse parts in a user-role message.
                // Multiple consecutive tool results should be merged into a single
                // user-role content entry. If images are attached, they are sent as
                // additional inline_data parts alongside the function_response part.
                let tool_call_id = message.tool_call_id.clone().unwrap_or_default();
                let tool_name = message.tool_name.clone().unwrap_or_default();
                let content_text = message_text_with_file_references(&message);

                // Parse the response as JSON if possible
                let response_value: serde_json::Value = serde_json::from_str(&content_text)
                    .unwrap_or(serde_json::Value::String(content_text));

                let mut parts: Vec<GeminiPart> = Vec::new();

                // Function response part (always present)
                parts.push(GeminiPart {
                    text: None,
                    inline_data: None,
                    file_data: None,
                    function_call: None,
                    function_response: Some(GeminiFunctionResponse {
                        name: tool_name,
                        response: response_value,
                        id: if tool_call_id.is_empty() {
                            None
                        } else {
                            Some(tool_call_id)
                        },
                    }),
                    executable_code: None,
                    code_execution_result: None,
                    thought: None,
                    thought_signature: None,
                });

                // Image parts (same logic as user_message_parts)
                for attachment in image_attachments(&message) {
                    if !model.supports_images {
                        anyhow::bail!("current model does not support image attachments");
                    }

                    if let MessageAttachment::Image { mime, data, .. } = attachment {
                        parts.push(GeminiPart {
                            text: None,
                            inline_data: Some(GeminiBlob {
                                mime_type: mime.clone(),
                                data: BASE64.encode(data),
                            }),
                            file_data: None,
                            function_call: None,
                            function_response: None,
                            executable_code: None,
                            code_execution_result: None,
                            thought: None,
                            thought_signature: None,
                        });
                    }
                }

                // Merge with the last user message if present, so all tool results
                // for the same assistant turn live in a single user message.
                if let Some(last) = contents.last_mut()
                    && last.role.as_deref() == Some("user")
                {
                    last.parts.extend(parts);
                } else {
                    contents.push(GeminiContent {
                        role: Some("user".to_string()),
                        parts,
                    });
                }
            }
            MessageRole::Error | MessageRole::Shell => {
                // Skip error and shell messages
            }
        }
    }

    // ── Tools ─────────────────────────────────────────────────────────────
    let gemini_tools = if tools.is_empty() {
        None
    } else {
        Some(vec![GeminiTool {
            function_declarations: tools
                .iter()
                .map(|t| GeminiFunctionDeclaration {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                })
                .collect(),
        }])
    };

    // ── Generation config ─────────────────────────────────────────────────
    let mut generation_config = serde_json::json!({
        "maxOutputTokens": model.max_output_tokens,
    });
    if let Some(temp) = model.temperature {
        generation_config["temperature"] = serde_json::json!(temp);
    }

    // Merge extra_body (including thinking config) into generation config
    if let Some(extra) = model.merged_extra_body()
        && let Some(obj) = extra.as_object()
    {
        for (k, v) in obj {
            generation_config[k] = v.clone();
        }
    }

    Ok(GeminiRequest {
        contents,
        system_instruction,
        tools: gemini_tools,
        generation_config: Some(generation_config),
    })
}

/// Build the parts array for a user message (text + optional images).
fn user_message_parts(model: &LlmProviderConfig, message: &Message) -> Result<Vec<GeminiPart>> {
    let text = message_text_with_file_references(message);
    let images: Vec<&tidev_types::message::MessageAttachment> =
        image_attachments(message).collect();

    let mut parts = Vec::new();

    if !text.trim().is_empty() {
        parts.push(GeminiPart {
            text: Some(text),
            inline_data: None,
            file_data: None,
            function_call: None,
            function_response: None,
            executable_code: None,
            code_execution_result: None,
            thought: None,
            thought_signature: None,
        });
    }

    for attachment in &images {
        if !model.supports_images {
            anyhow::bail!("current model does not support image attachments");
        }

        if let MessageAttachment::Image { mime, data, .. } = attachment {
            parts.push(GeminiPart {
                text: None,
                inline_data: Some(GeminiBlob {
                    mime_type: mime.clone(),
                    data: BASE64.encode(data),
                }),
                file_data: None,
                function_call: None,
                function_response: None,
                executable_code: None,
                code_execution_result: None,
                thought: None,
                thought_signature: None,
            });
        }
    }

    Ok(parts)
}

// ============================================================================
// Data Structures
// ============================================================================

/// Top-level request body for `generateContent` / `streamGenerateContent`.
#[derive(Clone, Debug, Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<serde_json::Value>,
}

/// A single turn in the conversation.
#[derive(Clone, Debug, Serialize)]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GeminiPart>,
}

/// A part within a `Content` — can be text, inline_data, file_data,
/// functionCall, or functionResponse.
#[derive(Clone, Debug, Serialize, Default)]
struct GeminiPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inline_data: Option<GeminiBlob>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_data: Option<GeminiFileData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_call: Option<GeminiFunctionCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_response: Option<GeminiFunctionResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    executable_code: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code_execution_result: Option<GeminiCodeExecutionResult>,
    /// If true, this part represents the model's thought process / reasoning.
    #[serde(skip_serializing_if = "Option::is_none")]
    thought: Option<bool>,
    /// Opaque signature for thought, must be echoed back if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    thought_signature: Option<String>,
}

/// Base64-encoded inline data (images, audio, video).
#[derive(Clone, Debug, Serialize)]
struct GeminiBlob {
    mime_type: String,
    data: String,
}

/// Reference to a file stored in Google Cloud Storage or elsewhere.
#[derive(Clone, Debug, Serialize)]
struct GeminiFileData {
    mime_type: String,
    file_uri: String,
}

/// A function call from the model.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct GeminiFunctionCall {
    name: String,
    args: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
}

/// The result of a function call, sent back to the model.
#[derive(Clone, Debug, Serialize)]
struct GeminiFunctionResponse {
    name: String,
    response: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
}

/// Result of server-side code execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct GeminiCodeExecutionResult {
    #[allow(dead_code)]
    outcome: Option<String>,
    output: Option<String>,
}

/// A tool definition (contains function declarations).
#[derive(Clone, Debug, Serialize)]
struct GeminiTool {
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Clone, Debug, Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

// ── Response types ─────────────────────────────────────────────────────────

/// Non-streaming response from `generateContent`.
#[derive(Clone, Debug, Deserialize)]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
    #[allow(dead_code)]
    usage_metadata: Option<GeminiUsage>,
    #[allow(dead_code)]
    model_version: Option<String>,
}

/// A single chunk in the SSE stream.
#[derive(Clone, Debug, Deserialize)]
struct GeminiChunkResponse {
    candidates: Option<Vec<GeminiCandidate>>,
    #[allow(dead_code)]
    usage_metadata: Option<GeminiUsage>,
    #[allow(dead_code)]
    model_version: Option<String>,
    #[allow(dead_code)]
    response_id: Option<String>,
}

/// A candidate response from the model.
#[derive(Clone, Debug, Deserialize)]
struct GeminiCandidate {
    content: Option<GeminiResponseContent>,
    #[allow(dead_code)]
    finish_reason: Option<String>,
    #[allow(dead_code)]
    index: Option<u32>,
    #[allow(dead_code)]
    safety_ratings: Option<Vec<serde_json::Value>>,
}

/// Content within a response candidate.
#[derive(Clone, Debug, Deserialize)]
struct GeminiResponseContent {
    parts: Vec<GeminiResponsePart>,
    #[allow(dead_code)]
    role: Option<String>,
}

/// A part within a response.
#[derive(Clone, Debug, Deserialize)]
struct GeminiResponsePart {
    text: Option<String>,
    #[allow(dead_code)]
    inline_data: Option<GeminiBlobResponse>,
    #[allow(dead_code)]
    file_data: Option<serde_json::Value>,
    function_call: Option<GeminiFunctionCall>,
    #[allow(dead_code)]
    function_response: Option<serde_json::Value>,
    #[allow(dead_code)]
    executable_code: Option<serde_json::Value>,
    code_execution_result: Option<GeminiCodeExecutionResult>,
    /// If true, this part is a thought/reasoning part.
    thought: Option<bool>,
    /// Opaque signature that must be echoed back.
    thought_signature: Option<String>,
}

/// Usage metadata (camelCase from the API).
#[derive(Clone, Debug, Deserialize)]
struct GeminiUsage {
    #[serde(alias = "promptTokenCount")]
    prompt_token_count: Option<u32>,
    #[serde(alias = "candidatesTokenCount")]
    candidates_token_count: Option<u32>,
    #[serde(alias = "totalTokenCount")]
    total_token_count: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
struct GeminiBlobResponse {
    #[allow(dead_code)]
    mime_type: Option<String>,
    #[allow(dead_code)]
    data: Option<String>,
}

// ============================================================================
// ToolCall builder — uses the shared ToolCallBuilder from tool_call_format
// ============================================================================
// The shared ToolCallBuilder (super::tool_call_format::ToolCallBuilder) is
// used for the `finalize_turn()` call. Gemini's tool calls arrive as complete
// objects rather than incremental deltas, so the builder is used only to
// match the finalize_turn() interface.
