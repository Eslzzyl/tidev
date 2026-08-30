//! ACP v2 request handlers for tidev.

use std::sync::Arc;

use agent_client_protocol::schema::v2 as acp;
use agent_client_protocol::{
    Agent, ConnectTo, Handled, on_receive_notification, on_receive_request,
};
use tokio::sync::{Mutex as AsyncMutex, RwLock};
use uuid::Uuid;

use tidev_config::ThinkingMatcher;
use tidev_core::Mode as SessionMode;
use tidev_core::Runtime;
use tidev_llm::message::{MessageAttachment, MessageRole};
use tidev_utils::session::title_from_prompt;

struct State {
    runtime: Runtime,
    active_session: RwLock<Option<Uuid>>,
    translator: RwLock<Option<crate::v2::event_translator::EventTranslator>>,
    current_mode: RwLock<SessionMode>,
    session_named: RwLock<bool>,
    client_supports_elicitation: Arc<RwLock<bool>>,
}

type ReceiverSlot<T> = Arc<AsyncMutex<Option<tokio::sync::mpsc::UnboundedReceiver<T>>>>;

/// Commands that the ACP client may expose in its prompt UI.
fn available_commands(mode: SessionMode) -> acp::AvailableCommandsUpdate {
    let mut commands = vec![acp::AvailableCommand::new(
        "compact",
        "Compact the current session context to free space",
    )];
    if mode == SessionMode::Build {
        commands.push(
            acp::AvailableCommand::new("init", "Analyze the project and create AGENTS.md").input(
                acp::AvailableCommandInput::Text(acp::TextCommandInput::new(
                    "Optional project focus or constraints",
                )),
            ),
        );
    }
    acp::AvailableCommandsUpdate::new(commands)
}

