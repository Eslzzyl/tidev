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
use tidev_types::prompts::SessionMode;

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
                                // session/load is not yet implemented
                                .load_session(false)
                                .prompt_capabilities(
                                    acp::PromptCapabilities::new().image(true),
                                )
                                .mcp_capabilities(
                                    acp::McpCapabilities::new().http(true).sse(true),
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

                        let translator =
                            crate::event_translator::EventTranslator::new(session_id);
                        *state.translator.write().await = Some(translator);
                        *state.active_session.write().await = Some(session_id);
                        log::info!("ACP: translator set for session {session_id}");

                        let response = acp::NewSessionResponse::new(session_id.to_string());
                        let _ = responder.respond(response);
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

                        let content = extract_prompt_text(&req.prompt);
                        log::info!(
                            "ACP: session/prompt, session={session_id}, content_len={}",
                            content.len()
                        );

                        if let Err(e) = state
                            .runtime
                            .submit_prompt(session_id, content, SessionMode::Build)
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
        if let BackendEvent::Finished { turn, .. } = &event {
            let stop_reason = match turn.finish_reason.as_deref() {
                Some("stop") | Some("end_turn") => acp::StopReason::EndTurn,
                Some("max_tokens") | Some("length") => acp::StopReason::MaxTokens,
                _ => acp::StopReason::EndTurn,
            };
            if let Some(tx) = state.pending_prompt.write().await.take() {
                log::info!("ACP: turn completed, sending PromptResponse({stop_reason:?})");
                let _ = tx.send(acp::PromptResponse::new(stop_reason));
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
