use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::{types::LlmProviderConfig, types::ToolDefinition};
use tidev_session::session::{BackendEvent, Message, MessageAttachment, MessageRole};

use log::{debug as log_debug, error as log_error};

use crate::attachments::{image_attachments, message_text_with_file_references};
use crate::debug::save_request_for_debugging;
use crate::error::classify_response_status;
use crate::think_parser::ThinkParser;
use crate::tool_call_format::ToolCallBuilder;
use crate::turn::finalize_turn;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream_anthropic(
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
    let request = build_anthropic_request(&model, messages, &tools)?;
    let request_body = serde_json::to_string(&request).unwrap_or_default();
    let request_body_size = request_body.len();
    save_request_for_debugging(&request_body, save_request_body, max_request_files);

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
                return Err(classify_response_status(status, Some(error_body)).into());
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
    let mut thinking_blocks: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    let mut think_parser = ThinkParser::default();
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
                        AnthropicDelta::Text { text } => {
                            if first_delta_time.is_none() {
                                first_delta_time = Some(std::time::Instant::now());
                            }
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
                        AnthropicDelta::InputJson { partial_json } => {
                            if first_delta_time.is_none() {
                                first_delta_time = Some(std::time::Instant::now());
                            }
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
                        AnthropicDelta::Thinking { thinking } => {
                            if first_delta_time.is_none() {
                                first_delta_time = Some(std::time::Instant::now());
                            }
                            if thinking_blocks.contains(&index) {
                                reasoning_text.push_str(&thinking);
                                let _ = tx.send(BackendEvent::ReasoningDelta {
                                    session_id,
                                    request_id,
                                    content: thinking,
                                });
                            }
                        }
                        AnthropicDelta::Signature { .. } => {
                            // Signature delta marks the end of a thinking block.
                            // We don't need to store the signature for display.
                        }
                    },
                    AnthropicStreamEvent::ContentBlockStart {
                        index,
                        content_block,
                    } => match content_block {
                        AnthropicContentBlockStart::Text { .. } => {
                            if first_delta_time.is_none() {
                                first_delta_time = Some(std::time::Instant::now());
                            }
                        }
                        AnthropicContentBlockStart::Thinking { .. } => {
                            if first_delta_time.is_none() {
                                first_delta_time = Some(std::time::Instant::now());
                            }
                            thinking_blocks.insert(index);
                        }
                        AnthropicContentBlockStart::ToolUse { id, name } => {
                            if first_delta_time.is_none() {
                                first_delta_time = Some(std::time::Instant::now());
                            }
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
                    AnthropicStreamEvent::MessageDelta { delta, usage } => {
                        if let Some(stop_reason) = delta.stop_reason {
                            finish_reason = Some(stop_reason);
                        }
                        if let Some(usage) = usage {
                            let total_tokens = usage.input_tokens + usage.output_tokens;
                            let duration_ms =
                                first_delta_time.map(|start| start.elapsed().as_millis() as u64);
                            let _ = tx.send(BackendEvent::UsageStats {
                                session_id,
                                request_id,
                                input_tokens: usage.input_tokens,
                                output_tokens: usage.output_tokens,
                                total_tokens,
                                cache_read_tokens: usage.cache_read_input_tokens,
                                cache_write_tokens: usage.cache_creation_input_tokens,
                                model_id: format!("{}:{}", model.provider_id, model.model_id),
                                duration_ms,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }

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

pub(crate) async fn complete_anthropic(
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
    let request = build_anthropic_request(&model, messages, &tools)?;
    let request_body = serde_json::to_string(&request).unwrap_or_default();
    let request_body_size = request_body.len();
    save_request_for_debugging(&request_body, save_request_body, max_request_files);

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
                return Err(classify_response_status(status, Some(error_body)).into());
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

/// Set `cache_control: ephemeral` on the last content block of the last message.
/// This marks a cache breakpoint so the conversation prefix is cached by Anthropic.
fn apply_cache_to_last_message(messages: &mut [AnthropicMessage]) {
    if let Some(last_msg) = messages.last_mut()
        && let Some(last_block) = last_msg.content.last_mut()
    {
        match last_block {
            AnthropicContentBlock::Text { cache_control, .. }
            | AnthropicContentBlock::ToolResult { cache_control, .. } => {
                *cache_control = Some(CacheControl::ephemeral());
            }
            AnthropicContentBlock::Thinking { .. }
            | AnthropicContentBlock::Image { .. }
            | AnthropicContentBlock::ToolUse { .. } => {}
        }
    }
}

fn build_anthropic_request(
    model: &LlmProviderConfig,
    messages: Vec<Message>,
    tools: &[ToolDefinition],
) -> Result<AnthropicRequest> {
    // System prompt comes from the model config directly.
    // Always use Blocks format with cache_control for prompt caching.
    let system_prompt = if model.system_prompt_str().trim().is_empty() {
        None
    } else {
        Some(SystemPrompt::Blocks(vec![SystemBlock {
            block_type: "text".to_string(),
            text: model.system_prompt.clone().unwrap_or_default(),
            cache_control: Some(CacheControl::ephemeral()),
        }]))
    };

    let mut anthropic_messages: Vec<AnthropicMessage> = Vec::new();

    // Process only User/Assistant/Tool messages (System messages already handled above)
    for message in messages {
        if message.streaming {
            continue;
        }

        match message.role {
            MessageRole::System => {}
            MessageRole::User => {
                let content = user_message_content(model, &message)?;
                // Merge consecutive user messages (including those from tool results)
                // into a single Anthropic user message, as required by the API.
                if let Some(last) = anthropic_messages.last_mut()
                    && last.role == "user"
                {
                    last.content.extend(content);
                } else {
                    anthropic_messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content,
                    });
                }
            }
            MessageRole::Assistant => {
                let mut content = Vec::new();
                let text = message_text_with_file_references(&message);
                if !text.is_empty() {
                    content.push(AnthropicContentBlock::Text {
                        text,
                        cache_control: None,
                    });
                }
                for tool_call in &message.tool_calls {
                    content.push(AnthropicContentBlock::ToolUse {
                        id: tool_call.id.clone(),
                        name: tool_call.name.clone(),
                        input: serde_json::from_str(&tool_call.arguments)
                            .unwrap_or(serde_json::Value::Object(Default::default())),
                        cache_control: None,
                    });
                }
                if !message.reasoning.is_empty() {
                    content.push(AnthropicContentBlock::Thinking {
                        thinking: message.reasoning.clone(),
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
                let block = AnthropicContentBlock::ToolResult {
                    tool_use_id: tool_call_id,
                    content,
                    cache_control: None,
                };
                // Merge with the last user message if present, so all tool results
                // for the same assistant turn live in a single user message.
                if let Some(last) = anthropic_messages.last_mut()
                    && last.role == "user"
                {
                    last.content.push(block);
                } else {
                    anthropic_messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: vec![block],
                    });
                }
            }
            MessageRole::Error => {}
            MessageRole::Shell => {}
        }
    }

    let anthropic_tools = if tools.is_empty() {
        None
    } else {
        let mut native_tools: Vec<AnthropicTool> = tools
            .iter()
            .map(|t| AnthropicTool {
                name: t.name.to_string(),
                description: t.description.to_string(),
                input_schema: t.parameters.clone(),
                cache_control: None,
            })
            .collect();
        // Cache the last tool definition (caches all tool definitions)
        if let Some(last_tool) = native_tools.last_mut() {
            last_tool.cache_control = Some(CacheControl::ephemeral());
        }
        Some(native_tools)
    };

    // For multi-turn conversations, set cache_control on the last message's
    // last content block so the conversation prefix is cached for the next request.
    if anthropic_messages.len() > 1 {
        apply_cache_to_last_message(&mut anthropic_messages);
    }

    Ok(AnthropicRequest {
        model: model.request_model_id.clone().unwrap_or_default(),
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
struct CacheControl {
    #[serde(rename = "type")]
    cache_type: String,
}

impl CacheControl {
    fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct SystemBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
enum SystemPrompt {
    #[allow(dead_code)]
    String(String),
    Blocks(Vec<SystemBlock>),
}

#[derive(Clone, Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<SystemPrompt>,
    messages: Vec<AnthropicMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
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
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    Thinking {
        thinking: String,
    },
    Image {
        source: AnthropicImageSource,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
}

#[derive(Clone, Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Clone, Debug, Serialize)]
struct AnthropicImageSource {
    #[serde(rename = "type")]
    kind: String,
    media_type: String,
    data: String,
}

fn user_message_content(
    model: &LlmProviderConfig,
    message: &Message,
) -> Result<Vec<AnthropicContentBlock>> {
    let text = message_text_with_file_references(message);
    let images: Vec<&MessageAttachment> = image_attachments(message).collect();

    if images.is_empty() {
        return Ok(vec![AnthropicContentBlock::Text {
            text,
            cache_control: None,
        }]);
    }

    if !model.supports_images {
        anyhow::bail!("current model does not support image attachments");
    }

    let mut content = Vec::new();
    if !text.trim().is_empty() {
        content.push(AnthropicContentBlock::Text {
            text,
            cache_control: None,
        });
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
            cache_control: None,
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
    Thinking {
        #[allow(dead_code)]
        thinking: Option<String>,
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
    Text {
        text: String,
    },
    InputJson {
        partial_json: String,
    },
    Thinking {
        thinking: String,
    },
    #[allow(dead_code)]
    Signature {
        signature: String,
    },
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
