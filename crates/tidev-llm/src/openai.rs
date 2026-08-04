use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::mpsc::UnboundedSender;

use crate::event::LlmEvent;
use crate::message::{Message, MessageAttachment, MessageRole, ToolCall};
use crate::reasoning::ThinkingLevelType;
use crate::{types::LlmProviderConfig, types::ToolDefinition};

use log::{debug as log_debug, error as log_error};

use crate::attachments::{image_attachments, message_text_with_file_references};
use crate::debug::{
    save_complete_response_for_debugging, save_raw_response_for_debugging,
    save_request_for_debugging,
};
use crate::error::classify_response_status;
use crate::think_parser::ThinkParser;
use crate::think_parser::strip_think_tags;
use crate::tool_call_format::ToolCallBuilder;
use crate::turn::finalize_turn;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream_openai(
    http: &Client,
    model: LlmProviderConfig,
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    tx: UnboundedSender<LlmEvent>,
    thinking_level: ThinkingLevelType,
    save_request_body: bool,
    max_request_files: usize,
    save_response_body: bool,
    max_response_files: usize,
) -> Result<()> {
    let api_key = model
        .api_key
        .clone()
        .with_context(|| format!("missing API key for provider '{}'", model.provider_id))?;
    let request = build_openai_request(&model, messages, true, &tools, thinking_level)?;
    let request_body = serde_json::to_string(&request).unwrap_or_default();
    let request_body_size = request_body.len();
    save_request_for_debugging(&request_body, save_request_body, max_request_files);

    let send_result = http
        .post(model.endpoint())
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
                    "openai request failed: method=POST url={} request_body_size={} status={} error_body={}",
                    model.endpoint(),
                    request_body_size,
                    status,
                    error_body
                );
                return Err(classify_response_status(status, Some(error_body)).into());
            }
        }
        Err(e) => {
            log_error!(
                "openai request failed: method=POST url={} request_body_size={} error={}",
                model.endpoint(),
                request_body_size,
                e
            );
            return Err(e.into());
        }
    };

    log_debug!(
        "openai request: method=POST url={} request_body_size={} status={}",
        model.endpoint(),
        request_body_size,
        response.status()
    );

    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();
    let mut assistant_text = String::new();
    let mut reasoning_text = String::new();
    let mut finish_reason: Option<String> = None;
    let mut tool_calls: BTreeMap<usize, ToolCallBuilder> = BTreeMap::new();
    let mut think_parser = ThinkParser::default();
    let mut first_delta_time: Option<std::time::Instant> = None;
    let mut raw_payloads: Vec<String> = Vec::new();
    // When the API already provides reasoning through separate fields
    // (reasoning_content / reasoning_details), skip ThinkParser on the
    // text content — it is guaranteed to be visible text only.
    let mut has_reasoning_channel = false;

    use futures_util::StreamExt;

    while let Some(chunk) = stream.next().await {
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

            if line.is_empty() {
                continue;
            }

            if let Some(payload) = line.strip_prefix("data:") {
                let payload = payload.trim();

                raw_payloads.push(payload.to_string());
                if payload == "[DONE]" {
                    save_raw_response_for_debugging(
                        &raw_payloads,
                        save_response_body,
                        max_response_files,
                    );
                    let turn = finalize_turn(
                        assistant_text.clone(),
                        reasoning_text.clone(),
                        finish_reason.clone(),
                        &tool_calls,
                        &mut think_parser,
                    );
                    let _ = tx.send(LlmEvent::Finished {
                        turn: Box::new(turn),
                    });
                    return Ok(());
                }

                let event: ChatCompletionStreamResponse =
                    serde_json::from_str(payload).context("failed to parse streaming response")?;

                if let Some(usage) = event.usage {
                    let cache_read_tokens = usage
                        .prompt_tokens_details
                        .as_ref()
                        .map(|d| d.cached_tokens)
                        .unwrap_or(0);
                    let duration_ms =
                        first_delta_time.map(|start| start.elapsed().as_millis() as u64);
                    let _ = tx.send(LlmEvent::UsageStats {
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        total_tokens: usage.total_tokens,
                        cache_read_tokens,
                        cache_write_tokens: 0,
                        model_id: format!("{}:{}", model.provider_id, model.model_id),
                        duration_ms,
                    });
                }

                for choice in event.choices {
                    if let Some(reasoning) = choice.delta.reasoning_content {
                        has_reasoning_channel = true;
                        if first_delta_time.is_none() {
                            first_delta_time = Some(std::time::Instant::now());
                        }
                        let cleaned = strip_think_tags(&reasoning);
                        if !cleaned.is_empty() {
                            reasoning_text.push_str(&cleaned);
                            let _ = tx.send(LlmEvent::ReasoningDelta { content: cleaned });
                        }
                    }

                    // Handle reasoning_details: structured multi-section reasoning
                    // from newer OpenAI Chat Completions API (e.g. gpt-5.6-luna).
                    if let Some(details) = &choice.delta.reasoning_details {
                        has_reasoning_channel = true;
                        if first_delta_time.is_none() {
                            first_delta_time = Some(std::time::Instant::now());
                        }
                        for detail in details {
                            // Pick the content field based on the type indicator.
                            let text = match detail.detail_type.as_str() {
                                "reasoning.summary" => detail.summary.as_deref(),
                                "reasoning.text" => detail.text.as_deref(),
                                _ => None,
                            };
                            if let Some(text) = text
                                && !text.is_empty()
                            {
                                let cleaned = strip_think_tags(text);
                                if !cleaned.is_empty() {
                                    reasoning_text.push_str(&cleaned);
                                    let _ = tx.send(LlmEvent::ReasoningDelta { content: cleaned });
                                }
                            }
                        }
                    }

                    if let Some(content) = choice.delta.content {
                        if first_delta_time.is_none() {
                            first_delta_time = Some(std::time::Instant::now());
                        }

                        if has_reasoning_channel {
                            // Reasoning already provided through separate fields
                            // (reasoning_content / reasoning_details).  Send
                            // content directly as visible text — the ThinkParser
                            // must not see it, otherwise literal <think> or
                            // <thinking> text in the visible output would be
                            // misclassified as reasoning.
                            assistant_text.push_str(&content);
                            let _ = tx.send(LlmEvent::Delta { content });
                        } else {
                            // No separate reasoning channel — use ThinkParser to
                            // extract any <think> / <thinking> tags from the text.
                            let (visible, reasoning) = think_parser.push(&content);

                            if !visible.is_empty() {
                                assistant_text.push_str(&visible);
                                let _ = tx.send(LlmEvent::Delta { content: visible });
                            }

                            if !reasoning.is_empty() {
                                reasoning_text.push_str(&reasoning);
                                let _ = tx.send(LlmEvent::ReasoningDelta { content: reasoning });
                            }
                        }
                    }

                    if let Some(ref tool_calls_delta) = choice.delta.tool_calls {
                        if first_delta_time.is_none() {
                            first_delta_time = Some(std::time::Instant::now());
                        }
                        for tool_call in tool_calls_delta {
                            let index = tool_call.index.unwrap_or(tool_calls.len());
                            let entry = tool_calls.entry(index).or_default();

                            if let Some(id) = &tool_call.id {
                                entry.id = id.clone();
                            }

                            if let Some(function) = &tool_call.function {
                                if let Some(name) = &function.name
                                    && !name.is_empty()
                                {
                                    entry.name = name.clone();
                                }

                                if let Some(arguments) = &function.arguments {
                                    entry.arguments.push_str(arguments);
                                }
                            }

                            if !entry.id.is_empty() && !entry.name.is_empty() {
                                let _ = tx.send(LlmEvent::ToolCallUpdated {
                                    tool_call: entry.clone().into_tool_call(index),
                                });
                            }
                        }
                    }

                    if let Some(reason) = choice.finish_reason {
                        finish_reason = Some(reason);
                    }
                }
            }
        }
    }

    save_raw_response_for_debugging(&raw_payloads, save_response_body, max_response_files);

    let turn = finalize_turn(
        assistant_text.clone(),
        reasoning_text.clone(),
        finish_reason.clone(),
        &tool_calls,
        &mut think_parser,
    );
    let _ = tx.send(LlmEvent::Finished {
        turn: Box::new(turn),
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn complete_openai(
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
    let request = build_openai_request(
        &model,
        messages,
        false,
        &tools,
        model.thinking_level.clone(),
    )?;
    let request_body = serde_json::to_string(&request).unwrap_or_default();
    let request_body_size = request_body.len();
    save_request_for_debugging(&request_body, save_request_body, max_request_files);

    let send_result = http
        .post(model.endpoint())
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
                    "openai request (complete) failed: method=POST url={} request_body_size={} status={} error_body={}",
                    model.endpoint(),
                    request_body_size,
                    status,
                    error_body
                );
                return Err(classify_response_status(status, Some(error_body)).into());
            }
        }
        Err(e) => {
            log_error!(
                "openai request (complete) failed: method=POST url={} request_body_size={} error={}",
                model.endpoint(),
                request_body_size,
                e
            );
            return Err(e.into());
        }
    };

    log_debug!(
        "openai request (complete): method=POST url={} request_body_size={} status={}",
        model.endpoint(),
        request_body_size,
        response.status()
    );

    let body_text = response.text().await?;
    save_complete_response_for_debugging(&body_text, save_response_body, max_response_files);

    let response: ChatCompletionResponse = serde_json::from_str(&body_text)?;
    let content = response
        .choices
        .into_iter()
        .find_map(|choice| choice.message.content)
        .unwrap_or_default();

    Ok(content)
}

