mod anthropic;
mod attachments;
mod debug;
mod error;
mod openai;
mod responses;
mod think_parser;
mod tool_call_format;
mod turn;

use anyhow::{Context, Result};
use reqwest::Client;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::{
    config::{ActiveModel, ApiType},
    config::LogConfig,
    session::{BackendEvent, Message},
    tooling::ToolDefinition,
};

use error::{MAX_RETRIES, backoff_delay, backoff_sleep, classify_anyhow_error};

#[derive(Clone, Debug)]
pub struct LlmClient {
    http: Client,
    pub save_request_body: bool,
    pub max_request_files: usize,
}

impl LlmClient {
    pub fn new(logging: &LogConfig) -> Result<Self> {
        let http = Client::builder()
            .user_agent("tidev/0.1")
            .timeout(Duration::from_mins(30))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .context("failed to construct HTTP client")?;

        Ok(Self {
            http,
            save_request_body: logging.save_request_body,
            max_request_files: logging.max_request_files,
        })
    }

    /// Get a reference to the HTTP client for reuse.
    pub fn http(&self) -> &Client {
        &self.http
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn stream_chat(
        &self,
        session_id: Uuid,
        request_id: u64,
        model: ActiveModel,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<BackendEvent>,
        thinking_level: crate::config::reasoning::ThinkingLevelType,
    ) {
        let result = self
            .stream_chat_with_retry(
                session_id,
                request_id,
                model,
                messages,
                tools,
                tx.clone(),
                thinking_level,
            )
            .await;

        if let Err(error) = result {
            let _ = tx.send(BackendEvent::Failed {
                session_id,
                request_id,
                error: error.to_string(),
            });
        }
    }

    pub async fn complete_with_messages(
        &self,
        model: ActiveModel,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
    ) -> Result<String> {
        let result = self.complete_with_retry(model, messages, tools).await;

        result.context("LLM completion failed after retries")
    }

    #[allow(clippy::too_many_arguments)]
    async fn stream_chat_with_retry(
        &self,
        session_id: Uuid,
        request_id: u64,
        model: ActiveModel,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<BackendEvent>,
        thinking_level: crate::config::reasoning::ThinkingLevelType,
    ) -> Result<()> {
        for attempt in 1..=MAX_RETRIES {
            let result = self
                .stream_chat_inner(
                    session_id,
                    request_id,
                    model.clone(),
                    messages.clone(),
                    tools.clone(),
                    tx.clone(),
                    thinking_level.clone(),
                )
                .await;

            match result {
                Ok(()) => return Ok(()),
                Err(e) => {
                    let network_error = classify_anyhow_error(e);
                    let is_last_attempt = attempt == MAX_RETRIES;

                    if !network_error.is_retryable() || is_last_attempt {
                        // Return the final error (non-retryable or exhausted retries)
                        return Err(anyhow::anyhow!("{}", network_error.message()));
                    }

                    let delay_secs = backoff_delay(attempt).as_secs() as u32;

                    let _ = tx.send(BackendEvent::Retrying {
                        session_id,
                        request_id,
                        attempt,
                        max_attempts: MAX_RETRIES,
                        reason: network_error.message().to_string(),
                        retry_after_secs: Some(delay_secs),
                    });

                    backoff_sleep(attempt).await;
                }
            }
        }

        unreachable!("loop should return before this point")
    }

    /// Internal: complete with retry logic.
    async fn complete_with_retry(
        &self,
        model: ActiveModel,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
    ) -> Result<String> {
        for attempt in 1..=MAX_RETRIES {
            let result = match model.api_type {
                ApiType::Anthropic => {
                    anthropic::complete_anthropic(
                        &self.http,
                        model.clone(),
                        messages.clone(),
                        tools.clone(),
                        self.save_request_body,
                        self.max_request_files,
                    )
                    .await
                }
                ApiType::OpenAiChatCompletions => {
                    openai::complete_openai(
                        &self.http,
                        model.clone(),
                        messages.clone(),
                        tools.clone(),
                        self.save_request_body,
                        self.max_request_files,
                    )
                    .await
                }
                ApiType::OpenAiResponses => {
                    responses::complete_responses(
                        &self.http,
                        model.clone(),
                        messages.clone(),
                        tools.clone(),
                        self.save_request_body,
                        self.max_request_files,
                    )
                    .await
                }
            };

            match result {
                Ok(response) => return Ok(response),
                Err(e) => {
                    let network_error = classify_anyhow_error(e);
                    let is_last_attempt = attempt == MAX_RETRIES;

                    if !network_error.is_retryable() || is_last_attempt {
                        return Err(anyhow::anyhow!("{}", network_error.message()));
                    }

                    let delay_secs = backoff_delay(attempt).as_secs() as u32;

                    eprintln!(
                        "Completion failed (attempt {}/{}): {}, retrying in {}s...",
                        attempt,
                        MAX_RETRIES,
                        network_error.message(),
                        delay_secs
                    );

                    backoff_sleep(attempt).await;
                }
            }
        }

        unreachable!("loop should return before this point")
    }

    #[allow(clippy::too_many_arguments)]
    async fn stream_chat_inner(
        &self,
        session_id: Uuid,
        request_id: u64,
        model: ActiveModel,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<BackendEvent>,
        thinking_level: crate::config::reasoning::ThinkingLevelType,
    ) -> Result<()> {
        match model.api_type {
            ApiType::Anthropic => {
                anthropic::stream_anthropic(
                    &self.http, session_id, request_id, model, messages, tools, tx,
                    self.save_request_body,
                    self.max_request_files,
                )
                .await
            }
            ApiType::OpenAiChatCompletions => {
                openai::stream_openai(
                    &self.http,
                    session_id,
                    request_id,
                    model,
                    messages,
                    tools,
                    tx,
                    thinking_level,
                    self.save_request_body,
                    self.max_request_files,
                )
                .await
            }
            ApiType::OpenAiResponses => {
                responses::stream_responses(
                    &self.http, session_id, request_id, model, messages, tools, tx,
                    self.save_request_body,
                    self.max_request_files,
                )
                .await
            }
        }
    }
}
