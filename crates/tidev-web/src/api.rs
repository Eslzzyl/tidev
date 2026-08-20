use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tidev_core::{
    ApprovedTool, EventCursor, EventEnvelope, EventReplay, FrontendRequest, FrontendResponse, Mode,
    PromptSubmission,
};
use tidev_llm::message::Message;
use tokio::sync::mpsc::UnboundedReceiver;
use uuid::Uuid;

use crate::frontend::FrontendMode;

#[derive(Clone)]
pub struct AppState {
    pub runtime: tidev_core::Runtime,
    pub frontend_mode: FrontendMode,
    pub cancel: tokio_util::sync::CancellationToken,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    frontend: &'static str,
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    after: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CreateSessionRequest {
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateSessionRequest {
    title: String,
}

#[derive(Debug, Deserialize)]
struct PromptRequest {
    message_id: Option<Uuid>,
    content: String,
    mode: Option<Mode>,
    thinking_level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApprovalResponseRequest {
    approved_tools: Vec<ApprovedTool>,
}

#[derive(Debug, Deserialize)]
struct SelectModelRequest {
    provider_id: String,
    model_id: String,
}

#[derive(Debug, Deserialize)]
struct SetThinkingLevelRequest {
    provider_id: String,
    model_id: String,
    thinking_level: String,
}

#[derive(Debug, Deserialize)]
struct SetTerminalShellRequest {
    shell: String,
}

#[derive(Debug, Deserialize)]
struct AuthVerifyRequest {
    token: String,
}

#[derive(Debug, Deserialize)]
struct AuthConfigureRequest {
    new_token: String,
}

#[derive(Debug, Serialize)]
struct SessionDto {
    session_id: Uuid,
    parent_session_id: Option<Uuid>,
    workspace_root: String,
    provider_id: String,
    provider_display_name: String,
    model_id: String,
    model_display_name: String,
    title: String,
    created_at: String,
    updated_at: String,
    status: String,
    ended_at: Option<String>,
    context_summary: Option<String>,
    context_retained_from: usize,
    busy: bool,
}

#[derive(Debug, Serialize)]
struct MessageDto {
    message: Message,
    app_data: tidev_core::MessageAppData,
}

#[derive(Debug, Serialize)]
struct MessagesResponse {
    messages: Vec<MessageDto>,
}

#[derive(Debug, Serialize)]
struct SessionCreatedResponse {
    session: SessionDto,
}

#[derive(Debug, Serialize)]
struct PromptResponse {
    message_id: Uuid,
    duplicate: bool,
}

#[derive(Debug, Serialize)]
struct AcceptedResponse {
    accepted: bool,
}

#[derive(Debug, Serialize)]
struct ModelDto {
    provider_id: String,
    provider_display_name: String,
    model_id: String,
    model_display_name: String,
    connected: bool,
    active: bool,
    thinking_levels: Vec<String>,
    thinking_level: String,
}

#[derive(Debug, Serialize)]
struct TodoDto {
    content: String,
    status: String,
}

#[derive(Debug, Serialize)]
struct TodosResponse {
    todos: Vec<TodoDto>,
}

#[derive(Debug, Serialize)]
struct TerminalShellResponse {
    shell: String,
    configured: bool,
}

#[derive(Debug, Serialize)]
struct AuthStatusResponse {
    auth_required: bool,
}

#[derive(Debug, Serialize)]
struct AuthVerifyResponse {
    valid: bool,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("{error:#}"),
        }
    }
}

impl From<tidev_core::ApprovalError> for ApiError {
    fn from(error: tidev_core::ApprovalError) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: error.to_string(),
        }
    }
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health))
        .route("/auth/status", get(auth_status))
        .route("/auth/verify", post(auth_verify))
        .route("/auth/configure", post(auth_configure))
        .route("/events", get(events))
        .route("/requests", get(requests))
        .route("/models", get(list_models))
        .route("/models/select", post(select_model))
        .route("/models/thinking-level", post(set_thinking_level))
        .route("/sessions", get(list_sessions).post(create_session))
        .route(
            "/sessions/{session_id}",
            get(get_session)
                .patch(update_session)
                .delete(delete_session),
        )
        .route("/sessions/{session_id}/todos", get(todos))
        .route("/sessions/{session_id}/messages", get(messages))
        .route("/sessions/{session_id}/prompts", post(submit_prompt))
        .route("/sessions/{session_id}/cancel", post(cancel_session))
        .route("/requests/{request_id}/respond", post(respond_to_request))
        .route(
            "/config/terminal-shell",
            get(get_terminal_shell).post(set_terminal_shell),
        )
}

pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().trim_start_matches("/api");
    if matches!(path, "/auth/status" | "/auth/verify" | "/auth/configure") {
        return next.run(request).await;
    }

    let configured = configured_auth_token(&state);
    let Some(configured) = configured else {
        return next.run(request).await;
    };

    if request_auth_token(request.headers(), request.uri()).as_deref() == Some(configured.as_str())
    {
        return next.run(request).await;
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: "Unauthorized: invalid or missing auth token".to_owned(),
        }),
    )
        .into_response()
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        service: "tidev-web",
        frontend: frontend_name(state.frontend_mode),
    })
}

async fn auth_status(State(state): State<Arc<AppState>>) -> Json<AuthStatusResponse> {
    Json(AuthStatusResponse {
        auth_required: configured_auth_token(&state).is_some(),
    })
}

async fn auth_verify(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AuthVerifyRequest>,
) -> Json<AuthVerifyResponse> {
    let valid = configured_auth_token(&state)
        .as_deref()
        .is_some_and(|configured| configured == request.token);
    Json(AuthVerifyResponse { valid })
}

async fn auth_configure(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<AuthConfigureRequest>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    if let Some(existing) = configured_auth_token(&state)
        && request_auth_token(&headers, &Uri::from_static("/")).as_deref()
            != Some(existing.as_str())
    {
        return Err(ApiError {
            status: StatusCode::UNAUTHORIZED,
            message: "Invalid current auth token".to_owned(),
        });
    }

    state.runtime.update_auth(|auth| {
        auth.web.auth_token = (!request.new_token.trim().is_empty()).then_some(request.new_token);
    });
    state.runtime.save_auth()?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

async fn events(State(state): State<Arc<AppState>>, Query(query): Query<EventsQuery>) -> Response {
    let after = query.after.map(EventCursor);
    let subscription = state.runtime.subscribe_events(after).await;
    let replay = subscription.replay.clone();
    let receiver = subscription.into_receiver();
    let cancel = state.cancel.clone();
    Sse::new(event_stream(replay, receiver, cancel))
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn requests(State(state): State<Arc<AppState>>) -> Response {
    let receiver = state.runtime.request_rx().await;
    Sse::new(request_stream(receiver, state.cancel.clone()))
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn list_models(State(state): State<Arc<AppState>>) -> Json<Vec<ModelDto>> {
    let config = state.runtime.config();
    let auth = state.runtime.auth();
    let active = state.runtime.active_model();
    let models = config
        .available_models()
        .into_iter()
        .map(|model| {
            let thinking_levels =
                tidev_config::reasoning::ThinkingMatcher::supported_levels(&model.request_model_id)
                    .into_iter()
                    .map(|level| level.to_string())
                    .collect();
            let is_active =
                active.provider_id == model.provider_id && active.model_id == model.model_id;
            let thinking_level = if is_active {
                active.thinking_level.to_string()
            } else {
                state
                    .runtime
                    .session_manager()
                    .store()
                    .load_model_thinking_level(&model.provider_id, &model.model_id)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| {
                        tidev_config::reasoning::ThinkingMatcher::match_for_model(
                            &model.request_model_id,
                        )
                        .to_string()
                    })
            };
            ModelDto {
                connected: auth.api_key(&model.provider_id).is_some(),
                active: is_active,
                provider_id: model.provider_id,
                provider_display_name: model.provider_display_name,
                model_id: model.model_id,
                model_display_name: model.model_display_name,
                thinking_levels,
                thinking_level,
            }
        })
        .collect();
    Json(models)
}

async fn select_model(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SelectModelRequest>,
) -> Result<Json<ModelDto>, ApiError> {
    let config = state.runtime.config();
    let auth = state.runtime.auth();
    let model = config.resolve_model_by_ids(&auth, &request.provider_id, &request.model_id)?;
    let thinking_levels =
        tidev_config::reasoning::ThinkingMatcher::supported_levels(&model.request_model_id)
            .into_iter()
            .map(|level| level.to_string())
            .collect();
    let response = ModelDto {
        provider_id: model.provider_id.clone(),
        provider_display_name: model.provider_display_name.clone(),
        model_id: model.model_id.clone(),
        model_display_name: model.display_name.clone(),
        connected: model.api_key.is_some(),
        active: true,
        thinking_levels,
        thinking_level: model.thinking_level.to_string(),
    };
    state.runtime.set_active_model(model);
    Ok(Json(response))
}

async fn set_thinking_level(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SetThinkingLevelRequest>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state.runtime.set_model_thinking_level(
        &request.provider_id,
        &request.model_id,
        &request.thinking_level,
    )?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<SessionDto>>, ApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let workspace = state.runtime.workspace_root().to_string_lossy().to_string();
    let sessions = state
        .runtime
        .session_manager()
        .store()
        .list_sessions_for_workspace(&workspace, limit, offset)?;
    let items = sessions
        .into_iter()
        .map(|session| session_dto(&state.runtime, session))
        .collect();
    Ok(Json(items))
}

async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<SessionCreatedResponse>, ApiError> {
    let title = request
        .title
        .as_deref()
        .unwrap_or("New conversation")
        .trim();
    if title.is_empty() {
        return Err(ApiError::bad_request("session title cannot be empty"));
    }
    let session_id = state.runtime.create_default_session(title)?;
    let session = state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .ok_or_else(|| ApiError::not_found("created session is unavailable"))?;
    Ok(Json(SessionCreatedResponse {
        session: session_dto(&state.runtime, session),
    }))
}

async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<SessionDto>, ApiError> {
    let session = state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    Ok(Json(session_dto(&state.runtime, session)))
}

async fn update_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Json(request): Json<UpdateSessionRequest>,
) -> Result<Json<SessionDto>, ApiError> {
    let title = request.title.trim();
    if title.is_empty() {
        return Err(ApiError::bad_request("session title cannot be empty"));
    }
    state.runtime.update_session_title(session_id, title)?;
    get_session(State(state), Path(session_id)).await
}

async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    if state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .is_none()
    {
        return Err(ApiError::not_found("session not found"));
    }
    if state.runtime.is_session_busy(session_id) {
        return Err(ApiError::conflict(
            "stop the active conversation before deleting it",
        ));
    }
    state
        .runtime
        .session_manager()
        .store()
        .delete_session(session_id)?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

async fn messages(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<MessagesResponse>, ApiError> {
    if state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .is_none()
    {
        return Err(ApiError::not_found("session not found"));
    }
    let messages = state
        .runtime
        .session_manager()
        .load_session_messages(session_id)?
        .into_iter()
        .map(|item| MessageDto {
            message: item.message,
            app_data: item.app_data,
        })
        .collect();
    Ok(Json(MessagesResponse { messages }))
}

async fn todos(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<TodosResponse>, ApiError> {
    if state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .is_none()
    {
        return Err(ApiError::not_found("session not found"));
    }
    let todos = state
        .runtime
        .session_manager()
        .store()
        .load_todos(session_id)?
        .into_iter()
        .map(|todo| TodoDto {
            content: todo.content,
            status: todo.status,
        })
        .collect();
    Ok(Json(TodosResponse { todos }))
}

async fn submit_prompt(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Json(request): Json<PromptRequest>,
) -> Result<Json<PromptResponse>, ApiError> {
    if request.content.trim().is_empty() {
        return Err(ApiError::bad_request("prompt content cannot be empty"));
    }
    if state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .is_none()
    {
        return Err(ApiError::not_found("session not found"));
    }
    let mut submission =
        PromptSubmission::new(request.content, request.mode.unwrap_or(Mode::Build));
    if let Some(message_id) = request.message_id {
        submission.message_id = message_id;
    }
    submission.thinking_level = request
        .thinking_level
        .as_deref()
        .map(tidev_llm::reasoning::ThinkingLevelType::from_string);
    let receipt = state
        .runtime
        .submit_prompt_submission(session_id, submission)
        .await?;
    Ok(Json(PromptResponse {
        message_id: receipt.message_id,
        duplicate: receipt.duplicate,
    }))
}

async fn cancel_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state.runtime.cancel_session(session_id).await;
    Ok(Json(AcceptedResponse { accepted: true }))
}

async fn respond_to_request(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<Uuid>,
    Json(request): Json<ApprovalResponseRequest>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state.runtime.respond_to_request(
        request_id,
        FrontendResponse::ToolApproval(request.approved_tools),
    )?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

async fn get_terminal_shell(State(state): State<Arc<AppState>>) -> Json<TerminalShellResponse> {
    let config = state.runtime.config();
    #[cfg(windows)]
    let configured = config.shell.windows_shell;
    #[cfg(not(windows))]
    let configured = config.shell.unix_shell;
    let configured = configured.filter(|shell| !shell.trim().is_empty());
    let shell = configured.as_ref().cloned().unwrap_or_else(default_shell);
    Json(TerminalShellResponse {
        shell,
        configured: configured.is_some(),
    })
}

async fn set_terminal_shell(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SetTerminalShellRequest>,
) -> Result<Json<TerminalShellResponse>, ApiError> {
    let shell = request.shell.trim().to_owned();
    state.runtime.update_config(|config| {
        #[cfg(windows)]
        {
            config.shell.windows_shell = (!shell.is_empty()).then_some(shell.clone());
        }
        #[cfg(not(windows))]
        {
            config.shell.unix_shell = (!shell.is_empty()).then_some(shell.clone());
        }
    });
    state.runtime.save_config()?;
    Ok(Json(TerminalShellResponse {
        configured: !shell.is_empty(),
        shell,
    }))
}

fn configured_auth_token(state: &AppState) -> Option<String> {
    state
        .runtime
        .auth()
        .web
        .auth_token
        .filter(|token| !token.trim().is_empty())
}

fn request_auth_token(headers: &HeaderMap, uri: &Uri) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(ToOwned::to_owned)
        .or_else(|| {
            uri.query().and_then(|query| {
                url::form_urlencoded::parse(query.as_bytes())
                    .find_map(|(key, value)| (key == "token").then(|| value.into_owned()))
            })
        })
}

fn default_shell() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell".to_owned())
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_owned())
    }
}