fn build_openai_request(
    model: &LlmProviderConfig,
    messages: Vec<Message>,
    stream: bool,
    tools: &[ToolDefinition],
    thinking_level: ThinkingLevelType,
) -> Result<ChatCompletionRequest> {
    let mut request_messages = Vec::new();

    // System prompt comes from the model config directly.
    // No context summary merging needed — compaction summaries are now
    // User messages inserted at the compression boundary, not System messages.
    if !model.system_prompt_str().trim().is_empty() {
        request_messages.push(ChatMessagePayload::system(
            model.system_prompt.clone().unwrap_or_default(),
        ));
    }

    // Process only User/Assistant/Tool messages (System messages already handled above)
    for message in &messages {
        if message.streaming {
            continue;
        }

        match message.role {
            MessageRole::System => {}
            MessageRole::User => request_messages.push(ChatMessagePayload::user(model, message)?),
            MessageRole::Assistant => {
                let tool_calls = if message.tool_calls.is_empty() {
                    None
                } else {
                    Some(
                        message
                            .tool_calls
                            .iter()
                            .map(ChatToolCallPayload::from)
                            .collect(),
                    )
                };

                request_messages.push(ChatMessagePayload::assistant(
                    message_text_with_file_references(message),
                    tool_calls,
                    Some(message.reasoning.clone()),
                ))
            }
            MessageRole::Tool => {
                let text = message_text_with_file_references(message);
                let images: Vec<&MessageAttachment> = image_attachments(message).collect();

                // Tool messages must use plain string content per the
                // Chat Completions API spec — `image_url` parts are only
                // allowed in user messages.
                request_messages.push(ChatMessagePayload::tool(
                    text,
                    message.tool_call_id.clone(),
                    message.tool_name.clone(),
                ));

                // If the tool returned image attachments and the model
                // supports vision, inject a synthetic user message so the
                // model can actually see the images in context.
                if !images.is_empty() && model.supports_images {
                    let mut parts: Vec<serde_json::Value> = Vec::new();
                    for attachment in &images {
                        if let MessageAttachment::Image { mime, data, .. } = attachment {
                            let b64 = BASE64.encode(data);
                            parts.push(serde_json::json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!(
                                        "data:{};base64,{}",
                                        mime, b64,
                                    ),
                                },
                            }));
                        }
                    }
                    if !parts.is_empty() {
                        request_messages.push(ChatMessagePayload::synthetic_user(parts));
                    }
                }
            }
            MessageRole::Error => {}
            MessageRole::Shell => {}
        }
    }

    let chat_tools = if tools.is_empty() {
        None
    } else {
        Some(tools.iter().map(ChatToolSpec::from).collect())
    };

    Ok(ChatCompletionRequest {
        model: model.request_model_id.clone().unwrap_or_default(),
        messages: request_messages,
        temperature: model.temperature,
        max_tokens: Some(model.max_output_tokens as u32),
        stream,
        stream_options: if stream {
            Some(StreamOptions {
                include_usage: true,
            })
        } else {
            None
        },
        tools: chat_tools.clone(),
        tool_choice: if stream && chat_tools.is_some() {
            Some("auto".to_string())
        } else {
            None
        },
        extra_body: model.merged_extra_body_with_thinking(thinking_level, model.api_type),
    })
}

