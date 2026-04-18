use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::{
    config::ActiveModel,
    log_debug, log_error,
    session::{BackendEvent, Message, MessageAttachment, MessageRole},
    tooling::ToolDefinition,
};

use super::attachments::{image_attachments, message_text_with_file_references};
use super::think_parser::{ThinkParser, ToolCallBuilder, finalize_turn};

pub(super) async fn stream_anthropic(
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
    let request = build_anthropic_request(&model, messages, &tools)?;
    let request_body_size = serde_json::to_string(&request)
        .map(|s| s.len())
        .unwrap_or(0);

    let send_result = http
        .post(model.endpoint())
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-dangerous-direct-browser-access", "true")
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
                    "anthropic request failed: method=POST url={} request_body_size={} status={} error_body={}",
                    model.endpoint(),
                    request_body_size,
                    status,
                    error_body
                );
                return Err(anyhow::anyhow!("HTTP error {}: {}", status, error_body));
            }
        }
        Err(e) => {
            log_error!(
                "anthropic request failed: method=POST url={} request_body_size={} error={}",
                model.endpoint(),
                request_body_size,
                e
            );
            return Err(e.into());
        }
    };

    log_debug!(
        "anthropic request: method=POST url={} request_body_size={} status={}",
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

                if payload.is_empty() {
                    continue;
                }

                let event: AnthropicStreamEvent = match serde_json::from_str(payload) {
                    Ok(event) => event,
                    Err(_) => continue,
                };

                match event {
                    AnthropicStreamEvent::ContentBlockDelta { delta, index } => match delta {
                        AnthropicDelta::TextDelta { text } => {
                            let (visible, reasoning) = think_parser.push(&text);
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
                        AnthropicDelta::InputJsonDelta { partial_json } => {
                            let entry = tool_calls.entry(index).or_default();
                            entry.arguments.push_str(&partial_json);

                            if !entry.id.is_empty() && !entry.name.is_empty() {
                                let _ = tx.send(BackendEvent::ToolCallUpdated {
                                    session_id,
                                    request_id,
                                    tool_call: entry.clone().into_tool_call(index),
                                });
                            }
                        }
                    },
                    AnthropicStreamEvent::ContentBlockStart {
                        index,
                        content_block,
                    } => match content_block {
                        AnthropicContentBlockStart::Text { .. } => {}
                        AnthropicContentBlockStart::ToolUse { id, name } => {
                            let entry = tool_calls.entry(index).or_default();
                            entry.id = id;
                            entry.name = name;

                            let _ = tx.send(BackendEvent::ToolCallUpdated {
                                session_id,
                                request_id,
                                tool_call: entry.clone().into_tool_call(index),
                            });
                        }
                    },
                    AnthropicStreamEvent::MessageStop => {
                        let turn = finalize_turn(
                            &mut assistant_text,
                            &mut reasoning_text,
                            &mut finish_reason,
                            &mut tool_calls,
                            &mut think_parser,
                        );
                        let _ = tx.send(BackendEvent::Finished {
                            session_id,
                            request_id,
                            turn,
                        });
                        return Ok(());
                    }
                    AnthropicStreamEvent::MessageDelta { delta, usage } => {
                        if let Some(stop_reason) = delta.stop_reason {
                            finish_reason = Some(stop_reason);
                        }
                        if let Some(usage) = usage {
                            let total_tokens = usage.input_tokens + usage.output_tokens;
                            let _ = tx.send(BackendEvent::UsageStats {
                                session_id,
                                request_id,
                                input_tokens: usage.input_tokens,
                                output_tokens: usage.output_tokens,
                                total_tokens,
                                cache_read_tokens: usage.cache_read_input_tokens,
                                cache_write_tokens: usage.cache_creation_input_tokens,
                                model_id: model.model_id.clone(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let turn = finalize_turn(
        &mut assistant_text,
        &mut reasoning_text,
        &mut finish_reason,
        &mut tool_calls,
        &mut think_parser,
    );
    let _ = tx.send(BackendEvent::Finished {
        session_id,
        request_id,
        turn,
    });
    Ok(())
}

pub(super) async fn complete_anthropic(
    http: &Client,
    model: ActiveModel,
    messages: Vec<Message>,
) -> Result<String> {
    let api_key = model
        .api_key
        .clone()
        .with_context(|| format!("missing API key for provider '{}'", model.provider_id))?;
    let request = build_anthropic_request(&model, messages, &[])?;
    let request_body_size = serde_json::to_string(&request)
        .map(|s| s.len())
        .unwrap_or(0);

    let send_result = http
        .post(model.endpoint())
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-dangerous-direct-browser-access", "true")
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
                    "anthropic request (complete) failed: method=POST url={} request_body_size={} status={} error_body={}",
                    model.endpoint(),
                    request_body_size,
                    status,
                    error_body
                );
                return Err(anyhow::anyhow!("HTTP error {}: {}", status, error_body));
            }
        }
        Err(e) => {
            log_error!(
                "anthropic request (complete) failed: method=POST url={} request_body_size={} error={}",
                model.endpoint(),
                request_body_size,
                e
            );
            return Err(e.into());
        }
    };

    log_debug!(
        "anthropic request (complete): method=POST url={} request_body_size={} status={}",
        model.endpoint(),
        request_body_size,
        response.status()
    );

    let response: AnthropicResponse = response.json().await?;
    let content = response
        .content
        .into_iter()
        .filter_map(|block| {
            if let AnthropicContentBlockResponse::Text { text } = block {
                Some(text)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");

    Ok(content)
}

fn build_anthropic_request(
    model: &ActiveModel,
    messages: Vec<Message>,
    tools: &[ToolDefinition],
) -> Result<AnthropicRequest> {
    // Extract context summary from System messages (from context compaction)
    // The System message, if present, contains the compression summary and should be
    // combined with the model's system prompt into a single system prompt.
    let context_summary: Option<String> = messages
        .iter()
        .filter(|message| !message.streaming)
        .filter(|message| message.role == MessageRole::System)
        .map(message_text_with_file_references)
        .next();

    // Build combined system prompt: model.system_prompt + context summary
    let system_prompt = match (
        model.system_prompt.trim().is_empty(),
        context_summary.as_ref().map(|s| s.trim().is_empty()),
    ) {
        (false, Some(false)) => {
            // Both present and non-empty: combine them
            Some(format!(
                "{}\n\n{}",
                model.system_prompt.trim(),
                context_summary.as_ref().unwrap().trim()
            ))
        }
        (false, _) => {
            // Only model.system_prompt present
            Some(model.system_prompt.clone())
        }
        (true, Some(false)) => {
            // Only context_summary present
            context_summary
        }
        (true, _) => {
            // Neither present
            None
        }
    };

    let mut anthropic_messages = Vec::new();

    // Process only User/Assistant/Tool messages (System messages already handled above)
    for message in messages {
        if message.streaming {
            continue;
        }

        match message.role {
            MessageRole::System => {}
            MessageRole::User => {
                anthropic_messages.push(AnthropicMessage {
                    role: "user".to_string(),
                    content: user_message_content(model, &message)?,
                });
            }
            MessageRole::Assistant => {
                let mut content = Vec::new();
                let text = message_text_with_file_references(&message);
                if !text.is_empty() {
                    content.push(AnthropicContentBlock::Text { text });
                }
                for tool_call in &message.tool_calls {
                    content.push(AnthropicContentBlock::ToolUse {
                        id: tool_call.id.clone(),
                        name: tool_call.name.clone(),
                        input: serde_json::from_str(&tool_call.arguments)
                            .unwrap_or(serde_json::Value::Object(Default::default())),
                    });
                }
                anthropic_messages.push(AnthropicMessage {
                    role: "assistant".to_string(),
                    content,
                });
            }
            MessageRole::Tool => {
                let tool_call_id = message.tool_call_id.clone().unwrap_or_default();
                let content = message_text_with_file_references(&message);
                anthropic_messages.push(AnthropicMessage {
                    role: "user".to_string(),
                    content: vec![AnthropicContentBlock::ToolResult {
                        tool_use_id: tool_call_id,
                        content,
                    }],
                });
            }
            MessageRole::Error => {}
        }
    }

    let anthropic_tools = if tools.is_empty() {
        None
    } else {
        Some(
            tools
                .iter()
                .map(|t| AnthropicTool {
                    name: t.name.to_string(),
                    description: t.description.to_string(),
                    input_schema: t.parameters.clone(),
                })
                .collect(),
        )
    };

    Ok(AnthropicRequest {
        model: model.request_model_id.clone(),
        max_tokens: model.max_output_tokens as u32,
        system: system_prompt,
        messages: anthropic_messages,
        stream: true,
        temperature: model.temperature,
        tools: anthropic_tools,
        extra_body: model.extra_body.clone(),
    })
}

#[derive(Clone, Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    stream: bool,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(flatten)]
    extra_body: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum AnthropicContentBlock {
    Text {
        text: String,
    },
    Image {
        source: AnthropicImageSource,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Clone, Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Clone, Debug, Serialize)]
struct AnthropicImageSource {
    #[serde(rename = "type")]
    kind: String,
    media_type: String,
    data: String,
}

fn user_message_content(
    model: &ActiveModel,
    message: &Message,
) -> Result<Vec<AnthropicContentBlock>> {
    let text = message_text_with_file_references(message);
    let images: Vec<&MessageAttachment> = image_attachments(message).collect();

    if images.is_empty() {
        return Ok(vec![AnthropicContentBlock::Text { text }]);
    }

    if !model.supports_images {
        anyhow::bail!("current model does not support image attachments");
    }

    let mut content = Vec::new();
    if !text.trim().is_empty() {
        content.push(AnthropicContentBlock::Text { text });
    }

    for attachment in images {
        if let MessageAttachment::Image { mime, data_url, .. } = attachment {
            let data = data_url
                .split_once(',')
                .map(|(_, data)| data.to_string())
                .unwrap_or_else(|| data_url.clone());
            content.push(AnthropicContentBlock::Image {
                source: AnthropicImageSource {
                    kind: "base64".to_string(),
                    media_type: mime.clone(),
                    data,
                },
            });
        }
    }

    if content.is_empty() {
        content.push(AnthropicContentBlock::Text {
            text: String::new(),
        });
    }

    Ok(content)
}

#[derive(Clone, Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlockResponse>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
enum AnthropicContentBlockResponse {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
enum AnthropicStreamEvent {
    MessageStart {
        message: AnthropicMessageInfo,
    },
    ContentBlockStart {
        index: usize,
        content_block: AnthropicContentBlockStart,
    },
    ContentBlockDelta {
        index: usize,
        delta: AnthropicDelta,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        delta: AnthropicMessageDelta,
        usage: Option<AnthropicUsage>,
    },
    MessageStop,
}

#[derive(Clone, Debug, Deserialize)]
struct AnthropicMessageInfo {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    role: String,
    #[allow(dead_code)]
    model: String,
    #[allow(dead_code)]
    stop_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum AnthropicContentBlockStart {
    Text {
        #[allow(dead_code)]
        text: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum AnthropicDelta {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
}

#[derive(Clone, Debug, Deserialize)]
struct AnthropicMessageDelta {
    stop_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct AnthropicUsage {
    #[allow(dead_code)]
    output_tokens: u32,
    #[serde(rename = "input_tokens", default)]
    input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
}
