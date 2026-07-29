//! ACP v2 request handlers for tidev.

use std::sync::Arc;

use agent_client_protocol::schema::v2 as acp;
use agent_client_protocol::{
    Agent, ConnectTo, Handled, on_receive_notification, on_receive_request,
};
use tokio::sync::{Mutex as AsyncMutex, RwLock};
use uuid::Uuid;

use tidev_config::ThinkingMatcher;
use tidev_core::Runtime;
use tidev_types::message::MessageRole;
use tidev_types::prompts::SessionMode;
use tidev_utils::session::title_from_prompt;

struct State {
    runtime: Runtime,
    active_session: RwLock<Option<Uuid>>,
    translator: RwLock<Option<crate::v2_event_translator::EventTranslator>>,
    current_mode: RwLock<SessionMode>,
    session_named: RwLock<bool>,
}

type ReceiverSlot<T> = Arc<AsyncMutex<Option<tokio::sync::mpsc::UnboundedReceiver<T>>>>;

pub(crate) fn build_agent(
    runtime: Runtime,
    event_rx_slot: ReceiverSlot<tidev_types::message::BackendEvent>,
    request_rx_slot: ReceiverSlot<tidev_core::TuiRequest>,
) -> impl ConnectTo<agent_client_protocol::Client> {
    let state = Arc::new(State {
        runtime,
        active_session: RwLock::new(None),
        translator: RwLock::new(None),
        current_mode: RwLock::new(SessionMode::Build),
        session_named: RwLock::new(false),
    });

    Agent
        .v2()
        .name("tidev-v2")
        .on_receive_request(
            |_request: acp::InitializeRequest,
             responder: agent_client_protocol::Responder<acp::InitializeResponse>,
             _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| async move {
                let response = acp::InitializeResponse::new(
                    agent_client_protocol::schema::ProtocolVersion::V2,
                    acp::Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
                )
                .capabilities(
                    acp::AgentCapabilities::new().session(
                        acp::SessionCapabilities::new()
                            .prompt(acp::PromptCapabilities::new().image(
                                acp::PromptImageCapabilities::new(),
                            ))
                            .mcp(
                                acp::McpCapabilities::new()
                                    .stdio(acp::McpStdioCapabilities::new())
                                    .http(acp::McpHttpCapabilities::new()),
                            )
                            .delete(acp::SessionDeleteCapabilities::new())
                            .additional_directories(
                                acp::SessionAdditionalDirectoriesCapabilities::new(),
                            ),
                    ),
                );
                let _ = responder.respond(response);
                Ok(Handled::Yes)
            },
            on_receive_request!(),
        )
        .on_receive_request({
            let state = state.clone();
            move |request: acp::NewSessionRequest,
                  responder: agent_client_protocol::Responder<acp::NewSessionResponse>,
                  _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                let state = state.clone();
                async move {
                    merge_mcp_servers(&state.runtime, &request.mcp_servers).await;
                    let session_id = state
                        .runtime
                        .create_default_session(&format!("ACP session - {}", request.cwd.0.display()))
                        .map_err(internal_error)?;
                    activate(&state, session_id).await;
                    let response = acp::NewSessionResponse::new(session_id.to_string())
                        .config_options(
                            build_config_options(&state.runtime, *state.current_mode.read().await),
                        );
                    let _ = responder.respond(response);
                    Ok(Handled::Yes)
                }
            }
        }, on_receive_request!())
        .on_receive_request({
            let state = state.clone();
            move |request: acp::ResumeSessionRequest,
                  responder: agent_client_protocol::Responder<acp::ResumeSessionResponse>,
                  cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                let state = state.clone();
                async move {
                    let session_id = parse_session_id(&request.session_id)?;
                    load_and_activate(&state, session_id, &request.cwd).await?;
                    merge_mcp_servers(&state.runtime, &request.mcp_servers).await;
                    if matches!(request.replay_from, Some(acp::ReplayFrom::Start(_))) {
                        replay_messages(&state, session_id, &cx).await;
                    }
                    let response = acp::ResumeSessionResponse::new()
                        .config_options(
                            build_config_options(&state.runtime, *state.current_mode.read().await),
                        );
                    let _ = responder.respond(response);
                    Ok(Handled::Yes)
                }
            }
        }, on_receive_request!())
        .on_receive_request({
            let state = state.clone();
            move |request: acp::ListSessionsRequest,
                  responder: agent_client_protocol::Responder<acp::ListSessionsResponse>,
                  _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                let state = state.clone();
                async move {
                    let records = state
                        .runtime
                        .session_manager()
                        .list_sessions(100, 0)
                        .map_err(internal_error)?;
                    let cwd = request.cwd.as_ref().map(|path| path.0.to_string_lossy().to_string());
                    let sessions = records
                        .into_iter()
                        .filter(|record| cwd.as_ref().is_none_or(|root| root == &record.workspace_root))
                        .map(|record| {
                            acp::SessionInfo::new(
                                record.session_id.to_string(),
                                crate::v2_types::absolute_path(record.workspace_root),
                            )
                            .title(record.title)
                            .updated_at(record.updated_at.to_rfc3339())
                        })
                        .collect();
                    let _ = responder.respond(acp::ListSessionsResponse::new(sessions));
                    Ok(Handled::Yes)
                }
            }
        }, on_receive_request!())
        .on_receive_request({
            let state = state.clone();
            move |request: acp::DeleteSessionRequest,
                  responder: agent_client_protocol::Responder<acp::DeleteSessionResponse>,
                  _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                let state = state.clone();
                async move {
                    let session_id = parse_session_id(&request.session_id)?;
                    state
                        .runtime
                        .session_manager()
                        .store()
                        .delete_session(session_id)
                        .map_err(internal_error)?;
                    if *state.active_session.read().await == Some(session_id) {
                        *state.active_session.write().await = None;
                        *state.translator.write().await = None;
                    }
                    let _ = responder.respond(acp::DeleteSessionResponse::new());
                    Ok(Handled::Yes)
                }
            }
        }, on_receive_request!())
        .on_receive_request({
            let state = state.clone();
            move |request: acp::PromptRequest,
                  responder: agent_client_protocol::Responder<acp::PromptResponse>,
                  _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                let state = state.clone();
                async move {
                    let session_id = validate_session(&state, &request.session_id).await?;
                    let content = extract_prompt_text(&request.prompt);
                    if !*state.session_named.read().await {
                        let title = title_from_prompt(&content);
                        let _ = state.runtime.update_session_title(session_id, &title);
                        *state.session_named.write().await = true;
                    }
                    let mode = *state.current_mode.read().await;
                    state
                        .runtime
                        .submit_prompt(session_id, content, mode)
                        .await
                        .map_err(internal_error)?;
                    let _ = responder.respond(acp::PromptResponse::new());
                    Ok(Handled::Yes)
                }
            }
        }, on_receive_request!())
        .on_receive_notification({
            let state = state.clone();
            move |notification: acp::CancelSessionNotification,
                  _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                let state = state.clone();
                async move {
                    if let Some(session_id) = validate_session(&state, &notification.session_id).await.ok() {
                        state.runtime.cancel_session(session_id).await;
                    }
                    Ok(Handled::Yes)
                }
            }
        }, on_receive_notification!())
        .on_receive_request({
            let state = state.clone();
            move |request: acp::CloseSessionRequest,
                  responder: agent_client_protocol::Responder<acp::CloseSessionResponse>,
                  _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                let state = state.clone();
                async move {
                    let _ = validate_session(&state, &request.session_id).await?;
                    *state.active_session.write().await = None;
                    *state.translator.write().await = None;
                    let _ = responder.respond(acp::CloseSessionResponse::new());
                    Ok(Handled::Yes)
                }
            }
        }, on_receive_request!())
        .on_receive_request({
            let state = state.clone();
            move |request: acp::SetSessionConfigOptionRequest,
                  responder: agent_client_protocol::Responder<acp::SetSessionConfigOptionResponse>,
                  cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                let state = state.clone();
                async move {
                    let session_id = validate_session(&state, &request.session_id).await?;
                    apply_config_option(&state, &request).await?;
                    if let Some(translator) = state.translator.write().await.as_mut() {
                        translator.set_context_window(state.runtime.active_model().context_window);
                    }
                    let options =
                        build_config_options(&state.runtime, *state.current_mode.read().await);
                    let _ = responder.respond(acp::SetSessionConfigOptionResponse::new(options.clone()));
                    let _ = cx.send_notification(acp::UpdateSessionNotification::new(
                        session_id.to_string(),
                        acp::SessionUpdate::ConfigOptionUpdate(acp::ConfigOptionUpdate::new(options)),
                    ));
                    Ok(Handled::Yes)
                }
            }
        }, on_receive_request!())
        .with_spawned({
            let state = state.clone();
            move |cx| async move {
                let event_rx = event_rx_slot
                    .lock()
                    .await
                    .take()
                    .ok_or_else(|| internal_error("v2 event receiver already taken"))?;
                let request_rx = request_rx_slot
                    .lock()
                    .await
                    .take()
                    .ok_or_else(|| internal_error("v2 request receiver already taken"))?;
                let events = tokio::spawn(run_event_loop(state.clone(), event_rx, cx.clone()));
                let permission = crate::v2_permission_bridge::spawn(request_rx, cx.clone());
                cx.incoming_closed().await;
                events.abort();
                let _ = permission.await;
                state.runtime.shutdown().await;
                Ok(())
            }
        })
}

