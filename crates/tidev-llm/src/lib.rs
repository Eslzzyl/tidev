//! LLM provider implementations — the core LLM abstraction used by the
//! tidev agent loop and background tasks.
//!
//! This crate exposes [`LlmClient`] which routes requests to provider-specific
//! implementations (Anthropic, OpenAI Chat Completions, OpenAI Responses API,
//! Google Gemini) based on the [`ApiType`] carried by [`LlmProviderConfig`].

mod anthropic;
mod attachments;
mod debug;
pub mod error;
pub mod event;
mod gemini;
pub mod message;
mod openai;
pub mod reasoning;
mod responses;
mod think_parser;
mod tool_call_format;
mod turn;
mod types;

pub use event::LlmEvent;
pub use types::{ApiType, LlmProviderConfig, ToolDefinition};

use anyhow::{Context, Result};
use reqwest::{
    Client, RequestBuilder,
    header::{HeaderValue, USER_AGENT},
};
use std::{
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Debug)]
pub struct LlmDebugConfig {
    pub save_request_body: bool,
    pub max_request_files: usize,
    pub save_response_body: bool,
    pub max_response_files: usize,
}

use crate::message::Message;

use error::{MAX_RETRIES, backoff_delay, backoff_sleep, classify_anyhow_error};

/// Ensures Reqwest can build Rustls clients with the Ring provider.
fn ensure_rustls_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

/// Apply a provider-specific User-Agent override while retaining the client default otherwise.
pub(crate) fn apply_user_agent(
    request: RequestBuilder,
    model: &LlmProviderConfig,
) -> Result<RequestBuilder> {
    let Some(user_agent) = model.user_agent.as_deref() else {
        return Ok(request);
    };

    if user_agent.trim().is_empty() {
        anyhow::bail!(
            "user_agent for provider '{}' cannot be empty",
            model.provider_id
        );
    }

    let value = HeaderValue::from_str(user_agent).with_context(|| {
        format!(
            "invalid user_agent for provider '{}': contains invalid header characters",
            model.provider_id
        )
    })?;
    Ok(request.header(USER_AGENT, value))
}

/// Streaming LLM client.
///
/// Create via [`LlmClient::new`] or [`LlmClient::new_with_user_agent`], then call
/// [`stream_chat`](LlmClient::stream_chat) or
/// [`complete_with_messages`](LlmClient::complete_with_messages).
#[derive(Clone, Debug)]
pub struct LlmClient {
    http: Client,
    debug_config: Arc<RwLock<LlmDebugConfig>>,
}

#[cfg(test)]
mod tests {
    use super::{LlmClient, LlmDebugConfig};

    #[test]
    fn cloned_clients_share_debug_configuration() {
        let client = LlmClient::new(false, 1, false, 1).expect("client should build");
        let clone = client.clone();

        clone.update_debug_config(LlmDebugConfig {
            save_request_body: true,
            max_request_files: 9,
            save_response_body: true,
            max_response_files: 7,
        });

        let current = client.debug_config();
        assert!(current.save_request_body);
        assert_eq!(current.max_request_files, 9);
        assert!(current.save_response_body);
        assert_eq!(current.max_response_files, 7);
    }
}

impl LlmClient {
    /// Build a new client without an application-specific default User-Agent.
    ///
    /// Applications that identify themselves in outbound requests should use
    /// [`LlmClient::new_with_user_agent`]. Provider-specific overrides from
    /// [`LlmProviderConfig`] still apply per request.
    pub fn new(
        save_request_body: bool,
        max_request_files: usize,
        save_response_body: bool,
        max_response_files: usize,
    ) -> Result<Self> {
        Self::new_with_user_agent(
            save_request_body,
            max_request_files,
            save_response_body,
            max_response_files,
            None,
        )
    }

    /// Build a client with an application-specific default User-Agent.
    ///
    /// A provider-level `user_agent` in [`LlmProviderConfig`] overrides this
    /// value for the individual request.
    pub fn new_with_user_agent(
        save_request_body: bool,
        max_request_files: usize,
        save_response_body: bool,
        max_response_files: usize,
        user_agent: Option<&str>,
    ) -> Result<Self> {
        ensure_rustls_crypto_provider();
        let mut builder = Client::builder();
        if let Some(user_agent) = user_agent {
            builder = builder.user_agent(user_agent);
        }
        let http = builder
            .timeout(Duration::from_secs(1800))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .context("failed to construct HTTP client")?;

        Ok(Self {
            http,
            debug_config: Arc::new(RwLock::new(LlmDebugConfig {
                save_request_body,
                max_request_files,
                save_response_body,
                max_response_files,
            })),
        })
    }

    pub fn update_debug_config(&self, config: LlmDebugConfig) {
        if let Ok(mut current) = self.debug_config.write() {
            *current = config;
        }
    }

    fn debug_config(&self) -> LlmDebugConfig {
        self.debug_config
            .read()
            .map(|config| config.clone())
            .unwrap_or_else(|_| LlmDebugConfig {
                save_request_body: false,
                max_request_files: 0,
                save_response_body: false,
                max_response_files: 0,
            })
    }