pub(crate) fn build_agent(
    runtime: Runtime,
    event_rx_slot: ReceiverSlot<tidev_core::BackendEvent>,
    request_rx_slot: ReceiverSlot<tidev_core::FrontendRequest>,
) -> impl ConnectTo<agent_client_protocol::Client> {
    let state = Arc::new(State {
        runtime,
        active_session: RwLock::new(None),
        translator: RwLock::new(None),
        current_mode: RwLock::new(SessionMode::Build),
        session_named: RwLock::new(false),
        client_supports_elicitation: Arc::new(RwLock::new(false)),
    });

    Agent
        .v2()
        .name("tidev-v2")
        .on_receive_request(
            {
                let state = state.clone();
                move |request: acp::InitializeRequest,
                      responder: agent_client_protocol::Responder<acp::InitializeResponse>,
                      _cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let state = state.clone();
                    async move {
                *state.client_supports_elicitation.write().await = request
                    .capabilities
                    .elicitation
                    .as_ref()
                    .and_then(|capabilities| capabilities.form.as_ref())
                    .is_some();
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
                    ),
                );
                let _ = responder.respond(response);
                Ok(Handled::Yes)
                    }
                }
            },
            on_receive_request!(),
        )
        .on_receive_request({
            let state = state.clone();
            move |request: acp::NewSessionRequest,
                  responder: agent_client_protocol::Responder<acp::NewSessionResponse>,
                  cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                let state = state.clone();
                async move {
                    let workspace = state
                        .runtime
                        .workspace_for(&request.cwd.0)
                        .await
                        .map_err(internal_error)?;
                    merge_mcp_servers(workspace.mcp_manager(), &request.mcp_servers).await;
                    let title = format!("ACP session - {}", request.cwd.0.display());
                    let session_id = state
                        .runtime
                        .create_session_with_workspace(&title, &request.cwd.0)
                        .await
                        .map_err(internal_error)?;
                    activate(&state, session_id).await;
                    let response = acp::NewSessionResponse::new(session_id.to_string())
                        .config_options(
                            build_config_options(&state.runtime, *state.current_mode.read().await),
                        );
                    let _ = responder.respond(response);
                    let _ = cx.send_notification(acp::UpdateSessionNotification::new(
                        session_id.to_string(),
                        acp::SessionUpdate::AvailableCommandsUpdate(available_commands(
                            SessionMode::Build,
                        )),
                    ));
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
                    let workspace = state
                        .runtime
                        .workspace_for(&request.cwd.0)
                        .await
                        .map_err(internal_error)?;
                    merge_mcp_servers(workspace.mcp_manager(), &request.mcp_servers).await;
                    if matches!(request.replay_from, Some(acp::ReplayFrom::Start(_))) {
                        replay_messages(&state, session_id, &cx).await;
                    }
                    let response = acp::ResumeSessionResponse::new()
                        .config_options(
                            build_config_options(&state.runtime, *state.current_mode.read().await),
                        );
                    let _ = responder.respond(response);
                    let _ = cx.send_notification(acp::UpdateSessionNotification::new(
                        session_id.to_string(),
                        acp::SessionUpdate::AvailableCommandsUpdate(available_commands(
                            SessionMode::Build,
                        )),
                    ));
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
                                crate::v2::types::absolute_path(record.workspace_root),
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
                    state.runtime.cancel_session(session_id).await;
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
                    let (mut content, attachments) = extract_prompt(&request.prompt)?;
                    if content.trim() == "/compact" {
                        state
                            .runtime
                            .compact_session(session_id, Some(0))
                            .await
                            .map_err(internal_error)?;
                        let _ = responder.respond(acp::PromptResponse::new());
                        return Ok(Handled::Yes);
                    }
                    let mode = if let Some(args) = command_arguments(&content, "/init") {
                        let args = args.to_owned();
                        if *state.current_mode.read().await != SessionMode::Build {
                            return Err(invalid_error("/init requires Build mode"));
                        }
                        content = tidev_core::prompts::init_command_with_args(&args);
                        SessionMode::Build
                    } else {
                        *state.current_mode.read().await
                    };
                    if !*state.session_named.read().await {
                        let title = title_from_prompt(&content);
                        let _ = state.runtime.update_session_title(session_id, &title);
                        *state.session_named.write().await = true;
                    }
                    state
                        .runtime
                        .submit_prompt_with_attachments(session_id, mode, content, attachments, None)
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
                    if let Ok(session_id) = validate_session(&state, &notification.session_id).await {
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
                    let session_id = validate_session(&state, &request.session_id).await?;
                    state.runtime.cancel_session(session_id).await;
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
                    let _ = cx.send_notification(acp::UpdateSessionNotification::new(
                        session_id.to_string(),
                        acp::SessionUpdate::AvailableCommandsUpdate(available_commands(
                            *state.current_mode.read().await,
                        )),
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
                let permission = crate::v2::permission_bridge::spawn(
                    state.runtime.clone(),
                    request_rx,
                    cx.clone(),
                    state.client_supports_elicitation.clone(),
                );
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
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<tidev_core::BackendEvent>,
    cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>,
) {
    while let Some(event) = event_rx.recv().await {
        if state.active_session.read().await.as_ref() != Some(&event.session_id()) {
            continue;
        }
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
    *state.translator.write().await = Some(crate::v2::event_translator::EventTranslator::new(
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
        return Err(invalid_error(
            "session workspace differs from requested cwd",
        ));
    }
    state.runtime.set_message_buffer(session_id, messages).await;
    activate(state, session_id).await;
    *state.session_named.write().await = true;
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
                        .content(crate::v2::types::message_content(&message)),
                ),
                MessageRole::Assistant => acp::SessionUpdate::AgentMessage(
                    acp::AgentMessage::new(message.id.to_string())
                        .content(crate::v2::types::message_content(&message)),
                ),
                MessageRole::Tool => {
                    let Some(tool_call_id) = message.tool_call_id.clone() else {
                        continue;
                    };
                    let tool_call = tidev_llm::message::ToolCall {
                        id: tool_call_id,
                        name: message
                            .tool_name
                            .clone()
                            .unwrap_or_else(|| "tool".to_string()),
                        arguments: "{}".to_string(),
                        thought_signature: None,
                    };
                    let update = crate::v2::types::tool_call_update(
                        &tool_call,
                        Some(acp::ToolCallStatus::Completed),
                    )
                    .content(vec![acp::ToolCallContent::Content(Box::new(
                        acp::Content::new(acp::ContentBlock::Text(acp::TextContent::new(
                            &message.content,
                        ))),
                    ))])
                    .raw_output(serde_json::Value::String(message.content.clone()));
                    acp::SessionUpdate::ToolCallUpdate(update)
                }
                MessageRole::System | MessageRole::Error => acp::SessionUpdate::AgentMessage(
                    acp::AgentMessage::new(message.id.to_string()).content(vec![
                        acp::ContentBlock::Text(acp::TextContent::new(format!(
                            "[{}]\n{}",
                            message.role.label(),
                            message.content
                        ))),
                    ]),
                ),
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

fn extract_prompt(
    blocks: &[acp::ContentBlock],
) -> Result<(String, Vec<MessageAttachment>), agent_client_protocol::Error> {
    use base64::Engine as _;

    let mut text = Vec::new();
    let mut attachments = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        match block {
            acp::ContentBlock::Text(value) => text.push(value.text.clone()),
            acp::ContentBlock::Image(image) => {
                let data = base64::engine::general_purpose::STANDARD
                    .decode(&image.data)
                    .map_err(|error| invalid_error(format!("invalid image data: {error}")))?;
                attachments.push(MessageAttachment::Image {
                    filename: format!("acp-image-{index}"),
                    mime: image.mime_type.to_string(),
                    file_size: data.len() as u64,
                    data,
                });
            }
            acp::ContentBlock::ResourceLink(link) => text.push(link.uri.clone()),
            acp::ContentBlock::Audio(_)
            | acp::ContentBlock::Resource(_)
            | acp::ContentBlock::Other(_) => {
                return Err(invalid_error("unsupported ACP v2 prompt content block"));
            }
            _ => return Err(invalid_error("unsupported ACP v2 prompt content block")),
        }
    }
    Ok((text.join("\n"), attachments))
}

async fn merge_mcp_servers(mcp_manager: &tidev_core::mcp::McpManager, servers: &[acp::McpServer]) {
    for server in servers {
        let converted = match server {
            acp::McpServer::Http(server) => Some((
                server.name.clone(),
                tidev_config::mcp::McpServerConfig::Http {
                    url: server.url.clone(),
                    headers: server
                        .headers
                        .iter()
                        .map(|header| (header.name.clone(), header.value.clone()))
                        .collect(),
                    disabled: false,
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
                    disabled: false,
                },
            )),
            _ => None,
        };
        if let Some((name, config)) = converted
            && let Err(error) = mcp_manager.upsert_server(name.clone(), config).await
        {
            log::warn!("ACP v2 failed to add MCP server '{name}': {error}");
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

/// Return the text following an exact slash command, if present.
fn command_arguments<'a>(content: &'a str, command: &str) -> Option<&'a str> {
    let trimmed = content.trim();
    let rest = trimmed.strip_prefix(command)?;
    if rest.is_empty() || rest.chars().next().is_some_and(|ch| ch.is_whitespace()) {
        Some(rest.trim())
    } else {
        None
    }
}

fn invalid_error(message: impl Into<String>) -> agent_client_protocol::Error {
    agent_client_protocol::Error::invalid_request().data(message.into())
}

fn internal_error(error: impl std::fmt::Display) -> agent_client_protocol::Error {
    agent_client_protocol::Error::internal_error().data(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_command_arguments_require_an_exact_command_name() {
        assert_eq!(command_arguments("/init", "/init"), Some(""));
        assert_eq!(
            command_arguments(" /init focus on tests ", "/init"),
            Some("focus on tests")
        );
        assert_eq!(command_arguments("/initialize", "/init"), None);
    }

    #[test]
    fn init_is_advertised_only_in_build_mode() {
        let plan = serde_json::to_value(available_commands(SessionMode::Plan)).unwrap();
        let build = serde_json::to_value(available_commands(SessionMode::Build)).unwrap();
        assert_eq!(plan["availableCommands"].as_array().unwrap().len(), 1);
        assert_eq!(build["availableCommands"].as_array().unwrap().len(), 2);
    }
}
