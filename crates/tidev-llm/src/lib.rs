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
pub use message::{COMPACTION_CONTINUATION_PREFIX, extract_compaction_summary, short_session_id};
pub use types::{ApiType, LlmCompletion, LlmProviderConfig, LlmRequestContext, ToolDefinition};

use anyhow::{Context, Result};
use reqwest::{
    Client, RequestBuilder,
    header::{HeaderName, HeaderValue, USER_AGENT},
};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::message::Message;

use error::{MAX_RETRIES, backoff_delay, backoff_sleep, classify_anyhow_error};

/// Authentication material resolved immediately before an LLM request.
#[derive(Clone)]
pub struct RequestAuth {
    pub bearer_token: String,
    pub headers: BTreeMap<String, String>,
}

impl std::fmt::Debug for RequestAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RequestAuth")
            .field("bearer_token", &"<redacted>")
            .field("headers", &self.headers)
            .finish()
    }
}

/// Resolve dynamic provider authentication without coupling the protocol crate
/// to tidev's configuration or persistence crates.
pub trait RequestAuthResolver: Send + Sync + std::fmt::Debug {
    fn resolve<'a>(
        &'a self,
        provider_id: &'a str,
        force_refresh: bool,
    ) -> Pin<Box<dyn Future<Output = Result<RequestAuth>> + Send + 'a>>;
}

#[derive(Clone, Debug)]
pub struct LlmDebugConfig {
    pub save_request_body: bool,
    pub max_request_files: usize,
    pub save_response_body: bool,
    pub max_response_files: usize,
}

/// Ensures Reqwest can build Rustls clients with the Ring provider.
fn ensure_rustls_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

/// Apply provider-specific request headers while retaining the client defaults otherwise.
pub(crate) fn apply_request_headers(
    request: RequestBuilder,
    model: &LlmProviderConfig,
) -> Result<RequestBuilder> {
    let request = if let Some(user_agent) = model.user_agent.as_deref() {
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
        request.header(USER_AGENT, value)
    } else {
        request
    };

    model
        .headers
        .iter()
        .try_fold(request, |request, (name, value)| {
            let header_name = HeaderName::from_bytes(name.as_bytes()).with_context(|| {
                format!(
                    "invalid header name '{name}' for provider '{}'",
                    model.provider_id
                )
            })?;
            let header_value = HeaderValue::from_str(value).with_context(|| {
                format!(
                    "invalid value for header '{name}' for provider '{}'",
                    model.provider_id
                )
            })?;
            Ok(request.header(header_name, header_value))
        })
}

pub(crate) async fn resolve_request_auth(
    model: &LlmProviderConfig,
    resolver: Option<&dyn RequestAuthResolver>,
    force_refresh: bool,
) -> Result<RequestAuth> {
    if model.api_type == ApiType::OpenAiCodexResponses {
        let resolver = resolver.with_context(|| {
            format!(
                "missing request auth resolver for Codex provider '{}'",
                model.provider_id
            )
        })?;
        return resolver.resolve(&model.provider_id, force_refresh).await;
    }

    let bearer_token = model
        .api_key
        .clone()
        .with_context(|| format!("missing API key for provider '{}'", model.provider_id))?;
    Ok(RequestAuth {
        bearer_token,
        headers: BTreeMap::new(),
    })
}

pub(crate) fn apply_request_auth(
    request: RequestBuilder,
    model: &LlmProviderConfig,
    auth: &RequestAuth,
) -> Result<RequestBuilder> {
    let request = request.bearer_auth(&auth.bearer_token);
    let request = auth.headers.iter().try_fold(
        request,
        |request, (name, value)| -> Result<RequestBuilder> {
            let header_name = HeaderName::from_bytes(name.as_bytes())
                .with_context(|| format!("invalid auth header name '{name}'"))?;
            let header_value = HeaderValue::from_str(value)
                .with_context(|| format!("invalid auth header value '{name}'"))?;
            Ok(request.header(header_name, header_value))
        },
    )?;

    if model.api_type == ApiType::OpenAiCodexResponses {
        Ok(request
            .header("originator", "tidev")
            .header("OpenAI-Beta", "responses=experimental")
            .header("Accept", "text/event-stream")
            .header("Content-Type", "application/json"))
    } else {
        Ok(request)
    }
}

