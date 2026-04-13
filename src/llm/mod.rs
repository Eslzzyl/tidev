mod anthropic;
mod attachments;
mod openai;
mod think_parser;

use anyhow::{Context, Result};
use reqwest::Client;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    config::{ActiveModel, ApiType},
    session::{BackendEvent, Message},
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
        request_id: u64,
        model: ActiveModel,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<BackendEvent>,
    ) {
        if let Err(error) = self
            .stream_chat_inner(request_id, model, messages, tools, tx.clone())
            .await
        {
            let _ = tx.send(BackendEvent::Failed {
                request_id,
                error: error.to_string(),
            });
        }
    }

    pub async fn complete_with_messages(
        &self,
        model: ActiveModel,
        messages: Vec<Message>,
    ) -> Result<String> {
        match model.api_type {
            ApiType::Anthropic => anthropic::complete_anthropic(&self.http, model, messages).await,
            ApiType::OpenAi => openai::complete_openai(&self.http, model, messages).await,
        }
    }

    async fn stream_chat_inner(
        &self,
        request_id: u64,
        model: ActiveModel,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<BackendEvent>,
    ) -> Result<()> {
        match model.api_type {
            ApiType::Anthropic => {
                anthropic::stream_anthropic(&self.http, request_id, model, messages, tools, tx)
                    .await
            }
            ApiType::OpenAi => {
                openai::stream_openai(&self.http, request_id, model, messages, tools, tx).await
            }
        }
    }
}
