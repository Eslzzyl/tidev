use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    config::ActiveModel,
    session::{BackendEvent, Message, MessageAttachment, MessageRole, ToolCall},
    tooling::ToolDefinition,
};

use super::attachments::{image_attachments, message_text_with_file_references};
use super::think_parser::{ThinkParser, ToolCallBuilder, finalize_turn};

pub(super) async fn stream_openai(
    http: &Client,
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
    let request = build_openai_request(&model, messages, true, &tools)?;

    let response = http
        .post(model.endpoint())
        .bearer_auth(api_key)
        .json(&request)
        .send()
        .await?
        .error_for_status()?;

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

                if payload == "[DONE]" {
                    let turn = finalize_turn(
                        &mut assistant_text,
                        &mut reasoning_text,
                        &mut finish_reason,
                        &mut tool_calls,
                        &mut think_parser,
                    );
                    let _ = tx.send(BackendEvent::Finished { request_id, turn });
                    return Ok(());
                }

                let event: ChatCompletionStreamResponse =
                    serde_json::from_str(payload).context("failed to parse streaming response")?;

                for choice in event.choices {
                    if let Some(reasoning) = choice.delta.reasoning_content {
                        reasoning_text.push_str(&reasoning);
                        let _ = tx.send(BackendEvent::ReasoningDelta {
                            request_id,
                            content: reasoning,
                        });
                    }

                    if let Some(content) = choice.delta.content {
                        let (visible, reasoning) = think_parser.push(&content);

                        if !visible.is_empty() {
                            assistant_text.push_str(&visible);
                            let _ = tx.send(BackendEvent::Delta {
                                request_id,
                                content: visible,
                            });
                        }

                        if !reasoning.is_empty() {
                            reasoning_text.push_str(&reasoning);
                            let _ = tx.send(BackendEvent::ReasoningDelta {
                                request_id,
                                content: reasoning,
                            });
                        }
                    }

                    for tool_call in choice.delta.tool_calls {
                        let index = tool_call.index.unwrap_or(tool_calls.len());
                        let entry = tool_calls.entry(index).or_default();

                        if let Some(id) = tool_call.id {
                            entry.id = id;
                        }

                        if let Some(function) = tool_call.function {
                            if let Some(name) = function.name {
                                entry.name = name;
                            }

                            if let Some(arguments) = function.arguments {
                                entry.arguments.push_str(&arguments);
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

    let turn = finalize_turn(
        &mut assistant_text,
        &mut reasoning_text,
        &mut finish_reason,
        &mut tool_calls,
        &mut think_parser,
    );
    let _ = tx.send(BackendEvent::Finished { request_id, turn });
    Ok(())
}

pub(super) async fn complete_openai(
    http: &Client,
    model: ActiveModel,
    messages: Vec<Message>,
) -> Result<String> {
    let request = build_openai_request(&model, messages, false, &[])?;
    let response =
        http.post(model.endpoint())
            .bearer_auth(
                model.api_key.clone().with_context(|| {
                    format!("missing API key for provider '{}'", model.provider_id)
                })?,
            )
            .json(&request)
            .send()
            .await?
            .error_for_status()?;

    let response: ChatCompletionResponse = response.json().await?;
    let content = response
        .choices
        .into_iter()
        .find_map(|choice| choice.message.content)
        .unwrap_or_default();

    Ok(content)
}

fn build_openai_request(
    model: &ActiveModel,
    messages: Vec<Message>,
    stream: bool,
    tools: &[ToolDefinition],
) -> Result<ChatCompletionRequest> {
    let mut request_messages = Vec::new();

    if !model.system_prompt.trim().is_empty() {
        request_messages.push(ChatMessagePayload::system(model.system_prompt.clone()));
    }

    for message in messages {
        if message.streaming {
            continue;
        }

        match message.role {
            MessageRole::System => request_messages.push(ChatMessagePayload::system(
                message_text_with_file_references(&message),
            )),
            MessageRole::User => request_messages.push(ChatMessagePayload::user(model, &message)?),
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
                    message_text_with_file_references(&message),
                    tool_calls,
                ))
            }
            MessageRole::Tool => request_messages.push(ChatMessagePayload::tool(
                message_text_with_file_references(&message),
                message.tool_call_id,
                message.tool_name,
            )),
            MessageRole::Error => {}
        }
    }

    let chat_tools = if tools.is_empty() {
        None
    } else {
        Some(tools.iter().map(ChatToolSpec::from).collect())
    };

    Ok(ChatCompletionRequest {
        model: model.model_id.clone(),
        messages: request_messages,
        temperature: Some(model.temperature),
        max_tokens: Some(model.max_output_tokens as u32),
        stream,
        tools: chat_tools.clone(),
        tool_choice: if stream && chat_tools.is_some() {
            Some("auto".to_string())
        } else {
            None
        },
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
    tools: Option<Vec<ChatToolSpec>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct ChatMessagePayload {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
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
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    fn user(model: &ActiveModel, message: &Message) -> Result<Self> {
        let content = user_message_content(model, message)?;
        Ok(Self {
            role: "user".to_string(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        })
    }

    fn assistant(content: String, tool_calls: Option<Vec<ChatToolCallPayload>>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: if content.is_empty() {
                None
            } else {
                Some(serde_json::Value::String(content))
            },
            tool_calls,
            tool_call_id: None,
            name: None,
        }
    }

    fn tool(content: String, tool_call_id: Option<String>, name: Option<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(serde_json::Value::String(content)),
            tool_calls: None,
            tool_call_id,
            name,
        }
    }
}

fn user_message_content(model: &ActiveModel, message: &Message) -> Result<serde_json::Value> {
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

#[derive(Clone, Debug, Deserialize)]
struct ChatCompletionStreamResponse {
    #[serde(default)]
    choices: Vec<ChatCompletionChoice>,
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
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChatCompletionToolCallDelta>,
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
