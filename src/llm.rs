use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    config::ActiveModel,
    session::{AssistantTurn, BackendEvent, Message, MessageRole, ToolCall},
    tooling::ToolDefinition,
};

#[derive(Clone)]
pub struct LlmClient {
    http: Client,
}

impl LlmClient {
    pub fn new() -> Result<Self> {
        let http = Client::builder()
            .user_agent("tidev/0.1")
            .build()
            .context("failed to construct HTTP client")?;

        Ok(Self { http })
    }

    pub async fn stream_chat(
        &self,
        model: ActiveModel,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<BackendEvent>,
    ) {
        if let Err(error) = self
            .stream_chat_inner(model, messages, tools, tx.clone())
            .await
        {
            let _ = tx.send(BackendEvent::Failed(error.to_string()));
        }
    }

    pub async fn complete_with_messages(
        &self,
        model: ActiveModel,
        messages: Vec<Message>,
    ) -> Result<String> {
        let request = self.build_request(&model, messages, false, &[])?;
        let response =
            self.http
                .post(model.endpoint())
                .bearer_auth(model.api_key.clone().with_context(|| {
                    format!("missing API key for provider '{}'", model.provider_id)
                })?)
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

    async fn stream_chat_inner(
        &self,
        model: ActiveModel,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<BackendEvent>,
    ) -> Result<()> {
        let api_key = model
            .api_key
            .clone()
            .with_context(|| format!("missing API key for provider '{}'", model.provider_id))?;
        let request = self.build_request(&model, messages, true, &tools)?;

        let response = self
            .http
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
                        let _ = tx.send(BackendEvent::Finished(turn));
                        return Ok(());
                    }

                    let event: ChatCompletionStreamResponse = serde_json::from_str(payload)
                        .context("failed to parse streaming response")?;

                    for choice in event.choices {
                        if let Some(reasoning) = choice.delta.reasoning_content {
                            reasoning_text.push_str(&reasoning);
                            let _ = tx.send(BackendEvent::ReasoningDelta(reasoning));
                        }

                        if let Some(content) = choice.delta.content {
                            let (visible, reasoning) = think_parser.push(&content);

                            if !visible.is_empty() {
                                assistant_text.push_str(&visible);
                                let _ = tx.send(BackendEvent::Delta(visible));
                            }

                            if !reasoning.is_empty() {
                                reasoning_text.push_str(&reasoning);
                                let _ = tx.send(BackendEvent::ReasoningDelta(reasoning));
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
        let _ = tx.send(BackendEvent::Finished(turn));
        Ok(())
    }

    fn build_request(
        &self,
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
                MessageRole::System => {
                    request_messages.push(ChatMessagePayload::system(message.content))
                }
                MessageRole::User => {
                    request_messages.push(ChatMessagePayload::user(message.content))
                }
                MessageRole::Assistant => request_messages.push(ChatMessagePayload::assistant(
                    message.content,
                    if message.tool_calls.is_empty() {
                        None
                    } else {
                        Some(
                            message
                                .tool_calls
                                .iter()
                                .map(ChatToolCallPayload::from)
                                .collect(),
                        )
                    },
                )),
                MessageRole::Tool => request_messages.push(ChatMessagePayload::tool(
                    message.content,
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
    content: Option<String>,
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
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    fn user(content: String) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    fn assistant(content: String, tool_calls: Option<Vec<ChatToolCallPayload>>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: if content.is_empty() {
                None
            } else {
                Some(content)
            },
            tool_calls,
            tool_call_id: None,
            name: None,
        }
    }

    fn tool(content: String, tool_call_id: Option<String>, name: Option<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content),
            tool_calls: None,
            tool_call_id,
            name,
        }
    }
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
                description: definition.description.to_string(),
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

#[derive(Clone, Debug, Default)]
struct ToolCallBuilder {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallBuilder {
    fn into_tool_call(self, index: usize) -> ToolCall {
        ToolCall {
            id: if self.id.is_empty() {
                format!("tool-call-{index}")
            } else {
                self.id
            },
            name: if self.name.is_empty() {
                "unknown_tool".to_string()
            } else {
                self.name
            },
            arguments: self.arguments,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ThinkParser {
    in_think: bool,
    buffer: String,
}

impl ThinkParser {
    fn push(&mut self, text: &str) -> (String, String) {
        self.buffer.push_str(text);

        let mut visible = String::new();
        let mut reasoning = String::new();

        loop {
            if self.in_think {
                if let Some(end) = self.buffer.find("</think>") {
                    reasoning.push_str(&self.buffer[..end]);
                    self.buffer.drain(..end + "</think>".len());
                    self.in_think = false;
                    continue;
                }

                let keep = think_tag_suffix_len(&self.buffer);
                let split = self.buffer.len().saturating_sub(keep);
                reasoning.push_str(&self.buffer[..split]);
                self.buffer.drain(..split);
                break;
            }

            if let Some(start) = self.buffer.find("<think>") {
                visible.push_str(&self.buffer[..start]);
                self.buffer.drain(..start + "<think>".len());
                self.in_think = true;
                continue;
            }

            let keep = think_tag_suffix_len(&self.buffer);
            let split = self.buffer.len().saturating_sub(keep);
            visible.push_str(&self.buffer[..split]);
            self.buffer.drain(..split);
            break;
        }

        (visible, reasoning)
    }

    fn finish(&mut self) -> (String, String) {
        let mut visible = String::new();
        let mut reasoning = String::new();

        if self.in_think {
            reasoning.push_str(&self.buffer);
        } else {
            visible.push_str(&self.buffer);
        }

        self.buffer.clear();
        (visible, reasoning)
    }
}

fn think_tag_suffix_len(text: &str) -> usize {
    const TAGS: [&str; 2] = ["<think>", "</think>"];

    for tag in TAGS {
        let max = tag.len().saturating_sub(1);
        for keep in (1..=max).rev() {
            if text.ends_with(&tag[..keep]) {
                return keep;
            }
        }
    }

    0
}

fn finalize_turn(
    assistant_text: &mut String,
    reasoning_text: &mut String,
    finish_reason: &mut Option<String>,
    tool_calls: &mut BTreeMap<usize, ToolCallBuilder>,
    think_parser: &mut ThinkParser,
) -> AssistantTurn {
    let (visible, reasoning) = think_parser.finish();
    assistant_text.push_str(&visible);
    reasoning_text.push_str(&reasoning);

    let tool_calls = tool_calls
        .iter()
        .map(|(index, builder)| builder.clone().into_tool_call(*index))
        .collect::<Vec<_>>();

    if finish_reason.is_none() {
        *finish_reason = Some(if tool_calls.is_empty() {
            "stop".to_string()
        } else {
            "tool_calls".to_string()
        });
    }

    AssistantTurn {
        content: assistant_text.clone(),
        reasoning: reasoning_text.clone(),
        tool_calls,
        finish_reason: finish_reason.clone(),
    }
}