    /// Get a reference to the HTTP client for reuse.
    pub fn http(&self) -> &Client {
        &self.http
    }

    /// Stream a chat completion, forwarding [`LlmEvent`]s through `tx`.
    #[allow(clippy::too_many_arguments)]
    pub async fn stream_chat(
        &self,
        model: LlmProviderConfig,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<LlmEvent>,
        thinking_level: crate::reasoning::ThinkingLevelType,
    ) {
        let result = self
            .stream_chat_with_retry(model, messages, tools, tx.clone(), thinking_level)
            .await;

        if let Err(error) = result {
            let _ = tx.send(LlmEvent::Failed {
                error: error.message().to_string(),
                retryable: error.is_retryable(),
            });
        }
    }

    /// Non-streaming completion — returns the full assistant text.
    pub async fn complete_with_messages(
        &self,
        model: LlmProviderConfig,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: Option<UnboundedSender<LlmEvent>>,
    ) -> Result<String> {
        let result = self.complete_with_retry(model, messages, tools, tx).await;
        result.context("LLM completion failed after retries")
    }

    #[allow(clippy::too_many_arguments)]
    async fn stream_chat_with_retry(
        &self,
        model: LlmProviderConfig,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<LlmEvent>,
        thinking_level: crate::reasoning::ThinkingLevelType,
    ) -> std::result::Result<(), error::NetworkError> {
        // Determine how many retries we can afford.
        let max = MAX_RETRIES;

        for attempt in 0..=max {
            let result = self
                .stream_chat_inner(
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
                        return Err(network_err);
                    }

                    let delay = backoff_delay(attempt + 1);
                    let _ = tx.send(LlmEvent::Retrying {
                        attempt: attempt + 1,
                        max_attempts: max + 1,
                        reason: network_err.message().to_string(),
                        retry_after_secs: Some(delay.as_secs() as u32),
                    });

                    if attempt == max {
                        return Err(network_err);
                    }

                    backoff_sleep(attempt + 1).await;
                }
            }
        }

        unreachable!()
    }

    async fn complete_with_retry(
        &self,
        model: LlmProviderConfig,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: Option<UnboundedSender<LlmEvent>>,
    ) -> Result<String> {
        let debug = self.debug_config();
        for attempt in 1..=MAX_RETRIES {
            let result = match model.api_type {
                ApiType::Anthropic => {
                    anthropic::complete_anthropic(
                        &self.http,
                        model.clone(),
                        messages.clone(),
                        tools.clone(),
                        debug.save_request_body,
                        debug.max_request_files,
                        debug.save_response_body,
                        debug.max_response_files,
                    )
                    .await
                }
                ApiType::OpenAiChatCompletions => {
                    openai::complete_openai(
                        &self.http,
                        model.clone(),
                        messages.clone(),
                        tools.clone(),
                        debug.save_request_body,
                        debug.max_request_files,
                        debug.save_response_body,
                        debug.max_response_files,
                    )
                    .await
                }
                ApiType::OpenAiResponses => {
                    responses::complete_responses(
                        &self.http,
                        model.clone(),
                        messages.clone(),
                        tools.clone(),
                        tx.as_ref(),
                        debug.save_request_body,
                        debug.max_request_files,
                        debug.save_response_body,
                        debug.max_response_files,
                    )
                    .await
                }
                ApiType::GoogleGemini => {
                    gemini::complete_gemini(
                        &self.http,
                        model.clone(),
                        messages.clone(),
                        tools.clone(),
                        debug.save_request_body,
                        debug.max_request_files,
                        debug.save_response_body,
                        debug.max_response_files,
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
                        let _ = tx.send(LlmEvent::Retrying {
                            attempt,
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
        model: LlmProviderConfig,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<LlmEvent>,
        thinking_level: crate::reasoning::ThinkingLevelType,
    ) -> Result<()> {
        let debug = self.debug_config();
        match model.api_type {
            ApiType::Anthropic => {
                anthropic::stream_anthropic(
                    &self.http,
                    model,
                    messages,
                    tools,
                    tx,
                    debug.save_request_body,
                    debug.max_request_files,
                    debug.save_response_body,
                    debug.max_response_files,
                )
                .await
            }
            ApiType::OpenAiChatCompletions => {
                openai::stream_openai(
                    &self.http,
                    model,
                    messages,
                    tools,
                    tx,
                    thinking_level,
                    debug.save_request_body,
                    debug.max_request_files,
                    debug.save_response_body,
                    debug.max_response_files,
                )
                .await
            }
            ApiType::OpenAiResponses => {
                responses::stream_responses(
                    &self.http,
                    model,
                    messages,
                    tools,
                    tx,
                    debug.save_request_body,
                    debug.max_request_files,
                    debug.save_response_body,
                    debug.max_response_files,
                )
                .await
            }
            ApiType::GoogleGemini => {
                gemini::stream_gemini(
                    &self.http,
                    model,
                    messages,
                    tools,
                    tx,
                    debug.save_request_body,
                    debug.max_request_files,
                    debug.save_response_body,
                    debug.max_response_files,
                )
                .await
            }
        }
    }
}