fn session_dto(runtime: &tidev_core::Runtime, session: tidev_core::SessionRecord) -> SessionDto {
    SessionDto {
        session_id: session.session_id,
        parent_session_id: session.parent_session_id,
        workspace_root: session.workspace_root,
        provider_id: session.provider_id,
        provider_display_name: session.provider_display_name,
        model_id: session.model_id,
        model_display_name: session.model_display_name,
        title: session.title,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        status: session.status,
        ended_at: session.ended_at.map(|date| date.to_rfc3339()),
        context_summary: session.context_summary,
        context_retained_from: session.context_retained_from,
        busy: runtime.is_session_busy(session.session_id),
    }
}

fn event_stream(
    replay: EventReplay,
    mut receiver: UnboundedReceiver<EventEnvelope>,
    cancel: tokio_util::sync::CancellationToken,
) -> impl futures_core::Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        match replay {
            EventReplay::Events(events) => {
                for envelope in events {
                    yield Ok(sse_event(&envelope));
                }
            }
            EventReplay::ResyncRequired {
                after,
                oldest_available,
                latest_available,
            } => {
                let payload = serde_json::json!({
                    "after": after,
                    "oldest_available": oldest_available,
                    "latest_available": latest_available,
                });
                yield Ok(Event::default().event("resync_required").data(payload.to_string()));
            }
        }

        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                envelope = receiver.recv() => {
                    let Some(envelope) = envelope else { break };
                    yield Ok(sse_event(&envelope));
                }
            }
        }
    }
}

fn request_stream(
    mut receiver: UnboundedReceiver<FrontendRequest>,
    cancel: tokio_util::sync::CancellationToken,
) -> impl futures_core::Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                request = receiver.recv() => {
                    let Some(request) = request else { break };
                    let Ok(data) = serde_json::to_string(&request) else { continue };
                    yield Ok(Event::default().event("frontend_request").data(data));
                }
            }
        }
    }
}

fn sse_event(envelope: &EventEnvelope) -> Event {
    let data = serde_json::to_string(envelope).unwrap_or_else(|_| "{}".to_owned());
    Event::default()
        .id(envelope.cursor.0.to_string())
        .event("backend_event")
        .data(data)
}

fn frontend_name(mode: FrontendMode) -> &'static str {
    match mode {
        FrontendMode::Dev => "vite",
        FrontendMode::Embedded => "embedded",
        FrontendMode::Fallback => "fallback",
    }
}