async fn run_event_loop(
    state: Arc<State>,
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<tidev_types::message::BackendEvent>,
    cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>,
) {
    while let Some(event) = event_rx.recv().await {
        let notifications = {
            let mut translator = state.translator.write().await;
            translator
                .as_mut()
                .map(|translator| translator.translate(&event))
                .unwrap_or_default()
        };
        for notification in notifications {
            let _ = cx.send_notification(notification);
        }
    }
}

async fn activate(state: &State, session_id: Uuid) {
    let window = state.runtime.active_model().context_window;
    *state.translator.write().await = Some(crate::v2_event_translator::EventTranslator::new(
        session_id, window,
    ));
    *state.active_session.write().await = Some(session_id);
    *state.current_mode.write().await = SessionMode::Build;
    *state.session_named.write().await = false;
}

async fn load_and_activate(
    state: &State,
    session_id: Uuid,
    cwd: &acp::AbsolutePath,
) -> Result<(), agent_client_protocol::Error> {
    let session = state
        .runtime
        .session_manager()
        .load_session(session_id)
        .map_err(internal_error)?
        .ok_or_else(|| invalid_error(format!("session not found: {session_id}")))?;
    let messages = state
        .runtime
        .session_manager()
        .load_messages(session_id)
        .map_err(internal_error)?;
    if session.workspace_root != cwd.0.to_string_lossy() {
        log::warn!("ACP v2 session workspace differs from requested cwd");
    }
    state.runtime.set_message_buffer(session_id, messages).await;
    activate(state, session_id).await;
    Ok(())
}

