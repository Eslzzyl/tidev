//! LLM provider implementations — the core LLM abstraction used by the
//! tidev agent loop and background tasks.
//!
//! This crate exposes [`LlmClient`] which routes requests to provider-specific
//! implementations (Anthropic, OpenAI Chat Completions, OpenAI Responses API,
//! Google Gemini) based on the [`ApiType`] carried by [`LlmProviderConfig`].

mod anthropic;
mod attachments;
mod debug;
mod error;
mod gemini;
mod openai;
mod responses;
mod think_parser;
mod tool_call_format;
mod turn;
mod types;

pub use types::{ApiType, LlmProviderConfig, ToolDefinition};

use anyhow::{Context, Result};
use reqwest::Client;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use tidev_types::message::{BackendEvent, Message};

use error::{MAX_RETRIES, backoff_delay, backoff_sleep, classify_anyhow_error};

/// Streaming LLM client.
///
/// Create via [`LlmClient::new`], then call [`stream_chat`](LlmClient::stream_chat)
/// or [`complete_with_messages`](LlmClient::complete_with_messages).
#[derive(Clone, Debug)]
pub struct LlmClient {
    http: Client,
    pub save_request_body: bool,
    pub max_request_files: usize,
    pub save_response_body: bool,
    pub max_response_files: usize,
}

impl LlmClient {
    /// Build a new client.  Only the debug knobs are needed — all other
    /// configuration comes per-request via [`LlmProviderConfig`].
    pub fn new(
        save_request_body: bool,
        max_request_files: usize,
        save_response_body: bool,
        max_response_files: usize,
    ) -> Result<Self> {
        let http = Client::builder()
            .user_agent("tidev/0.1")
            .timeout(Duration::from_secs(1800))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .context("failed to construct HTTP client")?;

        Ok(Self {
            http,
            save_request_body,
            max_request_files,
            save_response_body,
            max_response_files,
        })
    }

    /// Get a reference to the HTTP client for reuse.
    pub fn http(&self) -> &Client {
        &self.http
    }

    /// Stream a chat completion, forwarding [`BackendEvent`]s through `tx`.
    #[allow(clippy::too_many_arguments)]
    pub async fn stream_chat(
        &self,
        session_id: Uuid,
        request_id: u64,
        model: LlmProviderConfig,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<BackendEvent>,
        thinking_level: tidev_types::reasoning::ThinkingLevelType,
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

    /// Non-streaming completion — returns the full assistant text.
    pub async fn complete_with_messages(
        &self,
        session_id: Uuid,
        request_id: u64,
        model: LlmProviderConfig,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: Option<UnboundedSender<BackendEvent>>,
    ) -> Result<String> {
        let result = self
            .complete_with_retry(session_id, request_id, model, messages, tools, tx)
            .await;
        result.context("LLM completion failed after retries")
    }

    #[allow(clippy::too_many_arguments)]
    async fn stream_chat_with_retry(
        &self,
        session_id: Uuid,
        request_id: u64,
        model: LlmProviderConfig,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<BackendEvent>,
        thinking_level: tidev_types::reasoning::ThinkingLevelType,
    ) -> Result<()> {
        // Determine how many retries we can afford.
        let max = MAX_RETRIES;

        for attempt in 0..=max {
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
                    let network_err = classify_anyhow_error(e);

                    if !network_err.is_retryable() {
                        return Err(anyhow::anyhow!("{}", network_err.message()));
                    }

                    let delay = backoff_delay(attempt + 1);
                    let _ = tx.send(BackendEvent::Retrying {
                        session_id,
                        request_id,
                        attempt: attempt + 1,
                        max_attempts: max + 1,
                        reason: network_err.message().to_string(),
                        retry_after_secs: Some(delay.as_secs() as u32),
                    });

                    if attempt == max {
                        return Err(anyhow::anyhow!("{}", network_err.message()));
                    }

                    backoff_sleep(attempt + 1).await;
                }
            }
        }

        unreachable!()
    }

    async fn complete_with_retry(
        &self,
        session_id: Uuid,
        request_id: u64,
        model: LlmProviderConfig,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: Option<UnboundedSender<BackendEvent>>,
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
                        self.save_response_body,
                        self.max_response_files,
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
                        self.save_response_body,
                        self.max_response_files,
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
                        self.save_response_body,
                        self.max_response_files,
                    )
                    .await
                }
                ApiType::GoogleGemini => {
                    gemini::complete_gemini(
                        &self.http,
                        model.clone(),
                        messages.clone(),
                        tools.clone(),
                        self.save_request_body,
                        self.max_request_files,
                        self.save_response_body,
                        self.max_response_files,
                    )
                    .await
                }
            };

            match result {
                Ok(response) => return Ok(response),
                Err(e) => {
                    let network_error = classify_anyhow_error(e);

                    if !network_error.is_retryable() {
                        return Err(anyhow::anyhow!("{}", network_error.message()));
                    }

                    let delay_secs = backoff_delay(attempt).as_secs() as u32;

                    if let Some(tx) = &tx {
                        let _ = tx.send(BackendEvent::Retrying {
                            session_id,
                            request_id,
                            attempt: attempt as u32,
                            max_attempts: MAX_RETRIES,
                            reason: network_error.message().to_string(),
                            retry_after_secs: Some(delay_secs),
                        });
                    }

                    if attempt == MAX_RETRIES {
                        return Err(anyhow::anyhow!("{}", network_error.message()));
                    }

                    backoff_sleep(attempt).await;
                }
            }
        }

        // If we exhaust all retries without returning, something is wrong.
        unreachable!()
    }

    #[allow(clippy::too_many_arguments)]
    async fn stream_chat_inner(
        &self,
        session_id: Uuid,
        request_id: u64,
        model: LlmProviderConfig,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<BackendEvent>,
        thinking_level: tidev_types::reasoning::ThinkingLevelType,
    ) -> Result<()> {
        match model.api_type {
            ApiType::Anthropic => {
                anthropic::stream_anthropic(
                    &self.http,
                    session_id,
                    request_id,
                    model,
                    messages,
                    tools,
                    tx,
                    self.save_request_body,
                    self.max_request_files,
                    self.save_response_body,
                    self.max_response_files,
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
                    self.save_response_body,
                    self.max_response_files,
                )
                .await
            }
            ApiType::OpenAiResponses => {
                responses::stream_responses(
                    &self.http,
                    session_id,
                    request_id,
                    model,
                    messages,
                    tools,
                    tx,
                    self.save_request_body,
                    self.max_request_files,
                    self.save_response_body,
                    self.max_response_files,
                )
                .await
            }
            ApiType::GoogleGemini => {
                gemini::stream_gemini(
                    &self.http,
                    session_id,
                    request_id,
                    model,
                    messages,
                    tools,
                    tx,
                    self.save_request_body,
                    self.max_request_files,
                    self.save_response_body,
                    self.max_response_files,
                )
                .await
            }
        }
    }
}
