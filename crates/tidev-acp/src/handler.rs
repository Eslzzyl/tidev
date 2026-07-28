//! ACP agent entry point.
//!
//! Wires up the ACP SDK's [`Agent`] builder with request handlers for each
//! protocol method, spawns the event translator and permission bridge tasks,
//! and runs the connection over stdio.

use std::sync::Arc;

use agent_client_protocol::schema::v1 as acp;
use agent_client_protocol::{Agent, Stdio, on_receive_request};
use anyhow::Result;
use tokio::sync::{oneshot, RwLock};
use uuid::Uuid;

use tidev_core::Runtime;
use tidev_types::message::MessageRole;
use tidev_types::prompts::SessionMode;
use tidev_utils::session::title_from_prompt;
use tidev_config::{auth::ActiveModel, AppConfig, AuthStore, ThinkingMatcher};

/// Shared state accessible from all ACP request handlers.
struct AcpState {
    runtime: Runtime,
    /// The active ACP session ID, if any.
    active_session: RwLock<Option<Uuid>>,
    /// The event translator for the active session.
    translator: RwLock<Option<crate::event_translator::EventTranslator>>,
    /// Oneshot sender for the pending `session/prompt` response.
    /// When set, the event loop will send the `PromptResponse` through this
    /// channel once the LLM turn completes (or fails).
    pending_prompt: RwLock<Option<oneshot::Sender<acp::PromptResponse>>>,
    /// Whether the session title has been set from the first prompt.
    session_named: RwLock<bool>,
    /// Current session mode (Plan or Build).
    current_mode: RwLock<SessionMode>,
    /// Cached config options for the active session.
    config_options: RwLock<Vec<acp::SessionConfigOption>>,
    /// Cumulative input tokens across all turns (for PromptResponse.usage).
    cumulative_input: RwLock<u64>,
    /// Cumulative output tokens across all turns.
    cumulative_output: RwLock<u64>,
    /// Cumulative cache read tokens across all turns.
    cumulative_cache_read: RwLock<u64>,
}

// ---------------------------------------------------------------------------
// Config option helpers
// ---------------------------------------------------------------------------

/// Build the full set of session config options from runtime state.
fn build_config_options(runtime: &Runtime) -> Vec<acp::SessionConfigOption> {
    let config = runtime.config();
    let auth = runtime.auth();
    let active = runtime.active_model();
    vec![
        build_model_config_option(&config, &auth, &active),
        build_thought_level_config_option(&active),
    ]
}

/// Build the "model" config option listing all connected models.
fn build_model_config_option(
    config: &AppConfig,
    auth: &AuthStore,
    active: &ActiveModel,
) -> acp::SessionConfigOption {
    let connected = config.connected_models(auth);
    let options: Vec<acp::SessionConfigSelectOption> = connected
        .iter()
        .map(|m| {
            let val = format!("{}/{}", m.provider_id, m.model_id);
            let display = format!("{} ({})", m.model_display_name, m.provider_display_name);
            acp::SessionConfigSelectOption::new(val, display)
        })
        .collect();

    let current = format!("{}/{}", active.provider_id, active.model_id);

    acp::SessionConfigOption::select("model", "Model", current, options)
        .category(acp::SessionConfigOptionCategory::Model)
}

/// Build the "thought_level" config option based on the active model.
fn build_thought_level_config_option(active: &ActiveModel) -> acp::SessionConfigOption {
    let supported = ThinkingMatcher::supported_levels(&active.model_id);

    let options: Vec<acp::SessionConfigSelectOption> = supported
        .iter()
        .map(|tl| acp::SessionConfigSelectOption::new(tl.to_string(), tl.display_name()))
        .collect();

    let current = active.thinking_level.to_string();

    acp::SessionConfigOption::select("thought_level", "Thinking Level", current, options)
        .description("Controls how much reasoning effort the model applies")
        .category(acp::SessionConfigOptionCategory::ThoughtLevel)
}

