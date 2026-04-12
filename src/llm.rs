use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    config::ActiveModel,
    session::{BackendEvent, Message, MessageRole},
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
        tx: UnboundedSender<BackendEvent>,
    ) {
        if let Err(error) = self.stream_chat_inner(model, messages, tx.clone()).await {
            let _ = tx.send(BackendEvent::Failed(error.to_string()));
        }
    }

    pub async fn complete_with_messages(
        &self,
        model: ActiveModel,
        messages: Vec<Message>,
    ) -> Result<String> {
        let request = self.build_request(&model, messages, false)?;
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
        tx: UnboundedSender<BackendEvent>,
    ) -> Result<()> {
        let api_key = model
            .api_key
            .clone()
            .with_context(|| format!("missing API key for provider '{}'", model.provider_id))?;
        let request = self.build_request(&model, messages, true)?;

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
                        let _ = tx.send(BackendEvent::Finished(assistant_text.clone()));
                        return Ok(());
                    }

                    let event: ChatCompletionStreamResponse = serde_json::from_str(payload)
                        .context("failed to parse streaming response")?;

                    for choice in event.choices {
                        if let Some(content) = choice.delta.content {
                            assistant_text.push_str(&content);
                            let _ = tx.send(BackendEvent::Delta(content));
                        }
                    }
                }
            }
        }

        let _ = tx.send(BackendEvent::Finished(assistant_text));
        Ok(())
    }

    fn build_request(
        &self,
        model: &ActiveModel,
        messages: Vec<Message>,
        stream: bool,
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
                MessageRole::Assistant => {
                    request_messages.push(ChatMessagePayload::assistant(message.content))
                }
                MessageRole::Tool => {
                    request_messages.push(ChatMessagePayload::tool(message.content))
                }
                MessageRole::Error => {}
            }
        }

        Ok(ChatCompletionRequest {
            model: model.model_id.clone(),
            messages: request_messages,
            temperature: Some(model.temperature),
            max_tokens: Some(model.max_output_tokens as u32),
            stream,
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
}

#[derive(Clone, Debug, Serialize)]
struct ChatMessagePayload {
    role: String,
    content: String,
}

impl ChatMessagePayload {
    fn system(content: String) -> Self {
        Self {
            role: "system".to_string(),
            content,
        }
    }

    fn user(content: String) -> Self {
        Self {
            role: "user".to_string(),
            content,
        }
    }

    fn assistant(content: String) -> Self {
        Self {
            role: "assistant".to_string(),
            content,
        }
    }

    fn tool(content: String) -> Self {
        Self {
            role: "tool".to_string(),
            content,
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
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ChatCompletionDelta {
    #[serde(default)]
    content: Option<String>,
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
