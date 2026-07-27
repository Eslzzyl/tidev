//! ACP agent entry point.
//!
//! Wires up the ACP SDK's [`Agent`] builder with request handlers for each
//! protocol method, spawns the event translator and permission bridge tasks,
//! and runs the connection over stdio.

use std::sync::Arc;

use agent_client_protocol::schema::v1 as acp;
use agent_client_protocol::{Agent, Stdio, on_receive_request};
use anyhow::Result;
use tokio::sync::RwLock;
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
    });

    Agent.builder()
        .name("tidev")
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
                                ),
                        );
                        let _ = responder.respond(response);
                        Ok(agent_client_protocol::Handled::Yes)
                    }
                }
            },
            on_receive_request!(),
        )
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
        .on_receive_request(
            {
                let state = &state;
                move |req: acp::PromptRequest,
                      responder: agent_client_protocol::Responder<acp::PromptResponse>,
                      _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let state = state.clone();
                    async move {
                        let session_id = {
                            let active = state.active_session.read().await;
                            active.ok_or_else(|| {
                                agent_client_protocol::Error::invalid_request()
                                    .data("no active session")
                            })?
                        };

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
                            let response =
                                acp::PromptResponse::new(acp::StopReason::EndTurn);
                            let _ = responder.respond(response);
                            return Ok(agent_client_protocol::Handled::Yes);
                        }

                        // Respond immediately — actual output arrives via session/update.
                        let response = acp::PromptResponse::new(acp::StopReason::EndTurn);
                        let _ = responder.respond(response);
                        Ok(agent_client_protocol::Handled::Yes)
                    }
                }
            },
            on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = &state;
                move |req: acp::CloseSessionRequest,
                      responder: agent_client_protocol::Responder<acp::CloseSessionResponse>,
                      _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let state = state.clone();
                    async move {
                        log::info!("ACP: session/close, session={}", req.session_id);
                        *state.active_session.write().await = None;
                        *state.translator.write().await = None;
                        let response = acp::CloseSessionResponse::new();
                        let _ = responder.respond(response);
                        Ok(agent_client_protocol::Handled::Yes)
                    }
                }
            },
            on_receive_request!(),
        )
        .on_receive_notification(
            {
                let state = &state;
                move |notification: acp::CancelNotification,
                      _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let state = state.clone();
                    async move {
                        log::info!(
                            "ACP: session/cancel, session={}",
                            notification.session_id
                        );
                        if let Ok(session_id) =
                            Uuid::parse_str(&notification.session_id.to_string())
                        {
                            state.runtime.cancel_session(session_id).await;
                        }
                        Ok(agent_client_protocol::Handled::Yes)
                    }
                }
            },
            agent_client_protocol::on_receive_notification!(),
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
                let session_id = {
                    let active = state.active_session.read().await;
                    active.unwrap_or_else(Uuid::new_v4)
                };
                crate::permission_bridge::spawn(session_id, request_rx, cx.clone())
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
/// the client.
async fn run_event_loop(
    state: Arc<AcpState>,
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<tidev_types::message::BackendEvent>,
    cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>,
) {
    log::info!("ACP: event loop started, waiting for events");
    while let Some(event) = event_rx.recv().await {
        log::debug!("ACP: event loop received event: {:?}", std::mem::discriminant(&event));
        // Translate the event.
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
    }
    log::info!("ACP: event loop ended (channel closed)");
}

/// Extract plain text from a slice of ACP [`ContentBlock`]s.
///
/// Concatenates all text blocks; ignores non-text blocks for now.
fn extract_prompt_text(blocks: &[acp::ContentBlock]) -> String {
    let mut parts = Vec::new();
    for block in blocks {
        match block {
            acp::ContentBlock::Text(text) => parts.push(text.text.as_str()),
            acp::ContentBlock::ResourceLink(link) => parts.push(&link.uri),
            _ => {} // Skip images, audio, etc. for now.
        }
    }
    parts.join("\n")
}