/// Run tidev as an ACP agent over stdio.
///
/// This is the main entry point called from the `tidev acp` CLI subcommand.
pub async fn run_acp_agent() -> Result<()> {
    let runtime = Runtime::builder()
        .workspace_root(std::env::current_dir()?)
        .build()
        .await?;

    let event_rx = runtime
        .event_rx()
        .await
        .ok_or_else(|| anyhow::anyhow!("event_rx already taken"))?;
    let request_rx = runtime
        .request_rx()
        .await
        .ok_or_else(|| anyhow::anyhow!("request_rx already taken"))?;

    let state = Arc::new(AcpState {
        runtime,
        active_session: RwLock::new(None),
        translator: RwLock::new(None),
        pending_prompt: RwLock::new(None),
        session_named: RwLock::new(false),
        current_mode: RwLock::new(SessionMode::Build),
        config_options: RwLock::new(Vec::new()),
        cumulative_input: RwLock::new(0),
        cumulative_output: RwLock::new(0),
        cumulative_cache_read: RwLock::new(0),
    });

    Agent.builder()
        .name("tidev")
        // ── initialize ──────────────────────────────────────────────
        .on_receive_request(
            {
                let state = &state;
                move |_req: acp::InitializeRequest,
                      responder: agent_client_protocol::Responder<acp::InitializeResponse>,
                      _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let _state = state.clone();
                    async move {
                        log::info!("ACP: client sent initialize request");
                        let response = acp::InitializeResponse::new(
                            agent_client_protocol::schema::ProtocolVersion::V1,
                        )
                        .agent_info(acp::Implementation::new(
                            env!("CARGO_PKG_NAME"),
                            env!("CARGO_PKG_VERSION"),
                        ))
                        .agent_capabilities(
                            acp::AgentCapabilities::new()
                                .load_session(true)
                                .prompt_capabilities(
                                    acp::PromptCapabilities::new().image(true),
                                )
                                .mcp_capabilities(
                                    acp::McpCapabilities::new().http(true).sse(true),
                                )
                                .session_capabilities(
                                    acp::SessionCapabilities::new()
                                        .list(acp::SessionListCapabilities::new()),
                                ),
                        );
                        let _ = responder.respond(response);
                        Ok(agent_client_protocol::Handled::Yes)
                    }
                }
            },
            on_receive_request!(),
        )
        // ── session/new ─────────────────────────────────────────────
        .on_receive_request(
            {
                let state = &state;
                move |req: acp::NewSessionRequest,
                      responder: agent_client_protocol::Responder<acp::NewSessionResponse>,
                      _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let state = state.clone();
                    async move {
                        let cwd = &req.cwd;
                        log::info!("ACP: session/new, cwd={}", cwd.display());

                        // Merge MCP servers provided by the client.
                        for mcp_server in &req.mcp_servers {
                            let (name, config) = match acp_mcp_server_to_config(mcp_server) {
                                Some(pair) => pair,
                                None => continue,
                            };
                            log::info!("ACP: adding MCP server from client: {name}");
                            if let Err(e) = state
                                .runtime
                                .mcp_manager()
                                .upsert_server(name.clone(), config)
                                .await
                            {
                                log::warn!("ACP: failed to add MCP server '{name}': {e}");
                            }
                        }

                        let session_title = format!("ACP session — {}", cwd.display());
                        let session_id = state
                            .runtime
                            .create_default_session(&session_title)
                            .map_err(|e| {
                                agent_client_protocol::Error::internal_error()
                                    .data(format!("failed to create session: {e}"))
                            })?;

                        log::info!("ACP: created session {session_id}");

                        let context_window = state.runtime.active_model().context_window;
                        let translator =
                            crate::event_translator::EventTranslator::new(session_id, context_window);
                        *state.translator.write().await = Some(translator);
                        *state.active_session.write().await = Some(session_id);
                        *state.cumulative_input.write().await = 0;
                        *state.cumulative_output.write().await = 0;
                        *state.cumulative_cache_read.write().await = 0;
                        log::info!("ACP: translator set for session {session_id}");

                        // Reset mode to default for the new session.
                        *state.current_mode.write().await = SessionMode::Build;

                        let mode_state = acp::SessionModeState::new(
                            acp::SessionModeId::new("build"),
                            vec![
                                acp::SessionMode::new("plan", "Plan")
                                    .description(
                                        "Analyze and plan before making changes",
                                    ),
                                acp::SessionMode::new("build", "Build")
                                    .description(
                                        "Write and modify code with full tool access",
                                    ),
                            ],
                        );
                        let response = acp::NewSessionResponse::new(session_id.to_string())
                            .modes(mode_state);

                        // Cache and attach config options.
                        let config_opts = build_config_options(&state.runtime);
                        *state.config_options.write().await = config_opts.clone();
                        let response = response.config_options(config_opts);

                        let _ = responder.respond(response);
                        Ok(agent_client_protocol::Handled::Yes)
                    }
                }
            },
            on_receive_request!(),
        )
        // ── session/load ──────────────────────────────────────────────
        .on_receive_request(
            {
                let state = &state;
                move |req: acp::LoadSessionRequest,
                      responder: agent_client_protocol::Responder<acp::LoadSessionResponse>,
                      _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let state = state.clone();
                    async move {
                        log::info!(
                            "ACP: session/load, session_id={}, cwd={}",
                            req.session_id,
                            req.cwd.display(),
                        );

                        let session_id =
                            match Uuid::parse_str(req.session_id.to_string().as_str()) {
                                Ok(id) => id,
                                Err(e) => {
                                    return Err(
                                        agent_client_protocol::Error::invalid_request()
                                            .data(format!("invalid session ID: {e}")),
                                    )
                                }
                            };

                        // Load session record from DB.
                        let session = state
                            .runtime
                            .session_manager()
                            .load_session(session_id)
                            .map_err(|e| {
                                agent_client_protocol::Error::internal_error()
                                    .data(format!("failed to load session: {e}"))
                            })?
                            .ok_or_else(|| {
                                agent_client_protocol::Error::invalid_request()
                                    .data(format!("session not found: {session_id}"))
                            })?;

                        // Warn if the session's workspace root differs from the
                        // request cwd — we'll still use the session's own root.
                        let session_workspace = &session.workspace_root;
                        let req_cwd = req.cwd.to_string_lossy();
                        if session_workspace != req_cwd.as_ref() {
                            log::warn!(
                                "ACP: session workspace '{session_workspace}' \
                                 differs from request cwd '{req_cwd}', \
                                 using session workspace",
                            );
                        }

                        // Load messages and populate the in-memory buffer.
                        let messages = state
                            .runtime
                            .session_manager()
                            .load_messages(session_id)
                            .map_err(|e| {
                                agent_client_protocol::Error::internal_error()
                                    .data(format!("failed to load messages: {e}"))
                            })?;

                        // Compute cumulative token usage from stored messages
                        // (same as TUI sidebar "Total").
                        let (cum_in, cum_out, cum_cache) = messages
                            .iter()
                            .filter(|m| m.role == MessageRole::Assistant)
                            .fold((0u64, 0u64, 0u64), |(ci, co, ccr), m| {
                                (
                                    ci + m.input_tokens.unwrap_or(0) as u64,
                                    co + m.output_tokens.unwrap_or(0) as u64,
                                    ccr + m.cache_read_tokens.unwrap_or(0) as u64,
                                )
                            });

                        // Compute context_used from the last assistant message.
                        let _context_used = messages
                            .iter()
                            .filter(|m| m.role == MessageRole::Assistant)
                            .last()
                            .map(|m| {
                                m.input_tokens.unwrap_or(0) as u64
                                    + m.output_tokens.unwrap_or(0) as u64
                            })
                            .unwrap_or(0);

                        state
                            .runtime
                            .set_message_buffer(session_id, messages)
                            .await;

                        // The context_manager is lazily created from the DB
                        // record on first access via context_manager().

                        // Merge MCP servers provided by the client.
                        for mcp_server in &req.mcp_servers {
                            let (name, config) = match acp_mcp_server_to_config(mcp_server) {
                                Some(pair) => pair,
                                None => continue,
                            };
                            log::info!("ACP: adding MCP server from client: {name}");
                            if let Err(e) = state
                                .runtime
                                .mcp_manager()
                                .upsert_server(name.clone(), config)
                                .await
                            {
                                log::warn!("ACP: failed to add MCP server '{name}': {e}");
                            }
                        }

                        // Set up translator and mark session as active.
                        let context_window = state.runtime.active_model().context_window;
                        let translator =
                            crate::event_translator::EventTranslator::new(session_id, context_window);
                        *state.translator.write().await = Some(translator);
                        *state.active_session.write().await = Some(session_id);
                        *state.cumulative_input.write().await = cum_in;
                        *state.cumulative_output.write().await = cum_out;
                        *state.cumulative_cache_read.write().await = cum_cache;

                        // Reset mode to default for the loaded session.
                        *state.current_mode.write().await = SessionMode::Build;

                        log::info!("ACP: session loaded successfully: {session_id}");

                        // Advertise available modes in the response.
                        let mode_state = acp::SessionModeState::new(
                            acp::SessionModeId::new("build"),
                            vec![
                                acp::SessionMode::new("plan", "Plan")
                                    .description(
                                        "Analyze and plan before making changes",
                                    ),
                                acp::SessionMode::new("build", "Build")
                                    .description(
                                        "Write and modify code with full tool access",
                                    ),
                            ],
                        );

                        let response =
                            acp::LoadSessionResponse::new().modes(mode_state);

                        // Cache and attach config options.
                        let config_opts = build_config_options(&state.runtime);
                        *state.config_options.write().await = config_opts.clone();
                        let response = response.config_options(config_opts);

                        let _ = responder.respond(response);
                        Ok(agent_client_protocol::Handled::Yes)
                    }
                }
            },
            on_receive_request!(),
        )
        // ── list_sessions ─────────────────────────────────────────────
        .on_receive_request(
            {
                let state = &state;
                move |_req: acp::ListSessionsRequest,
                      responder: agent_client_protocol::Responder<acp::ListSessionsResponse>,
                      _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let state = state.clone();
                    async move {
                        log::info!("ACP: list_sessions");

                        let records = state
                            .runtime
                            .session_manager()
                            .list_sessions(50, 0)
                            .map_err(|e| {
                                agent_client_protocol::Error::internal_error()
                                    .data(format!("failed to list sessions: {e}"))
                            })?;

                        let sessions: Vec<acp::SessionInfo> = records
                            .into_iter()
                            .map(|r| {
                                acp::SessionInfo::new(
                                    acp::SessionId::new(r.session_id.to_string()),
                                    r.workspace_root,
                                )
                                .title(Some(r.title))
                                .updated_at(Some(
                                    r.updated_at.to_rfc3339(),
                                ))
                            })
                            .collect();

                        log::info!("ACP: list_sessions returning {} sessions", sessions.len());
                        let _ = responder.respond(acp::ListSessionsResponse::new(sessions));
                        Ok(agent_client_protocol::Handled::Yes)
                    }
                }
            },
            on_receive_request!(),
        )
        // ── set_session_mode ─────────────────────────────────────────
        .on_receive_request(
            {
                let state = &state;
                move |req: acp::SetSessionModeRequest,
                      responder: agent_client_protocol::Responder<acp::SetSessionModeResponse>,
                      cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let state = state.clone();
                    async move {
                        log::info!(
                            "ACP: set_session_mode, session={}, mode_id={}",
                            req.session_id,
                            req.mode_id,
                        );

                        // Validate session.
                        let _session_id = validate_session_id(&state, &req.session_id)
                            .await
                            .ok_or_else(|| {
                                agent_client_protocol::Error::invalid_request()
                                    .data("session ID mismatch or no active session")
                            })?;

                        // Map mode_id string to SessionMode.
                        let mode_str = req.mode_id.to_string();
                        let new_mode = match mode_str.as_str() {
                            "plan" => SessionMode::Plan,
                            "build" => SessionMode::Build,
                            _ => {
                                return Err(agent_client_protocol::Error::invalid_request()
                                    .data(format!("unknown mode: {mode_str}")))
                            }
                        };

                        // Store the new mode.
                        *state.current_mode.write().await = new_mode;
                        log::info!("ACP: session mode changed to {new_mode:?}");

                        // Notify the client of the mode change.
                        let update =
                            acp::CurrentModeUpdate::new(acp::SessionModeId::new(mode_str));
                        let notif = acp::SessionNotification::new(
                            req.session_id.clone(),
                            acp::SessionUpdate::CurrentModeUpdate(update),
                        );
                        if let Err(e) = cx.send_notification(notif) {
                            log::warn!("ACP: failed to send current_mode_update: {e}");
                        }

                        let _ = responder.respond(acp::SetSessionModeResponse::new());
                        Ok(agent_client_protocol::Handled::Yes)
                    }
                }
            },
            on_receive_request!(),
        )
        // ── session/prompt ──────────────────────────────────────────
        .on_receive_request(
            {
                let state = &state;
                move |req: acp::PromptRequest,
                      responder: agent_client_protocol::Responder<acp::PromptResponse>,
                      _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let state = state.clone();
                    async move {
                        let session_id = validate_session_id(
                            &state,
                            &req.session_id,
                        ).await.ok_or_else(|| {
                            agent_client_protocol::Error::invalid_request()
                                .data("session ID mismatch or no active session")
                        })?;

                        // Determine the prompt text content.
                        let content = extract_prompt_text(&req.prompt);
                        log::info!(
                            "ACP: session/prompt, session={session_id}, content_len={}",
                            content.len()
                        );

                        // Update session title from the first prompt.
                        if !*state.session_named.read().await {
                            let title = title_from_prompt(&content);
                            if let Err(e) = state.runtime.update_session_title(session_id, &title) {
                                log::warn!("ACP: failed to update session title: {e}");
                            }
                            *state.session_named.write().await = true;
                        }

                        let mode = *state.current_mode.read().await;
                        if let Err(e) = state
                            .runtime
                            .submit_prompt(session_id, content, mode)
                            .await
                        {
                            log::error!("ACP: failed to submit prompt: {e}");
                            // Submit failed — respond immediately with error.
                            let response =
                                acp::PromptResponse::new(acp::StopReason::EndTurn);
                            let _ = responder.respond(response);
                            return Ok(agent_client_protocol::Handled::Yes);
                        }

                        // ── Deferred response ────────────────────────
                        // Do NOT respond immediately. Instead, create a oneshot
                        // channel and pass the sender to the event loop. The
                        // receiver stays here — when the LLM turn completes
                        // (BackendEvent::Finished or Failed), the event loop
                        // sends the PromptResponse through this channel.
                        let (tx, rx) = oneshot::channel();
                        *state.pending_prompt.write().await = Some(tx);

                        tokio::spawn(async move {
                            match rx.await {
                                Ok(response) => {
                                    let _ = responder.respond(response);
                                }
                                Err(_) => {
                                    // Event loop dropped the sender (shutdown/cancel).
                                    // Respond with an error to avoid client hang.
                                    let _ = responder.respond_with_error(
                                        agent_client_protocol::Error::internal_error()
                                            .data("turn cancelled or agent shutting down"),
                                    );
                                }
                            }
                        });

                        Ok(agent_client_protocol::Handled::Yes)
                    }
                }
            },
            on_receive_request!(),
        )
        // ── session/cancel ───────────────────────────────────────────
        .on_receive_notification(
            {
                let state = &state;
                move |notification: acp::CancelNotification,
                      _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let state = state.clone();
                    async move {
                        log::info!("ACP: session/cancel, session={}", notification.session_id);

                        if let Some(session_id) = validate_session_id(
                            &state,
                            &notification.session_id,
                        ).await {
                            state.runtime.cancel_session(session_id).await;
                        }

                        // Cancel any pending prompt response.
                        if let Some(tx) = state.pending_prompt.write().await.take() {
                            let _ = tx.send(
                                acp::PromptResponse::new(acp::StopReason::EndTurn),
                            );
                        }

                        Ok(agent_client_protocol::Handled::Yes)
                    }
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
        // ── session/close ────────────────────────────────────────────
        .on_receive_request(
            {
                let state = &state;
                move |req: acp::CloseSessionRequest,
                      responder: agent_client_protocol::Responder<acp::CloseSessionResponse>,
                      _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let state = state.clone();
                    async move {
                        log::info!("ACP: session/close, session={}", req.session_id);

                        // Cancel any pending prompt.
                        if let Some(tx) = state.pending_prompt.write().await.take() {
                            let _ = tx.send(
                                acp::PromptResponse::new(acp::StopReason::EndTurn),
                            );
                        }

                        // Clear the translator and active session.
                        *state.translator.write().await = None;
                        *state.active_session.write().await = None;
                        log::info!("ACP: session closed, translator cleared");

                        let _ = responder.respond(acp::CloseSessionResponse::new());
                        Ok(agent_client_protocol::Handled::Yes)
                    }
                }
            },
            on_receive_request!(),
        )
        // ── session/set_config_option ────────────────────────────
        .on_receive_request(
            {
                let state = &state;
                move |req: acp::SetSessionConfigOptionRequest,
                      responder: agent_client_protocol::Responder<acp::SetSessionConfigOptionResponse>,
                      cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let state = state.clone();
                    async move {
                        let session_id = validate_session_id(&state, &req.session_id)
                            .await
                            .ok_or_else(|| {
                                agent_client_protocol::Error::invalid_request()
                                    .data("session ID mismatch or no active session")
                            })?;

                        let config_id = req.config_id.to_string();
                        let value = req.value;

                        match config_id.as_str() {
                            "model" => {
                                let val_str = value.as_value_id()
                                    .ok_or_else(|| invalid_value("model", "must be a string value ID"))?;
                                let val_str = val_str.to_string();
                                let parts: Vec<&str> = val_str.splitn(2, '/').collect();
                                let (provider_id, model_id) = match parts.as_slice() {
                                    [p, m] => (*p, *m),
                                    _ => return Err(invalid_value("model",
                                        "format must be 'provider_id/model_id'")),
                                };

                                let config = state.runtime.config();
                                let auth = state.runtime.auth();
                                let model = config.resolve_model_by_ids(&auth, provider_id, model_id)
                                    .map_err(|e| {
                                        agent_client_protocol::Error::invalid_request()
                                            .data(format!("failed to resolve model: {e}"))
                                    })?;

                                state.runtime.set_active_model(model);
                                state.runtime.update_config(|cfg| {
                                    cfg.default_provider = provider_id.to_string();
                                    cfg.default_model = model_id.to_string();
                                });
                                let _ = state.runtime.save_config();

                                // Update translator's context_window for UsageUpdate.size.
                                let new_window = state.runtime.active_model().context_window;
                                if let Some(ref mut t) = *state.translator.write().await {
                                    t.set_context_window(new_window);
                                }

                                log::info!("ACP: model changed to {provider_id}/{model_id}");
                            }
                            "thought_level" => {
                                let val_str = value.as_value_id()
                                    .ok_or_else(|| invalid_value("thought_level", "must be a string value ID"))?;
                                let tl_str = val_str.to_string();

                                let tl = ThinkingMatcher::match_for_model(&tl_str);
                                // Accept off/none values even if they don't match a known variant.
                                let is_off = tl_str.eq_ignore_ascii_case("none")
                                    || tl_str.ends_with(":Off")
                                    || tl_str.ends_with(":off");
                                if tl.is_none() && !is_off {
                                    // Still need to validate that the string is parseable.
                                    let parsed = tidev_types::reasoning::ThinkingLevelType::from_string(&tl_str);
                                    if parsed.is_none() && tl_str != "none" {
                                        return Err(invalid_value("thought_level",
                                            &format!("unknown thinking level: {tl_str}")));
                                    }
                                }

                                let active = state.runtime.active_model();
                                state.runtime.set_model_thinking_level(
                                    &active.provider_id,
                                    &active.model_id,
                                    &tl_str,
                                )?;

                                log::info!("ACP: thought_level changed to {tl_str}");
                            }
                            _ => {
                                return Err(agent_client_protocol::Error::invalid_request()
                                    .data(format!("unknown config option: {config_id}")));
                            }
                        }

                        // Rebuild config options (thought_level options may change after model switch).
                        let new_opts = build_config_options(&state.runtime);
                        *state.config_options.write().await = new_opts.clone();

                        // Respond with the full set of config options.
                        let response = acp::SetSessionConfigOptionResponse::new(new_opts.clone());
                        let _ = responder.respond(response);

                        // Notify the client of the config change.
                        let update = acp::ConfigOptionUpdate::new(new_opts);
                        let notif = acp::SessionNotification::new(
                            session_id.to_string(),
                            acp::SessionUpdate::ConfigOptionUpdate(update),
                        );
                        let _ = cx.send_notification(notif);

                        Ok(agent_client_protocol::Handled::Yes)
                    }
                }
            },
            on_receive_request!(),
        )
        .connect_with(Stdio::new(), async |cx| {
            // Spawn the event translator task.
            let translator_handle = {
                let state = state.clone();
                let cx = cx.clone();
                tokio::spawn(async move {
                    run_event_loop(state, event_rx, cx).await;
                })
            };

            // Spawn the permission bridge task.
            let _permission_handle = {
                crate::permission_bridge::spawn(request_rx, cx.clone())
            };

            // Wait until the connection closes.
            cx.incoming_closed().await;

            // Clean up.
            let _ = translator_handle.await;
            state.runtime.shutdown().await;
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("ACP connection error: {e}"))
}

