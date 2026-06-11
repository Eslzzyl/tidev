use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::{types::LlmProviderConfig, types::ToolDefinition};
use tidev_session::session::{BackendEvent, Message, MessageAttachment, MessageRole, ToolCall};
use tidev_types::reasoning::ThinkingLevelType;

use log::{debug as log_debug, error as log_error};

use crate::attachments::{image_attachments, message_text_with_file_references};
use crate::debug::{save_raw_response_for_debugging, save_request_for_debugging};
use crate::error::classify_response_status;
use crate::think_parser::ThinkParser;
use crate::tool_call_format::ToolCallBuilder;
use crate::turn::finalize_turn;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream_openai(
    http: &Client,
    session_id: Uuid,
    request_id: u64,
    model: LlmProviderConfig,
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    tx: UnboundedSender<BackendEvent>,
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
    let mut buffer = String::new();
    let mut assistant_text = String::new();
    let mut reasoning_text = String::new();
    let mut finish_reason: Option<String> = None;
    let mut tool_calls: BTreeMap<usize, ToolCallBuilder> = BTreeMap::new();
    let mut think_parser = ThinkParser::default();
    let mut first_delta_time: Option<std::time::Instant> = None;
    let mut raw_payloads: Vec<String> = Vec::new();

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

            if let Some(payload) = line.strip_prefix("data:") {
                let payload = payload.trim();

                raw_payloads.push(payload.to_string());
                if payload == "[DONE]" {
                    save_raw_response_for_debugging(
                        session_id,
                        request_id,
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
                    let _ = tx.send(BackendEvent::Finished {
                        session_id,
                        request_id,
                        turn,
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
                    let _ = tx.send(BackendEvent::UsageStats {
                        session_id,
                        request_id,
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
                        if first_delta_time.is_none() {
                            first_delta_time = Some(std::time::Instant::now());
                        }
                        reasoning_text.push_str(&reasoning);
                        let _ = tx.send(BackendEvent::ReasoningDelta {
                            session_id,
                            request_id,
                            content: reasoning,
                        });
                    }

                    if let Some(content) = choice.delta.content {
                        if first_delta_time.is_none() {
                            first_delta_time = Some(std::time::Instant::now());
                        }
                        let (visible, reasoning) = think_parser.push(&content);

                        if !visible.is_empty() {
                            assistant_text.push_str(&visible);
                            let _ = tx.send(BackendEvent::Delta {
                                session_id,
                                request_id,
                                content: visible,
                            });
                        }

                        if !reasoning.is_empty() {
                            reasoning_text.push_str(&reasoning);
                            let _ = tx.send(BackendEvent::ReasoningDelta {
                                session_id,
                                request_id,
                                content: reasoning,
                            });
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
                                if let Some(name) = &function.name {
                                    entry.name = name.clone();
                                }

                                if let Some(arguments) = &function.arguments {
                                    entry.arguments.push_str(arguments);
                                }
                            }

                            if !entry.id.is_empty() && !entry.name.is_empty() {
                                let _ = tx.send(BackendEvent::ToolCallUpdated {
                                    session_id,
                                    request_id,
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

    save_raw_response_for_debugging(
        session_id,
        request_id,
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
    let _ = tx.send(BackendEvent::Finished {
        session_id,
        request_id,
        turn,
    });
    Ok(())
}

pub(crate) async fn complete_openai(
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

    let response: ChatCompletionResponse = response.json().await?;
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
            MessageRole::Tool => request_messages.push(ChatMessagePayload::tool(
                message_text_with_file_references(message),
                image_attachments(message).collect(),
                message.tool_call_id.clone(),
                message.tool_name.clone(),
            )),
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
        extra_body: model.merged_extra_body_with_thinking(thinking_level),
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

    fn tool(
        content: String,
        images: Vec<&MessageAttachment>,
        tool_call_id: Option<String>,
        name: Option<String>,
    ) -> Self {
        let content = if images.is_empty() {
            Some(serde_json::Value::String(content))
        } else {
            let mut parts = Vec::new();
            if !content.trim().is_empty() {
                parts.push(serde_json::json!({
                    "type": "text",
                    "text": content,
                }));
            }
            for attachment in images {
                if let MessageAttachment::Image { data_url, .. } = attachment {
                    parts.push(serde_json::json!({
                        "type": "image_url",
                        "image_url": { "url": data_url },
                    }));
                }
            }
            if parts.is_empty() {
                parts.push(serde_json::json!({
                    "type": "text",
                    "text": "",
                }));
            }
            Some(serde_json::Value::Array(parts))
        };
        Self {
            role: "tool".to_string(),
            content,
            reasoning_content: None,
            tool_calls: None,
            tool_call_id,
            name,
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
        if let MessageAttachment::Image { data_url, .. } = attachment {
            parts.push(serde_json::json!({
                "type": "image_url",
                "image_url": { "url": data_url },
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
    use crate::types::{ApiType, LlmProviderConfig};
    use tidev_session::session::{Message, MessageRole};

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
            system_prompt: Some("base system prompt".to_string()),
            api_key: None,
            extra_body: None,
            thinking_level: tidev_types::reasoning::ThinkingLevelType::None,
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
            system_prompt: Some("base system prompt".to_string()),
            api_key: None,
            extra_body: None,
            thinking_level: tidev_types::reasoning::ThinkingLevelType::None,
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
    #[serde(default)]
    tool_calls: Option<Vec<ChatCompletionToolCallDelta>>,
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