async fn replay_messages(
    state: &State,
    session_id: Uuid,
    cx: &agent_client_protocol::ConnectionTo<agent_client_protocol::Client>,
) {
    if let Ok(messages) = state.runtime.session_manager().load_messages(session_id) {
        for message in messages {
            let update = match message.role {
                MessageRole::User => acp::SessionUpdate::UserMessage(
                    acp::UserMessage::new(message.id.to_string())
                        .content(crate::v2_types::message_content(&message)),
                ),
                MessageRole::Assistant => acp::SessionUpdate::AgentMessage(
                    acp::AgentMessage::new(message.id.to_string())
                        .content(crate::v2_types::message_content(&message)),
                ),
                _ => continue,
            };
            let _ = cx.send_notification(acp::UpdateSessionNotification::new(
                session_id.to_string(),
                update,
            ));
        }
    }
}

fn build_config_options(
    runtime: &Runtime,
    current_mode: SessionMode,
) -> Vec<acp::SessionConfigOption> {
    let config = runtime.config();
    let auth = runtime.auth();
    let active = runtime.active_model();
    let models: Vec<acp::SessionConfigSelectOption> = config
        .connected_models(&auth)
        .into_iter()
        .map(|model| {
            acp::SessionConfigSelectOption::new(
                format!("{}/{}", model.provider_id, model.model_id),
                format!(
                    "{} ({})",
                    model.model_display_name, model.provider_display_name
                ),
            )
        })
        .collect();
    let model = acp::SessionConfigOption::select(
        "model",
        "Model",
        format!("{}/{}", active.provider_id, active.model_id),
        models,
    )
    .category(acp::SessionConfigOptionCategory::Model);
    let levels: Vec<acp::SessionConfigSelectOption> =
        ThinkingMatcher::supported_levels(&active.model_id)
            .into_iter()
            .map(|level| {
                acp::SessionConfigSelectOption::new(level.to_string(), level.display_name())
            })
            .collect();
    let thinking = acp::SessionConfigOption::select(
        "thought_level",
        "Thinking Level",
        active.thinking_level.to_string(),
        levels,
    )
    .description("Controls how much reasoning effort the model applies")
    .category(acp::SessionConfigOptionCategory::ThoughtLevel);
    let mode = acp::SessionConfigOption::select(
        "mode",
        "Mode",
        match current_mode {
            SessionMode::Plan => "plan",
            SessionMode::Build => "build",
        },
        vec![
            acp::SessionConfigSelectOption::new("plan", "Plan")
                .description("Analyze and plan before making changes"),
            acp::SessionConfigSelectOption::new("build", "Build")
                .description("Write and modify code with full tool access"),
        ],
    )
    .category(acp::SessionConfigOptionCategory::Mode);
    vec![model, thinking, mode]
}