/// Background task that reads [`BackendEvent`]s from the runtime channel,
/// translates them into ACP [`SessionNotification`]s, and sends them to
/// the client. Also triggers the deferred `session/prompt` response when
/// the LLM turn completes or fails.
async fn run_event_loop(
    state: Arc<AcpState>,
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<tidev_types::message::BackendEvent>,
    cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>,
) {
    log::info!("ACP: event loop started, waiting for events");
    use tidev_types::message::BackendEvent;

    while let Some(event) = event_rx.recv().await {
        log::debug!("ACP: event loop received event: {:?}", std::mem::discriminant(&event));

        // ── Update cumulative token counts ──────────────────
        // Before translation, update per-session cumulative values from UsageStats.
        if let BackendEvent::UsageStats {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            ..
        } = &event
        {
            *state.cumulative_input.write().await += *input_tokens as u64;
            *state.cumulative_output.write().await += *output_tokens as u64;
            *state.cumulative_cache_read.write().await += *cache_read_tokens as u64;
        }

        // Translate the event into ACP notifications.
        let notifications = {
            let mut guard = state.translator.write().await;
            match guard.as_mut() {
                Some(t) => t.translate(&event),
                None => {
                    log::debug!("ACP: no translator set, skipping event");
                    continue;
                }
            }
        };

        log::debug!("ACP: translated to {} notifications", notifications.len());
        // Send notifications to the client.
        for notif in notifications {
            if let Err(e) = cx.send_notification(notif) {
                log::warn!("ACP: failed to send notification: {e}");
            }
        }

        // ── Deferred prompt response on turn completion ──────────
        // When the LLM turn finishes (BackendEvent::Finished) or fails
        // (BackendEvent::Failed), resolve the pending `session/prompt`
        // response with the actual stop reason.
        if let BackendEvent::Finished { .. } = &event {
            let cum_in = *state.cumulative_input.read().await;
            let cum_out = *state.cumulative_output.read().await;
            let cum_cache = *state.cumulative_cache_read.read().await;
            let total = cum_in + cum_out;

            let usage = acp::Usage::new(total, cum_in, cum_out)
                .cached_read_tokens(cum_cache);

            if let Some(tx) = state.pending_prompt.write().await.take() {
                log::info!("ACP: turn completed, sending PromptResponse with usage");
                let _ = tx.send(
                    acp::PromptResponse::new(acp::StopReason::EndTurn).usage(usage),
                );
            }
        } else if matches!(&event, BackendEvent::Failed { .. }) {
            if let Some(tx) = state.pending_prompt.write().await.take() {
                log::info!("ACP: turn failed, sending PromptResponse(Error)");
                let _ = tx.send(acp::PromptResponse::new(acp::StopReason::EndTurn));
            }
        }
    }
    log::info!("ACP: event loop ended (channel closed)");
}