fn prepare_model_for_request(
    mut model: LlmProviderConfig,
    context: LlmRequestContext,
) -> Result<LlmProviderConfig> {
    if let Some(session_header) = model.session_header.clone() {
        let session_id = context
            .session_id
            .with_context(|| {
                format!(
                    "provider '{}' requires a session id for header '{session_header}'",
                    model.provider_id
                )
            })?
            .to_string();
        model
            .headers
            .retain(|name, _| !name.eq_ignore_ascii_case(&session_header));
        model.headers.insert(session_header, session_id);
    }

    if model.api_type == ApiType::OpenAiCodexResponses {
        model.headers.retain(|name, _| {
            !name.eq_ignore_ascii_case("session-id")
                && !name.eq_ignore_ascii_case("x-client-request-id")
        });
        if let Some(session_id) = context.session_id {
            let session_id = session_id.to_string();
            model
                .headers
                .insert("session-id".to_string(), session_id.clone());
            model
                .headers
                .insert("x-client-request-id".to_string(), session_id);
        }
    }

    Ok(model)
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
    auth_resolver: Option<Arc<dyn RequestAuthResolver>>,
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
        Self::new_with_user_agent_and_auth_resolver(
            save_request_body,
            max_request_files,
            save_response_body,
            max_response_files,
            None,
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
        Self::new_with_user_agent_and_auth_resolver(
            save_request_body,
            max_request_files,
            save_response_body,
            max_response_files,
            user_agent,
            None,
        )
    }

    pub fn new_with_user_agent_and_auth_resolver(
        save_request_body: bool,
        max_request_files: usize,
        save_response_body: bool,
        max_response_files: usize,
        user_agent: Option<&str>,
        auth_resolver: Option<Arc<dyn RequestAuthResolver>>,
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
            auth_resolver,
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
        self.stream_chat_with_context(
            model,
            messages,
            tools,
            tx,
            thinking_level,
            LlmRequestContext::default(),
        )
        .await;
    }

    /// Stream a chat completion with per-request context.
    #[allow(clippy::too_many_arguments)]
    pub async fn stream_chat_with_context(
        &self,
        model: LlmProviderConfig,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: UnboundedSender<LlmEvent>,
        thinking_level: crate::reasoning::ThinkingLevelType,
        context: LlmRequestContext,
    ) {
        let result = match prepare_model_for_request(model, context) {
            Ok(model) => {
                self.stream_chat_with_retry(model, messages, tools, tx.clone(), thinking_level)
                    .await
            }
            Err(error) => Err(error::NetworkError::NonRetryable {
                message: error.to_string(),
            }),
        };

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
        self.complete_with_messages_with_context(
            model,
            messages,
            tools,
            tx,
            LlmRequestContext::default(),
        )
        .await
    }

    /// Non-streaming completion with per-request context.
    pub async fn complete_with_messages_with_context(
        &self,
        model: LlmProviderConfig,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: Option<UnboundedSender<LlmEvent>>,
        context: LlmRequestContext,
    ) -> Result<String> {
        Ok(self
            .complete_with_messages_with_context_result(model, messages, tools, tx, context)
            .await?
            .content)
    }

    /// Non-streaming completion with structured tool-call metadata.
    pub async fn complete_with_messages_with_context_result(
        &self,
        model: LlmProviderConfig,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        tx: Option<UnboundedSender<LlmEvent>>,
        context: LlmRequestContext,
    ) -> Result<LlmCompletion> {
        let model = prepare_model_for_request(model, context)?;
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
        if model.api_type == ApiType::OpenAiCodexResponses {
            return self
                .stream_chat_inner(model, messages, tools, tx, thinking_level)
                .await
                .map_err(classify_anyhow_error);
        }

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
    ) -> Result<LlmCompletion> {
        if model.api_type == ApiType::OpenAiCodexResponses {
            let debug = self.debug_config();
            return responses::complete_responses(
                &self.http,
                model,
                messages,
                tools,
                tx.as_ref(),
                debug.save_request_body,
                debug.max_request_files,
                debug.save_response_body,
                debug.max_response_files,
                self.auth_resolver.as_deref(),
            )
            .await;
        }

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
                ApiType::OpenAiResponses | ApiType::OpenAiCodexResponses => {
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
                        self.auth_resolver.as_deref(),
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
            ApiType::OpenAiResponses | ApiType::OpenAiCodexResponses => {
                responses::stream_responses(
                    &self.http,
                    model,
                    messages,
                    tools,
                    thinking_level,
                    tx,
                    debug.save_request_body,
                    debug.max_request_files,
                    debug.save_response_body,
                    debug.max_response_files,
                    self.auth_resolver.as_deref(),
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

#[cfg(test)]
mod tests {
    use super::{
        ApiType, LlmClient, LlmDebugConfig, LlmProviderConfig, RequestAuth, RequestAuthResolver,
    };
    use crate::message::{Message, MessageRole};
    use crate::reasoning::ThinkingLevelType;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::io;
    use std::pin::Pin;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

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
    #[derive(Debug)]
    struct TestResolver {
        force_refresh: Arc<Mutex<Vec<bool>>>,
    }

    impl RequestAuthResolver for TestResolver {
        fn resolve<'a>(
            &'a self,
            _provider_id: &'a str,
            force_refresh: bool,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<RequestAuth>> + Send + 'a>> {
            let force_refreshes = self.force_refresh.clone();
            Box::pin(async move {
                force_refreshes.lock().await.push(force_refresh);
                Ok(RequestAuth {
                    bearer_token: if force_refresh {
                        "refreshed-access-token".into()
                    } else {
                        "initial-access-token".into()
                    },
                    headers: BTreeMap::new(),
                })
            })
        }
    }

    #[tokio::test]
    async fn codex_retries_401_with_the_exact_same_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for index in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (headers, body) = read_http_request(&mut stream).await.unwrap();
                requests.push((headers, body));
                let response = if index == 0 {
                    "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                } else {
                    let body = r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"ok"}]}]}"#;
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                };
                stream.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });

        let force_refresh = Arc::new(Mutex::new(Vec::new()));
        let resolver = Arc::new(TestResolver {
            force_refresh: force_refresh.clone(),
        });
        let client = LlmClient::new_with_user_agent_and_auth_resolver(
            false,
            1,
            false,
            1,
            None,
            Some(resolver),
        )
        .unwrap();
        let model = LlmProviderConfig {
            provider_id: "openai-codex".into(),
            api_type: ApiType::OpenAiCodexResponses,
            api_key: None,
            base_url: format!("http://{address}/codex"),
            user_agent: None,
            headers: BTreeMap::new(),
            session_header: None,
            model_id: "gpt-5.5".into(),
            request_model_id: Some("gpt-5.5".into()),
            system_prompt: None,
            thinking_level: ThinkingLevelType::None,
            extra_body: None,
            max_output_tokens: 256,
            context_window: 128_000,
            temperature: None,
            supports_images: false,
            supports_parallel_tool_calls: true,
        };

        let result = client
            .complete_with_messages(
                model,
                vec![Message::new(MessageRole::User, "same request")],
                Vec::new(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(result, "ok");
        assert_eq!(*force_refresh.lock().await, vec![false, true]);

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].1, requests[1].1);
        assert!(requests[0].0.starts_with("POST /codex/responses HTTP/1.1"));
        let first_headers = requests[0].0.to_ascii_lowercase();
        let second_headers = requests[1].0.to_ascii_lowercase();
        assert!(first_headers.contains("authorization: bearer initial-access-token"));
        assert!(second_headers.contains("authorization: bearer refreshed-access-token"));
        assert!(first_headers.contains("originator: tidev"));
        assert!(first_headers.contains("openai-beta: responses=experimental"));
    }

    #[tokio::test]
    async fn codex_does_not_retry_after_partial_stream_output() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_http_request(&mut stream).await.unwrap();
            let body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let resolver = Arc::new(TestResolver {
            force_refresh: Arc::new(Mutex::new(Vec::new())),
        });
        let client = LlmClient::new_with_user_agent_and_auth_resolver(
            false,
            1,
            false,
            1,
            None,
            Some(resolver),
        )
        .unwrap();
        let model = LlmProviderConfig {
            provider_id: "openai-codex".into(),
            api_type: ApiType::OpenAiCodexResponses,
            api_key: None,
            base_url: format!("http://{address}/codex"),
            user_agent: None,
            headers: BTreeMap::new(),
            session_header: None,
            model_id: "gpt-5.5".into(),
            request_model_id: Some("gpt-5.5".into()),
            system_prompt: None,
            thinking_level: ThinkingLevelType::None,
            extra_body: None,
            max_output_tokens: 256,
            context_window: 128_000,
            temperature: None,
            supports_images: false,
            supports_parallel_tool_calls: true,
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        client
            .stream_chat(
                model,
                vec![Message::new(MessageRole::User, "partial")],
                Vec::new(),
                tx,
                ThinkingLevelType::None,
            )
            .await;

        let mut saw_delta = false;
        let mut saw_non_retryable_failure = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                super::event::LlmEvent::Delta { content } if content == "partial" => {
                    saw_delta = true;
                }
                super::event::LlmEvent::Failed { retryable, .. } => {
                    saw_non_retryable_failure = !retryable;
                }
                _ => {}
            }
        }
        assert!(saw_delta);
        assert!(saw_non_retryable_failure);
        server.await.unwrap();
    }

    async fn read_http_request(
        stream: &mut tokio::net::TcpStream,
    ) -> io::Result<(String, Vec<u8>)> {
        let mut request = Vec::new();
        let header_end;
        loop {
            let mut chunk = [0_u8; 4096];
            let count = stream.read(&mut chunk).await?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "request closed before headers",
                ));
            }
            request.extend_from_slice(&chunk[..count]);
            if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                header_end = index + 4;
                break;
            }
        }
        let headers = String::from_utf8_lossy(&request[..header_end]).into_owned();
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length: ")
                    .or_else(|| line.strip_prefix("content-length: "))
            })
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 4096];
            let count = stream.read(&mut chunk).await?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "request closed before body",
                ));
            }
            request.extend_from_slice(&chunk[..count]);
        }
        Ok((
            headers,
            request[header_end..header_end + content_length].to_vec(),
        ))
    }
}