async fn apply_config_option(
    state: &State,
    request: &acp::SetSessionConfigOptionRequest,
) -> Result<(), agent_client_protocol::Error> {
    let value = request
        .value
        .as_id()
        .ok_or_else(|| invalid_error("configuration value must be an id"))?
        .to_string();
    match request.config_id.to_string().as_str() {
        "model" => {
            let parts = value.splitn(2, '/').collect::<Vec<_>>();
            if parts.len() != 2 {
                return Err(invalid_error("model value must be provider/model"));
            }
            let model = state
                .runtime
                .config()
                .resolve_model_by_ids(&state.runtime.auth(), parts[0], parts[1])
                .map_err(internal_error)?;
            state.runtime.set_active_model(model);
            state.runtime.update_config(|config| {
                config.default_provider = parts[0].to_string();
                config.default_model = parts[1].to_string();
            });
            state.runtime.save_config().map_err(internal_error)?;
        }
        "thought_level" => {
            let active = state.runtime.active_model();
            state
                .runtime
                .set_model_thinking_level(&active.provider_id, &active.model_id, &value)
                .map_err(internal_error)?;
        }
        "mode" => match value.as_str() {
            "plan" => *state.current_mode.write().await = SessionMode::Plan,
            "build" => *state.current_mode.write().await = SessionMode::Build,
            other => return Err(invalid_error(format!("unknown mode: {other}"))),
        },
        other => return Err(invalid_error(format!("unknown config option: {other}"))),
    }
    Ok(())
}

fn extract_prompt_text(blocks: &[acp::ContentBlock]) -> String {
    blocks
        .iter()
        .map(|block| match block {
            acp::ContentBlock::Text(text) => text.text.clone(),
            acp::ContentBlock::Image(image) => {
                format!("[image: {}]", image.mime_type)
            }
            acp::ContentBlock::ResourceLink(link) => link.uri.clone(),
            other => serde_json::to_string(other).unwrap_or_default(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn merge_mcp_servers(runtime: &Runtime, servers: &[acp::McpServer]) {
    for server in servers {
        let converted = match server {
            acp::McpServer::Http(server) => Some((
                server.name.clone(),
                tidev_config::mcp::McpServerConfig::Http {
                    url: server.url.clone(),
                },
            )),
            acp::McpServer::Stdio(server) => Some((
                server.name.clone(),
                tidev_config::mcp::McpServerConfig::Stdio {
                    command: server.command.0.to_string_lossy().to_string(),
                    args: server.args.clone(),
                    cwd: None,
                    env: server
                        .env
                        .iter()
                        .map(|entry| (entry.name.clone(), entry.value.clone()))
                        .collect(),
                },
            )),
            _ => None,
        };
        if let Some((name, config)) = converted {
            if let Err(error) = runtime
                .mcp_manager()
                .upsert_server(name.clone(), config)
                .await
            {
                log::warn!("ACP v2 failed to add MCP server '{name}': {error}");
            }
        }
    }
}

async fn validate_session(
    state: &State,
    requested: &acp::SessionId,
) -> Result<Uuid, agent_client_protocol::Error> {
    let id = parse_session_id(requested)?;
    if *state.active_session.read().await != Some(id) {
        return Err(invalid_error("session ID mismatch or no active session"));
    }
    Ok(id)
}

fn parse_session_id(id: &acp::SessionId) -> Result<Uuid, agent_client_protocol::Error> {
    Uuid::parse_str(id.0.as_ref())
        .map_err(|error| invalid_error(format!("invalid session ID: {error}")))
}

fn invalid_error(message: impl Into<String>) -> agent_client_protocol::Error {
    agent_client_protocol::Error::invalid_request().data(message.into())
}

fn internal_error(error: impl std::fmt::Display) -> agent_client_protocol::Error {
    agent_client_protocol::Error::internal_error().data(error.to_string())
}