/// Validate that the given ACP session ID matches the active session.
///
/// Returns the tidev `Uuid` if valid, or `None` if there is no active session.
/// If the IDs don't match but an active session exists, we accept it since
/// ACP is single-session at a time.
async fn validate_session_id(
    state: &AcpState,
    request_session_id: &acp::SessionId,
) -> Option<Uuid> {
    let active = *state.active_session.read().await;
    match active {
        Some(id) => {
            if id.to_string() != request_session_id.to_string() {
                log::warn!(
                    "ACP: session ID mismatch: active={}, requested={}, using active",
                    id,
                    request_session_id
                );
            }
            Some(id)
        }
        None => {
            log::warn!("ACP: no active session for request");
            None
        }
    }
}

/// Convert an ACP `McpServer` to a tidev `(name, McpServerConfig)` pair.
fn acp_mcp_server_to_config(
    server: &acp::McpServer,
) -> Option<(String, tidev_config::mcp::McpServerConfig)> {
    match server {
        acp::McpServer::Stdio(s) => {
            let mut env = std::collections::BTreeMap::new();
            for var in &s.env {
                env.insert(var.name.clone(), var.value.clone());
            }
            Some((
                s.name.clone(),
                tidev_config::mcp::McpServerConfig::Stdio {
                    command: s.command.to_string_lossy().to_string(),
                    args: s.args.clone(),
                    cwd: None,
                    env,
                },
            ))
        }
        acp::McpServer::Http(s) => Some((
            s.name.clone(),
            tidev_config::mcp::McpServerConfig::Http {
                url: s.url.clone(),
            },
        )),
        acp::McpServer::Sse(s) => Some((
            s.name.clone(),
            tidev_config::mcp::McpServerConfig::Sse {
                url: s.url.clone(),
            },
        )),
        _ => {
            log::warn!("ACP: unsupported MCP server type, skipping");
            None
        }
    }
}