#[derive(Clone, Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessagePayload>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ChatToolSpec>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    #[serde(flatten)]
    extra_body: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize)]
struct StreamOptions {
    #[serde(rename = "include_usage")]
    include_usage: bool,
}

#[derive(Clone, Debug, Serialize)]
struct ChatMessagePayload {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ChatToolCallPayload>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

impl ChatMessagePayload {
    fn system(content: String) -> Self {
        Self {
            role: "system".to_string(),
            content: Some(serde_json::Value::String(content)),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    fn user(model: &LlmProviderConfig, message: &Message) -> Result<Self> {
        let content = user_message_content(model, message)?;
        Ok(Self {
            role: "user".to_string(),
            content: Some(content),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        })
    }

    fn assistant(
        content: String,
        tool_calls: Option<Vec<ChatToolCallPayload>>,
        reasoning_content: Option<String>,
    ) -> Self {
        Self {
            role: "assistant".to_string(),
            content: if content.is_empty() {
                if tool_calls.is_some() {
                    // When tool_calls exist but content is empty, set a placeholder
                    // to satisfy API requirements that require either content or tool_calls.
                    Some(serde_json::Value::String("".to_string()))
                } else {
                    None
                }
            } else {
                Some(serde_json::Value::String(content))
            },
            reasoning_content,
            tool_calls,
            tool_call_id: None,
            name: None,
        }
    }

    /// Chat Completions API: tool messages only support `type: "text"` content
    /// parts per the OpenAI spec.  Images are never embedded here — they are
    /// passed via a synthetic user message inserted by `build_openai_request`.
    fn tool(content: String, tool_call_id: Option<String>, name: Option<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(serde_json::Value::String(content)),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id,
            name,
        }
    }

    /// Synthetic user message carrying inline images.  Used when a tool result
    /// contains image attachments and the model supports vision — the Chat
    /// Completions API does not allow `image_url` parts in tool messages, so
    /// the images are forwarded as a separate user message.
    fn synthetic_user(content_parts: Vec<serde_json::Value>) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(serde_json::Value::Array(content_parts)),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }
}

fn user_message_content(model: &LlmProviderConfig, message: &Message) -> Result<serde_json::Value> {
    let text = message_text_with_file_references(message);
    let images: Vec<&MessageAttachment> = image_attachments(message).collect();

    if images.is_empty() {
        return Ok(serde_json::Value::String(text));
    }

    if !model.supports_images {
        anyhow::bail!("current model does not support image attachments");
    }

    let mut parts = Vec::new();
    if !text.trim().is_empty() {
        parts.push(serde_json::json!({
            "type": "text",
            "text": text,
        }));
    }

    for attachment in images {
        if let MessageAttachment::Image { mime, data, .. } = attachment {
            let b64 = BASE64.encode(data);
            parts.push(serde_json::json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{};base64,{}", mime, b64) },
            }));
        }
    }

    if parts.is_empty() {
        parts.push(serde_json::json!({
            "type": "text",
            "text": "",
        }));
    }

    Ok(serde_json::Value::Array(parts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, MessageRole};
    use crate::types::{ApiType, LlmProviderConfig};

    #[test]
    fn openai_system_messages_are_combined() {
        let model = LlmProviderConfig {
            provider_id: "openai".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiChatCompletions,
            model_id: "gpt-4".to_string(),
            request_model_id: Some("gpt-4".to_string()),
            max_output_tokens: 1024,
            temperature: Some(0.7),
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: Some("base system prompt".to_string()),
            api_key: None,
            extra_body: None,
            thinking_level: crate::reasoning::ThinkingLevelType::None,
        };

        // System messages in the message list are now skipped (role match arm is empty).
        // Only model.system_prompt is used as the system message.
        let messages = vec![
            Message::new(MessageRole::User, "Hello"),
            Message::new(MessageRole::System, "Context summary"),
            Message::new(MessageRole::Assistant, "Hi there"),
        ];

        let request =
            build_openai_request(&model, messages, false, &[], model.thinking_level.clone())
                .expect("build request");
        let roles: Vec<_> = request
            .messages
            .iter()
            .map(|msg| msg.role.as_str())
            .collect();

        // Should have exactly one system message at the beginning
        assert_eq!(roles, vec!["system", "user", "assistant"]);

        // Verify the system message content uses only model.system_prompt,
        // without merging the System message from the conversation
        let system_content = request.messages[0].content.as_ref().unwrap();
        let system_text = system_content.as_str().unwrap();
        assert!(system_text.contains("base system prompt"));
        assert!(
            !system_text.contains("Context summary"),
            "System messages from conversation should no longer be merged into system prompt"
        );
    }

    #[test]
    fn openai_system_prompt_only() {
        let model = LlmProviderConfig {
            provider_id: "openai".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiChatCompletions,
            model_id: "gpt-4".to_string(),
            request_model_id: Some("gpt-4".to_string()),
            max_output_tokens: 1024,
            temperature: Some(0.7),
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: Some("base system prompt".to_string()),
            api_key: None,
            extra_body: None,
            thinking_level: crate::reasoning::ThinkingLevelType::None,
        };

        // No System message in messages, only model.system_prompt
        let messages = vec![
            Message::new(MessageRole::User, "Hello"),
            Message::new(MessageRole::Assistant, "Hi there"),
        ];

        let request =
            build_openai_request(&model, messages, false, &[], model.thinking_level.clone())
                .expect("build request");
        let roles: Vec<_> = request
            .messages
            .iter()
            .map(|msg| msg.role.as_str())
            .collect();

        assert_eq!(roles, vec!["system", "user", "assistant"]);

        let system_content = request.messages[0].content.as_ref().unwrap();
        let system_text = system_content.as_str().unwrap();
        assert_eq!(system_text, "base system prompt");
    }

    /// Helper: build a minimal `LlmProviderConfig` for tests.
    fn test_model(supports_images: bool) -> LlmProviderConfig {
        LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.test.com".to_string(),
            api_type: ApiType::OpenAiChatCompletions,
            model_id: "test-model".to_string(),
            request_model_id: Some("test-model".to_string()),
            max_output_tokens: 1024,
            temperature: Some(0.7),
            supports_images,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: None,
            api_key: None,
            extra_body: None,
            thinking_level: crate::reasoning::ThinkingLevelType::None,
        }
    }

    /// Helper: build a Tool message carrying an image attachment.
    fn tool_message_with_image(tool_call_id: &str) -> Message {
        use crate::message::ToolExecutionResult;
        let mut result = ToolExecutionResult::new("Image read successfully.");
        result.attachments.push(MessageAttachment::Image {
            filename: "icon.png".to_string(),
            mime: "image/png".to_string(),
            data: vec![0x89, 0x50, 0x4E, 0x47], // dummy PNG header bytes
            file_size: 4,
        });
        Message::tool_result(tool_call_id, "read", result)
    }

    #[test]
    fn tool_message_with_images_uses_text_only_and_synthetic_user() {
        let model = test_model(true);

        let mut assistant = Message::new(MessageRole::Assistant, "");
        assistant.tool_calls.push(ToolCall {
            id: "call_abc".to_string(),
            name: "read".to_string(),
            arguments: r#"{"file_path":"icon.png"}"#.to_string(),
            thought_signature: None,
        });

        let messages = vec![
            Message::new(MessageRole::User, "describe the icon"),
            assistant,
            tool_message_with_image("call_abc"),
        ];

        let request =
            build_openai_request(&model, messages, false, &[], model.thinking_level.clone())
                .expect("build request");

        let roles: Vec<_> = request
            .messages
            .iter()
            .map(|msg| msg.role.as_str())
            .collect();
        assert_eq!(
            roles,
            vec!["user", "assistant", "tool", "user"],
            "expected synthetic user message after tool"
        );

        // The tool message must be a plain string, never an array.
        let tool_msg = &request.messages[2];
        assert_eq!(tool_msg.role, "tool");
        let content = tool_msg.content.as_ref().unwrap();
        assert!(
            content.is_string(),
            "tool message content must be a plain string, got: {:?}",
            content
        );

        // The synthetic user message must carry image_url parts.
        let synthetic = &request.messages[3];
        assert_eq!(synthetic.role, "user");
        let arr = synthetic.content.as_ref().unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "image_url");
    }

    #[test]
    fn tool_message_images_dropped_when_no_vision() {
        let model = test_model(false);

        let mut assistant = Message::new(MessageRole::Assistant, "");
        assistant.tool_calls.push(ToolCall {
            id: "call_abc".to_string(),
            name: "read".to_string(),
            arguments: r#"{"file_path":"icon.png"}"#.to_string(),
            thought_signature: None,
        });

        let messages = vec![
            Message::new(MessageRole::User, "describe the icon"),
            assistant,
            tool_message_with_image("call_abc"),
        ];

        let request =
            build_openai_request(&model, messages, false, &[], model.thinking_level.clone())
                .expect("build request");

        let roles: Vec<_> = request
            .messages
            .iter()
            .map(|msg| msg.role.as_str())
            .collect();
        // No synthetic user message — images silently dropped.
        assert_eq!(
            roles,
            vec!["user", "assistant", "tool"],
            "no synthetic user when supports_images is false"
        );
    }

    #[test]
    fn reasoning_detail_summary_type_uses_summary_field() {
        let json = serde_json::json!({
            "type": "reasoning.summary",
            "summary": "## Planning\nDetermine the approach",
            "format": "openai-responses-v1",
            "index": 0
        });
        let detail: ReasoningDetail = serde_json::from_value(json).unwrap();
        assert_eq!(detail.detail_type, "reasoning.summary");
        assert_eq!(
            detail.summary.as_deref(),
            Some("## Planning\nDetermine the approach")
        );
        assert!(detail.text.is_none());
        assert_eq!(detail.format.as_deref(), Some("openai-responses-v1"));
        assert_eq!(detail.index, Some(0));
    }

    #[test]
    fn reasoning_detail_text_type_uses_text_field() {
        let json = serde_json::json!({
            "type": "reasoning.text",
            "text": "Let me think step by step...",
            "format": "anthropic-claude-v1",
            "index": 1
        });
        let detail: ReasoningDetail = serde_json::from_value(json).unwrap();
        assert_eq!(detail.detail_type, "reasoning.text");
        assert_eq!(detail.text.as_deref(), Some("Let me think step by step..."));
        assert!(detail.summary.is_none());
    }

    #[test]
    fn reasoning_detail_encrypted_type_has_no_readable_content() {
        let json = serde_json::json!({
            "type": "reasoning.encrypted",
            "data": "encrypted-blob",
            "format": "openai-responses-v1",
            "index": 0
        });
        let detail: ReasoningDetail = serde_json::from_value(json).unwrap();
        assert_eq!(detail.detail_type, "reasoning.encrypted");
        assert!(detail.summary.is_none());
        assert!(detail.text.is_none());
    }

    #[test]
    fn reasoning_detail_delta_with_reasoning_content_tag_stripped() {
        // Simulate a streaming delta where reasoning_content contains <thinking> tags.
        let json = serde_json::json!({
            "choices": [{
                "index": 0,
                "delta": {
                    "reasoning_content": "<thinking>## Step 2</thinking>"
                },
                "finish_reason": null
            }]
        });
        let event: ChatCompletionStreamResponse = serde_json::from_value(json).unwrap();
        let reasoning = event.choices[0]
            .delta
            .reasoning_content
            .as_deref()
            .unwrap_or("");
        let cleaned = crate::think_parser::strip_think_tags(reasoning);
        assert_eq!(cleaned, "## Step 2");
    }

    #[test]
    fn reasoning_details_delta_extracts_content_by_type() {
        // Simulate a streaming delta with reasoning_details array.
        let json = serde_json::json!({
            "choices": [{
                "index": 0,
                "delta": {
                    "reasoning_details": [
                        {
                            "type": "reasoning.summary",
                            "summary": "## Planning",
                            "format": "openai-responses-v1",
                            "index": 0
                        },
                        {
                            "type": "reasoning.summary",
                            "summary": "## Implementation",
                            "format": "openai-responses-v1",
                            "index": 1
                        }
                    ]
                },
                "finish_reason": null
            }]
        });
        let event: ChatCompletionStreamResponse = serde_json::from_value(json).unwrap();
        let details = event.choices[0].delta.reasoning_details.as_ref().unwrap();
        assert_eq!(details.len(), 2);
        assert_eq!(details[0].detail_type, "reasoning.summary");
        assert_eq!(details[0].summary.as_deref(), Some("## Planning"));
        assert_eq!(details[1].detail_type, "reasoning.summary");
        assert_eq!(details[1].summary.as_deref(), Some("## Implementation"));
    }
}