/// Build an ACP error for an invalid config option value.
fn invalid_value(config_id: &str, detail: &str) -> agent_client_protocol::Error {
    agent_client_protocol::Error::invalid_request()
        .data(format!("invalid value for '{}': {}", config_id, detail))
}

/// Extract plain text from a slice of ACP [`ContentBlock`]s.
///
/// Concatenates all text blocks; includes image references (as URI or
/// base64 marker) and resource links for context.
fn extract_prompt_text(blocks: &[acp::ContentBlock]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in blocks {
        match block {
            acp::ContentBlock::Text(text) => parts.push(text.text.clone()),
            acp::ContentBlock::Image(image) => {
                // Include image as a reference marker.
                if let Some(uri) = &image.uri {
                    parts.push(format!("[image: {} ({})]", uri, image.mime_type));
                } else {
                    parts.push(format!(
                        "[image: inline base64, {} bytes, {}]",
                        image.data.len(),
                        image.mime_type
                    ));
                }
            }
            acp::ContentBlock::ResourceLink(link) => parts.push(link.uri.clone()),
            acp::ContentBlock::Resource(resource) => {
                // Include embedded resource content if it has text.
                match &resource.resource {
                    acp::EmbeddedResourceResource::TextResourceContents(text_res) => {
                        parts.push(text_res.text.clone());
                    }
                    acp::EmbeddedResourceResource::BlobResourceContents(blob_res) => {
                        parts.push(format!("[binary resource: {}]", blob_res.uri));
                    }
                    _ => {} // Unknown resource types (non-exhaustive enum).
                }
            }
            _ => {} // Skip audio, etc.
        }
    }
    parts.join("\n")
}