#[derive(Clone, Debug, Deserialize)]
struct ChatCompletionStreamResponse {
    #[serde(default)]
    choices: Vec<ChatCompletionChoice>,
    #[serde(default)]
    usage: Option<ChatCompletionUsage>,
}

#[derive(Clone, Debug, Deserialize)]
struct ChatCompletionUsage {
    #[serde(rename = "prompt_tokens", default)]
    input_tokens: u32,
    #[serde(rename = "completion_tokens", default)]
    output_tokens: u32,
    #[serde(rename = "total_tokens", default)]
    total_tokens: u32,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: u32,
}

#[derive(Clone, Debug, Deserialize)]
struct ChatCompletionChoice {
    #[serde(default)]
    delta: ChatCompletionDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ChatCompletionDelta {
    #[serde(default)]
    content: Option<String>,
    /// Accept both `reasoning` (OpenCode Go, some providers) and
    /// `reasoning_content` (DeepSeek, standard) field names.
    #[serde(default, alias = "reasoning")]
    reasoning_content: Option<String>,
    /// Newer OpenAI Chat Completions API returns structured reasoning
    /// sections in this array (e.g. gpt-5.6-luna interleaved thinking).
    #[serde(default)]
    reasoning_details: Option<Vec<ReasoningDetail>>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatCompletionToolCallDelta>>,
}

/// A single reasoning section from the `reasoning_details` array.
#[derive(Clone, Debug, Default, Deserialize)]
struct ReasoningDetail {
    /// `reasoning.summary` → content in `summary` field,
    /// `reasoning.text` → content in `text` field,
    /// `reasoning.encrypted` → encrypted content, cannot be read.
    #[serde(rename = "type")]
    detail_type: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    text: Option<String>,
    /// Provider format identifier, e.g. `"openai-responses-v1"`.
    #[serde(default)]
    #[allow(dead_code)]
    format: Option<String>,
    /// Position in the reasoning sequence for interleaved thinking.
    #[serde(default)]
    #[allow(dead_code)]
    index: Option<usize>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ChatCompletionToolCallDelta {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ChatCompletionToolCallFunctionDelta>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ChatCompletionToolCallFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ChatCompletionResponse {
    #[serde(default)]
    choices: Vec<ChatCompletionResponseChoice>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ChatCompletionResponseChoice {
    #[serde(default)]
    message: ChatCompletionResponseMessage,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ChatCompletionResponseMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct ChatToolSpec {
    #[serde(rename = "type")]
    kind: String,
    function: ChatToolFunctionSpec,
}

impl From<&ToolDefinition> for ChatToolSpec {
    fn from(definition: &ToolDefinition) -> Self {
        Self {
            kind: "function".to_string(),
            function: ChatToolFunctionSpec {
                name: definition.name.to_string(),
                description: definition.description.clone(),
                parameters: definition.parameters.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct ChatToolFunctionSpec {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Clone, Debug, Serialize)]
struct ChatToolCallPayload {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: ChatToolCallFunctionPayload,
}

impl From<&ToolCall> for ChatToolCallPayload {
    fn from(call: &ToolCall) -> Self {
        Self {
            id: call.id.clone(),
            kind: "function".to_string(),
            function: ChatToolCallFunctionPayload {
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct ChatToolCallFunctionPayload {
    name: String,
    arguments: String,
}
