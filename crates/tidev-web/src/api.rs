#![allow(clippy::all)]
use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;

use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use tidev_config::ApiType;
use tidev_config::provider::{ModelConfig, ProviderConfig};
use tidev_core::agent_type::AgentType;
use tidev_core::{
    ApprovedTool, EventCursor, EventEnvelope, EventReplay, FrontendRequest, FrontendResponse, Mode,
    PromptSubmission,
};
use tidev_llm::message::{Message, MessageAttachment};
use tidev_utils::path::{display_path_with_tilde, expand_tilde};
use tokio::fs;
use tokio::sync::mpsc::UnboundedReceiver;
use uuid::Uuid;

use crate::frontend::FrontendMode;

#[derive(Clone)]
pub struct AppState {
    pub runtime: tidev_core::Runtime,
    pub frontend_mode: FrontendMode,
    pub cancel: tokio_util::sync::CancellationToken,
    pub terminal_manager: std::sync::Arc<crate::terminal::TerminalManager>,
    pub terminal_tx: tokio::sync::broadcast::Sender<crate::terminal::TerminalOutput>,
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
    q: Option<String>,
    workspace_root: Option<String>,
    before_updated_at: Option<String>,
    before_session_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, Default)]
struct StatsQuery {
    granularity: Option<String>,
    start: Option<String>,
    end: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Serialize)]
struct StatsActivityCell {
    date: String,
    request_count: u64,
    total_tokens: u64,
    level: u8,
}

#[derive(Debug, Serialize)]
struct StatsActivityResponse {
    start_date: String,
    end_date: String,
    total_requests: u64,
    total_tokens: u64,
    cells: Vec<StatsActivityCell>,
}

#[derive(Debug, Serialize)]
struct StatsActiveSessionPoint {
    time_bucket: String,
    active_sessions: u64,
}

#[derive(Debug, Serialize)]
struct StatsRhythmCell {
    weekday: u8,
    hour: u8,
    request_count: u64,
    total_tokens: u64,
    level: u8,
}

#[derive(Debug, Serialize)]
struct StatsRhythmResponse {
    cells: Vec<StatsRhythmCell>,
}

#[derive(Debug, Serialize)]
struct StatsModelMixSeries {
    key: String,
    provider_display_name: String,
    model_display_name: String,
    is_other: bool,
}

#[derive(Debug, Serialize)]
struct StatsModelMixPoint {
    time_bucket: String,
    shares: BTreeMap<String, f64>,
}

#[derive(Debug, Serialize)]
struct StatsModelMixResponse {
    series: Vec<StatsModelMixSeries>,
    points: Vec<StatsModelMixPoint>,
}

#[derive(Debug, Serialize)]
struct StatsRequestSizeBucket {
    lower_bound: u64,
    upper_bound: Option<u64>,
    request_count: u64,
    total_tokens: u64,
}

#[derive(Debug, Serialize)]
struct StatsInsightsResponse {
    granularity: String,
    active_sessions: Vec<StatsActiveSessionPoint>,
    rhythm: StatsRhythmResponse,
    model_mix: StatsModelMixResponse,
    request_size_distribution: Vec<StatsRequestSizeBucket>,
}

#[derive(Debug, Deserialize)]
struct CreateSessionRequest {
    title: Option<String>,
    workspace_root: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkspacePathQuery {
    path: String,
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
    #[serde(default)]
    attachments: Vec<PromptImageAttachmentRequest>,
}

#[derive(Debug, Deserialize)]
struct PromptImageAttachmentRequest {
    #[serde(rename = "type")]
    attachment_type: String,
    filename: String,
    mime: String,
    data: Vec<u8>,
}

impl PromptImageAttachmentRequest {
    fn into_message_attachment(self) -> Result<MessageAttachment, ApiError> {
        if self.attachment_type != "image" {
            return Err(ApiError::bad_request(
                "prompt attachments must have type image",
            ));
        }
        if !self.mime.starts_with("image/") {
            return Err(ApiError::bad_request(
                "prompt attachment MIME must be an image",
            ));
        }
        if self.data.is_empty() {
            return Err(ApiError::bad_request(
                "prompt image attachment cannot be empty",
            ));
        }

        Ok(MessageAttachment::Image {
            filename: self.filename,
            mime: self.mime,
            file_size: self.data.len() as u64,
            data: self.data,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RetryRequest {
    message_id: Uuid,
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
struct SetAgentModelRequest {
    agent_type: String,
    model_str: String,
    thinking_level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SetSubagentEnabledRequest {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct SetTerminalShellRequest {
    shell: String,
}

#[derive(Debug, Deserialize)]
struct ConnectProviderRequest {
    api_key: String,
}

#[derive(Debug, Deserialize)]
struct CreateProviderRequest {
    provider_id: String,
    display_name: String,
    base_url: String,
    #[serde(default)]
    api_type: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
    api_key: String,
    models: Vec<CreateModelRequest>,
}

#[derive(Debug, Deserialize)]
struct CreateModelRequest {
    model_id: String,
    display_name: String,
    #[serde(default)]
    request_model_id: Option<String>,
    context_window: usize,
    max_output_tokens: usize,
    #[serde(default)]
    api_type: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default = "default_true")]
    supports_streaming: bool,
    #[serde(default)]
    supports_images: bool,
    #[serde(default = "default_true")]
    supports_parallel_tool_calls: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct AuthVerifyRequest {
    token: String,
}

#[derive(Debug, Deserialize)]
struct AuthConfigureRequest {
    new_token: String,
}

#[derive(Debug, Deserialize)]
struct FileSearchQuery {
    query: Option<String>,
    q: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RevertRequest {
    message_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct ForkRequest {
    message_id: Uuid,
    title: Option<String>,
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
struct SessionListCursorDto {
    updated_at: String,
    session_id: Uuid,
}

#[derive(Debug, Serialize)]
struct SessionListResponse {
    items: Vec<SessionDto>,
    next_cursor: Option<SessionListCursorDto>,
    workspace_roots: Vec<String>,
}

#[derive(Debug, Serialize)]
struct WorkspaceContextResponse {
    workspace_root: String,
    workspace_display: String,
    workspace_name: String,
    git_branch: Option<String>,
}

#[derive(Debug, Serialize)]
struct WorkspaceCompletionResponse {
    directories: Vec<String>,
    parent: Option<String>,
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
    context_window: usize,
    connected: bool,
    active: bool,
    supports_vision: bool,
    thinking_levels: Vec<String>,
    thinking_level: String,
}

#[derive(Debug, Serialize)]
struct ProviderModelDto {
    id: String,
    display_name: String,
    request_model_id: Option<String>,
    context_window: usize,
    max_output_tokens: usize,
    api_type: Option<String>,
    base_url: Option<String>,
    temperature: Option<f32>,
    supports_images: bool,
    supports_streaming: bool,
    supports_parallel_tool_calls: bool,
}

#[derive(Debug, Serialize)]
struct ProviderDto {
    id: String,
    display_name: String,
    source: &'static str,
    can_delete: bool,
    connected: bool,
    base_url: String,
    api_type: Option<String>,
    user_agent: Option<String>,
    models: Vec<ProviderModelDto>,
}

#[derive(Debug, Serialize)]
struct ProvidersResponse {
    providers: Vec<ProviderDto>,
}

#[derive(Debug, Serialize)]
struct ProviderMutationResponse {
    success: bool,
    connected: Option<bool>,
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

#[derive(Debug, Serialize, Deserialize)]
pub struct McpServerDto {
    pub name: String,
    pub kind: String,
    pub status: String,
    pub error: Option<String>,
    pub disabled: bool,
    pub config: Option<tidev_config::mcp::McpServerConfig>,
    pub tools: Vec<McpToolDto>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpToolDto {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct UpsertMcpServerRequest {
    pub name: String,
    pub config: tidev_config::mcp::McpServerConfig,
    pub original_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDto {
    pub name: String,
    pub description: String,
    pub directory: String,
    pub location: String,
    pub is_bundled: bool,
    pub companion_files: Vec<String>,
    pub content: String,
    pub document: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillListResponse {
    pub skills: Vec<SkillDto>,
}

#[derive(Debug, Deserialize)]
pub struct SkillFileQuery {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFileResponse {
    pub content: String,
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
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
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
        .route("/sessions/{session_id}/retry", post(retry_session))
        .route("/sessions/{session_id}/cancel", post(cancel_session))
        .route("/sessions/{session_id}/revert", post(revert))
        .route("/sessions/{session_id}/redo", post(redo))
        .route("/sessions/{session_id}/fork", post(fork_session))
        .route("/sessions/{session_id}/compact", post(compact))
        .route("/files/search", get(search_files))
        .route("/fs/list", get(fs_list))
        .route("/fs/read", get(fs_read))
        .route("/fs/write", post(fs_write))
        .route("/fs/create", post(fs_create))
        .route("/fs/rename", post(fs_rename))
        .route("/fs/remove", delete(fs_remove))
        .route("/fs/read-base64", get(fs_read_base64))
        .route("/git/status", get(git_status))
        .route("/git/branches", get(git_branches))
        .route("/git/history", get(git_log))
        .route("/git/graph", get(git_graph))
        .route("/git/show/{sha}", get(git_show_files))
        .route("/git/show/{sha}/diff", get(git_show_diff))
        .route("/git/diff/file", get(git_diff_file))
        .route("/git/commit", post(git_commit))
        .route("/git/branch", post(git_branch_create))
        .route("/git/branch/{name}", delete(git_branch_delete))
        .route("/git/push", post(git_push))
        .route("/git/pull", post(git_pull))
        .route("/git/stash", post(git_stash))
        .route("/git/stash/pop", post(git_stash_pop))
        .route("/workspace", get(get_workspace))
        .route("/workspaces/context", get(workspace_context))
        .route("/workspaces/complete", get(workspace_complete))
        .route("/init", get(get_init))
        .route("/tools", get(list_tools))
        .route("/skills", get(list_skills))
        .route("/skills/{name}", get(get_skill))
        .route("/skills/{name}/file", get(get_skill_file))
        .route("/providers", get(list_providers).post(create_provider))
        .route("/providers/{id}", delete(delete_provider))
        .route(
            "/providers/{id}/connect",
            post(connect_provider).delete(disconnect_provider),
        )
        .route(
            "/config/default-model",
            get(get_default_model).post(set_default_model),
        )
        .route(
            "/config/agent-models",
            get(get_agent_models).post(set_agent_model),
        )
        .route(
            "/config/subagent",
            get(get_subagent_config).post(set_subagent_config),
        )
        .route(
            "/config/memory-model",
            get(get_memory_model).post(set_memory_model),
        )
        .route(
            "/config/model-thinking-level",
            get(get_model_thinking_level).post(set_model_thinking_level),
        )
        .route("/stats/summary", get(stats_summary))
        .route("/stats/timeseries", get(stats_timeseries))
        .route("/stats/models", get(stats_models))
        .route("/stats/providers", get(stats_providers))
        .route("/stats/sessions", get(stats_sessions))
        .route("/stats/overview", get(stats_overview))
        .route("/stats/activity", get(stats_activity))
        .route("/stats/insights", get(stats_insights))
        .merge(crate::terminal_api::terminal_routes())
        .route("/system/restart", post(system_restart))
        .route("/requests/{request_id}/respond", post(respond_to_request))
        .route(
            "/config/terminal-shell",
            get(get_terminal_shell).post(set_terminal_shell),
        )
        .route(
            "/mcp/servers",
            get(list_mcp_servers).post(upsert_mcp_server),
        )
        .route("/mcp/servers/{name}", delete(delete_mcp_server))
        .route("/mcp/servers/{name}/connect", post(connect_mcp_server))
        .route(
            "/mcp/servers/{name}/disconnect",
            post(disconnect_mcp_server),
        )
        .route("/mcp/servers/{name}/refresh", post(refresh_mcp_server))
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
                tidev_config::reasoning::ThinkingMatcher::match_for_model(&model.request_model_id)
                    .to_string()
            };
            ModelDto {
                connected: auth.api_key(&model.provider_id).is_some(),
                active: is_active,
                supports_vision: model.supports_images,
                provider_id: model.provider_id,
                provider_display_name: model.provider_display_name,
                model_id: model.model_id,
                model_display_name: model.model_display_name,
                context_window: model.context_window,
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
        context_window: model.context_window,
        connected: model.api_key.is_some(),
        active: true,
        supports_vision: model.supports_images,
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
) -> Result<Json<SessionListResponse>, ApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let cursor = match (query.before_updated_at.as_deref(), query.before_session_id) {
        (None, None) => None,
        (Some(updated_at), Some(session_id)) => {
            let updated_at = DateTime::parse_from_rfc3339(updated_at)
                .map_err(|_| ApiError::bad_request("invalid session cursor timestamp"))?
                .with_timezone(&Utc);
            Some((updated_at, session_id))
        }
        _ => {
            return Err(ApiError::bad_request(
                "session cursor requires both before_updated_at and before_session_id",
            ));
        }
    };
    let store = state.runtime.session_manager().store();
    let mut sessions = store.list_sessions_by_activity(
        limit + 1,
        cursor,
        query.q.as_deref(),
        query.workspace_root.as_deref(),
    )?;
    let next_cursor = if sessions.len() > limit as usize {
        sessions.pop();
        sessions.last().map(|session| SessionListCursorDto {
            updated_at: session.updated_at.to_rfc3339(),
            session_id: session.session_id,
        })
    } else {
        None
    };
    let workspace_roots = store.list_session_workspace_roots()?;
    let items = sessions
        .into_iter()
        .map(|session| session_dto(&state.runtime, session))
        .collect();
    Ok(Json(SessionListResponse {
        items,
        next_cursor,
        workspace_roots,
    }))
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
    let session_id = match request.workspace_root.as_deref() {
        Some(workspace_root) if !workspace_root.trim().is_empty() => {
            let workspace_root = canonical_workspace_path(workspace_root).await?;
            state
                .runtime
                .create_session_with_workspace(title, workspace_root)
                .await?
        }
        _ => state.runtime.create_default_session(title)?,
    };
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
    if request.content.trim().is_empty() && request.attachments.is_empty() {
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
    let attachments = request
        .attachments
        .into_iter()
        .map(PromptImageAttachmentRequest::into_message_attachment)
        .collect::<Result<Vec<_>, _>>()?;
    if !attachments.is_empty() && !state.runtime.active_model().supports_images {
        return Err(ApiError::bad_request(
            "current model does not support image attachments",
        ));
    }
    let mut submission =
        PromptSubmission::new(request.content, request.mode.unwrap_or(Mode::Build));
    submission.attachments = attachments;
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

async fn retry_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Json(request): Json<RetryRequest>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    if state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .is_none()
    {
        return Err(ApiError::not_found("session not found"));
    }
    state
        .runtime
        .retry_session(session_id, request.message_id)
        .await?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

async fn revert(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Json(request): Json<RevertRequest>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state.runtime.revert(session_id, request.message_id).await?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

async fn redo(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state.runtime.cancel_session(session_id).await;
    state.runtime.redo(session_id).await?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

async fn fork_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Json(request): Json<ForkRequest>,
) -> Result<Json<SessionDto>, ApiError> {
    let new_session_id =
        state
            .runtime
            .fork_session(session_id, request.message_id, request.title)?;
    let record = state
        .runtime
        .session_manager()
        .load_session(new_session_id)?
        .ok_or_else(|| ApiError::not_found("forked session not found"))?;
    Ok(Json(SessionDto {
        session_id: record.session_id,
        parent_session_id: record.parent_session_id,
        workspace_root: record.workspace_root,
        provider_id: record.provider_id,
        provider_display_name: record.provider_display_name,
        model_id: record.model_id,
        model_display_name: record.model_display_name,
        title: record.title,
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
        status: record.status,
        ended_at: record.ended_at.map(|time| time.to_rfc3339()),
        context_summary: record.context_summary,
        context_retained_from: record.context_retained_from,
        busy: false,
    }))
}

async fn compact(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state.runtime.compact_session(session_id, None).await?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

async fn search_files(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FileSearchQuery>,
) -> Json<serde_json::Value> {
    let q = query.query.or(query.q).unwrap_or_default();
    let workspace = state.runtime.workspace_root().clone();
    let index = state.runtime.file_search_index();
    index.ensure_background_indexing(&workspace);
    let suggestions = index.search(&q);
    let limited = suggestions.into_iter().take(20).collect::<Vec<_>>();
    Json(serde_json::json!({ "files": limited }))
}

#[derive(Debug, Deserialize)]
struct ListDirParams {
    path: Option<String>,
}

#[derive(Debug, Serialize)]
struct DirectoryEntry {
    name: String,
    path: String,
    is_directory: bool,
    is_symlink: bool,
    size: Option<u64>,
    modified: Option<String>,
}

#[derive(Debug, Serialize)]
struct ListDirResponse {
    directory: String,
    entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Deserialize)]
struct ReadFileParams {
    path: String,
}

#[derive(Debug, Serialize)]
struct ReadFileResponse {
    content: String,
    path: String,
    language: Option<String>,
    line_count: usize,
    size: u64,
}

#[derive(Debug, Deserialize)]
struct WriteFileRequest {
    path: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct WriteFileResponse {
    path: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
struct CreateItemRequest {
    path: String,
    #[serde(rename = "type")]
    item_type: String,
}

#[derive(Debug, Serialize)]
struct CreateItemResponse {
    path: String,
    #[serde(rename = "type")]
    item_type: String,
}

#[derive(Debug, Deserialize)]
struct RenameItemRequest {
    path: String,
    new_path: String,
}

#[derive(Debug, Serialize)]
struct RenameItemResponse {
    path: String,
    new_path: String,
}

#[derive(Debug, Deserialize)]
struct RemoveItemRequest {
    path: String,
}

#[derive(Debug, Serialize)]
struct RemoveItemResponse {
    path: String,
}

#[derive(Debug, Deserialize)]
struct ReadBase64Params {
    path: String,
}

#[derive(Debug, Serialize)]
struct ReadBase64Response {
    path: String,
    data: String,
    mime: String,
}

async fn fs_list(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListDirParams>,
) -> Result<Json<ListDirResponse>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let requested = params.path.unwrap_or_default();
    let target = resolve_path(&workspace_root, &requested)?;
    let directory = target.to_string_lossy().to_string();
    let mut entries = Vec::new();
    let mut read_dir = fs::read_dir(&target)
        .await
        .map_err(|e| ApiError::not_found(format!("Directory not found: {e}")))?;
    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let rel_path = entry
            .path()
            .strip_prefix(&workspace_root)
            .unwrap_or(&entry.path())
            .to_string_lossy()
            .to_string();
        let metadata = entry.metadata().await.ok();
        let is_symlink = entry
            .file_type()
            .await
            .map(|ft| ft.is_symlink())
            .unwrap_or(false);
        let is_directory = entry
            .file_type()
            .await
            .map(|ft| ft.is_dir())
            .unwrap_or(false);
        entries.push(DirectoryEntry {
            name,
            path: rel_path,
            is_directory,
            is_symlink,
            size: metadata
                .as_ref()
                .and_then(|m| if m.is_file() { Some(m.len()) } else { None }),
            modified: metadata
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| {
                    let secs = d.as_secs();
                    format!("{}", secs)
                }),
        });
    }
    entries.sort_by(|a, b| match (a.is_directory, b.is_directory) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(Json(ListDirResponse { directory, entries }))
}

async fn fs_read(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ReadFileParams>,
) -> Result<Json<ReadFileResponse>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let target = resolve_path(&workspace_root, &params.path)?;
    let metadata = fs::metadata(&target)
        .await
        .map_err(|e| ApiError::not_found(format!("File not found: {e}")))?;
    if metadata.is_dir() {
        return Err(ApiError::bad_request("Path is a directory"));
    }
    if metadata.len() > 10 * 1024 * 1024 {
        return Err(ApiError::bad_request("File too large (max 10MB)"));
    }
    let content = fs::read_to_string(&target)
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to read file: {e}")))?;
    let line_count = content.lines().count();
    let language = detect_language(&params.path);
    Ok(Json(ReadFileResponse {
        content,
        path: params.path,
        language,
        line_count,
        size: metadata.len(),
    }))
}

async fn fs_write(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WriteFileRequest>,
) -> Result<Json<WriteFileResponse>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let target = resolve_path(&workspace_root, &body.path)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::bad_request(format!("Failed to create parent dir: {e}")))?;
    }
    fs::write(&target, body.content.as_bytes())
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to write file: {e}")))?;
    let size = body.content.len() as u64;
    Ok(Json(WriteFileResponse {
        path: body.path,
        size,
    }))
}

async fn fs_create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateItemRequest>,
) -> Result<Json<CreateItemResponse>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let target = resolve_path_for_create(&workspace_root, &body.path)?;
    if body.item_type == "directory" {
        fs::create_dir_all(&target)
            .await
            .map_err(|e| ApiError::bad_request(format!("Failed to create directory: {e}")))?;
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| ApiError::bad_request(format!("Failed to create parent dir: {e}")))?;
        }
        fs::write(&target, b"")
            .await
            .map_err(|e| ApiError::bad_request(format!("Failed to create file: {e}")))?;
    }
    Ok(Json(CreateItemResponse {
        path: body.path,
        item_type: body.item_type,
    }))
}

async fn fs_rename(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RenameItemRequest>,
) -> Result<Json<RenameItemResponse>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let source = resolve_path(&workspace_root, &body.path)?;
    let dest = resolve_path_for_create(&workspace_root, &body.new_path)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::bad_request(format!("Failed to create dest parent: {e}")))?;
    }
    fs::rename(&source, &dest)
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to rename: {e}")))?;
    Ok(Json(RenameItemResponse {
        path: body.path,
        new_path: body.new_path,
    }))
}

async fn fs_remove(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RemoveItemRequest>,
) -> Result<Json<RemoveItemResponse>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let target = resolve_path(&workspace_root, &params.path)?;
    let metadata = fs::metadata(&target)
        .await
        .map_err(|e| ApiError::not_found(format!("Path not found: {e}")))?;
    if metadata.is_dir() {
        fs::remove_dir_all(&target)
            .await
            .map_err(|e| ApiError::bad_request(format!("Failed to remove dir: {e}")))?;
    } else {
        fs::remove_file(&target)
            .await
            .map_err(|e| ApiError::bad_request(format!("Failed to remove file: {e}")))?;
    }
    Ok(Json(RemoveItemResponse { path: params.path }))
}

async fn fs_read_base64(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ReadBase64Params>,
) -> Result<Json<ReadBase64Response>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let target = resolve_path(&workspace_root, &params.path)?;
    let bytes = fs::read(&target)
        .await
        .map_err(|e| ApiError::not_found(format!("File not found: {e}")))?;
    if bytes.len() > 10 * 1024 * 1024 {
        return Err(ApiError::bad_request("File too large"));
    }
    let mime = mime_guess::from_path(&target)
        .first_or_octet_stream()
        .to_string();
    let data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    Ok(Json(ReadBase64Response {
        path: params.path,
        data,
        mime,
    }))
}

fn resolve_path(workspace_root: &StdPath, requested: &str) -> Result<PathBuf, ApiError> {
    let base = if requested.is_empty() || requested == "/" || requested == "." {
        workspace_root.to_path_buf()
    } else {
        let clean = requested.trim_start_matches('/');
        workspace_root.join(clean)
    };
    let canonical = base
        .canonicalize()
        .map_err(|e| ApiError::not_found(format!("Path not found: {e}")))?;
    if !canonical.starts_with(workspace_root) {
        return Err(ApiError::forbidden(
            "Access denied: path is outside workspace",
        ));
    }
    Ok(canonical)
}

fn resolve_path_for_create(workspace_root: &StdPath, requested: &str) -> Result<PathBuf, ApiError> {
    let clean = requested.trim_start_matches('/');
    let target = workspace_root.join(clean);
    let parent = target
        .parent()
        .ok_or_else(|| ApiError::bad_request("Invalid path"))?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|e| ApiError::not_found(format!("Parent directory not found: {e}")))?;
    if !canonical_parent.starts_with(workspace_root) {
        return Err(ApiError::forbidden(
            "Access denied: path is outside workspace",
        ));
    }
    let file_name = target
        .file_name()
        .ok_or_else(|| ApiError::bad_request("Invalid path"))?;
    Ok(canonical_parent.join(file_name))
}

fn detect_language(path: &str) -> Option<String> {
    let ext = StdPath::new(path).extension()?.to_str()?;
    let lang = match ext {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "rb" => "ruby",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" => "cpp",
        "cs" => "csharp",
        "css" | "scss" | "sass" | "less" => "css",
        "html" | "htm" => "html",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "sql" => "sql",
        "sh" | "bash" | "zsh" => "bash",
        "dockerfile" | "Dockerfile" => "dockerfile",
        "vue" => "vue",
        "svelte" => "svelte",
        "astro" => "astro",
        "tex" => "latex",
        "xml" | "svg" => "xml",
        _ => return None,
    };
    Some(lang.to_string())
}

// ── Git helpers and handlers (ported from last-full) ──────────────────────────

fn git_workspace(state: &AppState) -> PathBuf {
    state.runtime.workspace_root().clone()
}

fn run_git(args: &[&str], cwd: &PathBuf) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("Failed to run git: {e}"))?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|e| format!("Invalid UTF-8: {e}"))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr.trim().to_string())
    }
}

#[derive(Serialize)]
struct GitStatusResponse {
    branch: String,
    sha: String,
    files: Vec<GitStatusFile>,
    ahead: i32,
    behind: i32,
}

#[derive(Serialize)]
struct GitStatusFile {
    path: String,
    status: String,
    staged: bool,
}

#[derive(Serialize)]
struct GitBranchesResponse {
    current: String,
    branches: Vec<GitBranchItem>,
}

#[derive(Serialize)]
struct GitBranchItem {
    name: String,
    current: bool,
    remote: Option<String>,
}

#[derive(Serialize)]
struct GitLogResponse {
    commits: Vec<GitCommitItem>,
    has_more: bool,
}

#[derive(Serialize)]
struct GitCommitItem {
    sha: String,
    message: String,
    author: String,
    date: String,
}

#[derive(Serialize)]
struct GitGraphResponse {
    commits: Vec<GitGraphCommit>,
}

#[derive(Serialize)]
struct GitGraphCommit {
    sha: String,
    parents: Vec<String>,
    message: String,
    author: String,
    date: String,
    refs: Vec<String>,
}

#[derive(Serialize)]
struct GitShowResponse {
    sha: String,
    author: String,
    date: String,
    message: String,
    files: Vec<GitCommitFileInfo>,
    total_additions: usize,
    total_deletions: usize,
}

#[derive(Serialize)]
struct GitCommitFileInfo {
    path: String,
    status: String,
    additions: usize,
    deletions: usize,
}

#[derive(Serialize)]
struct GitFileDiffResponse {
    path: String,
    diff: String,
}

#[derive(Deserialize)]
struct GitLogParams {
    skip: Option<usize>,
    count: Option<usize>,
}

#[derive(Deserialize)]
struct GitGraphParams {
    count: Option<usize>,
}

#[derive(Deserialize)]
struct GitDiffFileParams {
    path: String,
    staged: Option<bool>,
}

#[derive(Deserialize)]
struct GitCommitRequest {
    message: String,
}

#[derive(Deserialize)]
struct GitBranchCreateRequest {
    name: String,
    checkout: Option<bool>,
}

#[derive(Serialize)]
struct GitMessageResponse {
    success: bool,
    message: String,
}

async fn git_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GitStatusResponse>, ApiError> {
    let cwd = git_workspace(&state);
    let branch = run_git(&["rev-parse", "--abbrev-ref", "HEAD"], &cwd)
        .unwrap_or_else(|_| "unknown".to_string());
    let sha = run_git(&["rev-parse", "HEAD"], &cwd)
        .unwrap_or_default()
        .trim()
        .to_string();
    let status_output = run_git(&["status", "--porcelain", "-b"], &cwd).unwrap_or_default();
    let mut files = Vec::new();
    for line in status_output.lines().skip(1) {
        if line.len() < 3 {
            continue;
        }
        let status = line[0..2].trim().to_string();
        let path = line[3..].trim().to_string();
        let staged = line
            .chars()
            .next()
            .map(|c| c != ' ' && c != '?')
            .unwrap_or(false);
        files.push(GitStatusFile {
            path,
            status,
            staged,
        });
    }
    // ahead/behind parsing from first line like "## main...origin/main [ahead 1, behind 2]"
    let first_line = status_output.lines().next().unwrap_or("");
    let ahead = if first_line.contains("ahead") {
        first_line
            .split("ahead ")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    } else {
        0
    };
    let behind = if first_line.contains("behind") {
        first_line
            .split("behind ")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    } else {
        0
    };
    Ok(Json(GitStatusResponse {
        branch: branch.trim().to_string(),
        sha,
        files,
        ahead,
        behind,
    }))
}

async fn git_branches(
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<GitBranchesResponse>, ApiError> {
    let cwd = git_workspace(&state);
    let _show_submodules = params
        .get("showSubmodules")
        .map(|v| v == "true")
        .unwrap_or(false);
    let current = run_git(&["rev-parse", "--abbrev-ref", "HEAD"], &cwd)
        .unwrap_or_default()
        .trim()
        .to_string();
    let output =
        run_git(&["branch", "--all", "--format=%(refname:short)"], &cwd).unwrap_or_default();
    let branches = output
        .lines()
        .map(|line| {
            let name = line.trim().to_string();
            let is_current = name == current;
            GitBranchItem {
                name: name.clone(),
                current: is_current,
                remote: if name.contains('/') { Some(name) } else { None },
            }
        })
        .collect();
    Ok(Json(GitBranchesResponse { current, branches }))
}

async fn git_log(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GitLogParams>,
) -> Result<Json<GitLogResponse>, ApiError> {
    let cwd = git_workspace(&state);
    let skip = params.skip.unwrap_or(0);
    let count = params.count.unwrap_or(50);
    let output = run_git(
        &[
            "log",
            &format!("--skip={skip}"),
            &format!("-n{}", count + 1),
            "--pretty=format:%H|%s|%an|%aI",
        ],
        &cwd,
    )
    .unwrap_or_default();
    let lines: Vec<&str> = output.lines().collect();
    let has_more = lines.len() > count;
    let commits = lines
        .into_iter()
        .take(count)
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() < 4 {
                return None;
            }
            Some(GitCommitItem {
                sha: parts[0].to_string(),
                message: parts[1].to_string(),
                author: parts[2].to_string(),
                date: parts[3].to_string(),
            })
        })
        .collect();
    Ok(Json(GitLogResponse { commits, has_more }))
}

async fn git_graph(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GitGraphParams>,
) -> Result<Json<GitGraphResponse>, ApiError> {
    let cwd = git_workspace(&state);
    let count = params.count.unwrap_or(100);
    let output = run_git(
        &[
            "log",
            "--all",
            &format!("-n{count}"),
            "--pretty=format:%H|%P|%s|%an|%aI|%D",
        ],
        &cwd,
    )
    .unwrap_or_default();
    let commits = output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(6, '|').collect();
            if parts.len() < 6 {
                return None;
            }
            Some(GitGraphCommit {
                sha: parts[0].to_string(),
                parents: if parts[1].is_empty() {
                    vec![]
                } else {
                    parts[1].split(' ').map(|s| s.to_string()).collect()
                },
                message: parts[2].to_string(),
                author: parts[3].to_string(),
                date: parts[4].to_string(),
                refs: if parts[5].is_empty() {
                    vec![]
                } else {
                    parts[5].split(", ").map(|s| s.to_string()).collect()
                },
            })
        })
        .collect();
    Ok(Json(GitGraphResponse { commits }))
}

async fn git_show_files(
    State(state): State<Arc<AppState>>,
    Path(sha): Path<String>,
) -> Result<Json<GitShowResponse>, ApiError> {
    let cwd = git_workspace(&state);
    let metadata = run_git(
        &["show", "-s", "--format=%H%x1f%an%x1f%aI%x1f%s", &sha],
        &cwd,
    )
    .map_err(ApiError::not_found)?;
    let fields: Vec<&str> = metadata.trim_end().splitn(4, '\x1f').collect();
    if fields.len() != 4 {
        return Err(ApiError::not_found("Invalid git show response"));
    }

    let output = run_git(
        &["show", "--format=", "--numstat", "--find-renames", &sha],
        &cwd,
    )
    .map_err(ApiError::not_found)?;
    let files: Vec<GitCommitFileInfo> = output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() != 3 {
                return None;
            }
            let additions = parts[0].parse().unwrap_or(0);
            let deletions = parts[1].parse().unwrap_or(0);
            let status = match (additions, deletions) {
                (0, deletions) if deletions > 0 => "D",
                (additions, 0) if additions > 0 => "A",
                _ => "M",
            };
            Some(GitCommitFileInfo {
                path: parts[2].to_string(),
                status: status.to_string(),
                additions,
                deletions,
            })
        })
        .collect();
    let total_additions = files.iter().map(|file| file.additions).sum();
    let total_deletions = files.iter().map(|file| file.deletions).sum();

    Ok(Json(GitShowResponse {
        sha: fields[0].to_string(),
        author: fields[1].to_string(),
        date: fields[2].to_string(),
        message: fields[3].trim_end().to_string(),
        files,
        total_additions,
        total_deletions,
    }))
}

async fn git_show_diff(
    State(state): State<Arc<AppState>>,
    Path(sha): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Vec<GitFileDiffResponse>>, ApiError> {
    let cwd = git_workspace(&state);
    if let Some(path) = params.get("path") {
        let output = run_git(&["show", "--pretty=format:", &sha, "--", path], &cwd)
            .map_err(ApiError::not_found)?;
        return Ok(Json(vec![GitFileDiffResponse {
            path: path.clone(),
            diff: output,
        }]));
    }

    let commit = git_show_files(State(state.clone()), Path(sha.clone()))
        .await
        .map_err(|error| ApiError::not_found(error.message))?
        .0;
    let mut diffs = Vec::with_capacity(commit.files.len());
    for file in commit.files {
        let output = run_git(&["show", "--pretty=format:", &sha, "--", &file.path], &cwd)
            .map_err(ApiError::not_found)?;
        diffs.push(GitFileDiffResponse {
            path: file.path,
            diff: output,
        });
    }
    Ok(Json(diffs))
}

async fn git_diff_file(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GitDiffFileParams>,
) -> Result<Json<GitFileDiffResponse>, ApiError> {
    let cwd = git_workspace(&state);
    let staged = params.staged.unwrap_or(false);
    let output = if staged {
        run_git(&["diff", "--cached", "--", &params.path], &cwd)
    } else {
        run_git(&["diff", "--", &params.path], &cwd)
    }
    .map_err(|e| ApiError::not_found(e))?;
    Ok(Json(GitFileDiffResponse {
        path: params.path.clone(),
        diff: output,
    }))
}

async fn git_commit(
    State(state): State<Arc<AppState>>,
    Json(body): Json<GitCommitRequest>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    if body.message.trim().is_empty() {
        return Err(ApiError::bad_request("Commit message cannot be empty"));
    }
    run_git(&["commit", "-m", &body.message], &cwd).map_err(|e| ApiError::bad_request(e))?;
    Ok(Json(GitMessageResponse {
        success: true,
        message: "Committed".to_string(),
    }))
}

async fn git_branch_create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<GitBranchCreateRequest>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    if body.checkout.unwrap_or(true) {
        run_git(&["checkout", "-b", &body.name], &cwd).map_err(|e| ApiError::bad_request(e))?;
    } else {
        run_git(&["branch", &body.name], &cwd).map_err(|e| ApiError::bad_request(e))?;
    }
    Ok(Json(GitMessageResponse {
        success: true,
        message: format!("Branch {} created", body.name),
    }))
}

async fn git_branch_delete(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    run_git(&["branch", "-d", &name], &cwd).map_err(|e| ApiError::bad_request(e))?;
    Ok(Json(GitMessageResponse {
        success: true,
        message: format!("Branch {} deleted", name),
    }))
}

async fn git_push(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    run_git(&["push"], &cwd).map_err(|e| ApiError::bad_request(e))?;
    Ok(Json(GitMessageResponse {
        success: true,
        message: "Pushed".to_string(),
    }))
}

async fn git_pull(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    run_git(&["pull"], &cwd).map_err(|e| ApiError::bad_request(e))?;
    Ok(Json(GitMessageResponse {
        success: true,
        message: "Pulled".to_string(),
    }))
}

#[derive(Deserialize)]
struct StashRequest {
    message: Option<String>,
}

async fn git_stash(
    State(state): State<Arc<AppState>>,
    body: Option<Json<StashRequest>>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    if let Some(Json(req)) = body {
        if let Some(msg) = req.message {
            run_git(&["stash", "push", "-m", &msg], &cwd).map_err(|e| ApiError::bad_request(e))?;
        } else {
            run_git(&["stash", "push"], &cwd).map_err(|e| ApiError::bad_request(e))?;
        }
    } else {
        run_git(&["stash", "push"], &cwd).map_err(|e| ApiError::bad_request(e))?;
    }
    Ok(Json(GitMessageResponse {
        success: true,
        message: "Stashed".to_string(),
    }))
}

async fn git_stash_pop(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    run_git(&["stash", "pop"], &cwd).map_err(|e| ApiError::bad_request(e))?;
    Ok(Json(GitMessageResponse {
        success: true,
        message: "Stash popped".to_string(),
    }))
}

fn workspace_input_path(raw_path: &str) -> Result<PathBuf, ApiError> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return Err(ApiError::bad_request("workspace path is required"));
    }
    let path = expand_tilde(StdPath::new(raw_path))
        .map_err(|error| ApiError::bad_request(format!("invalid workspace path: {error}")))?;
    if !path.is_absolute() {
        return Err(ApiError::bad_request(
            "workspace path must be absolute or start with ~/",
        ));
    }
    Ok(path)
}

fn workspace_completion_path(path: &StdPath, use_tilde: bool) -> String {
    if use_tilde {
        display_path_with_tilde(path)
    } else {
        path.to_string_lossy().to_string()
    }
}

async fn canonical_workspace_path(raw_path: &str) -> Result<PathBuf, ApiError> {
    let path = workspace_input_path(raw_path)?;
    let canonical = fs::canonicalize(&path)
        .await
        .map_err(|e| ApiError::not_found(format!("workspace directory not found: {e}")))?;
    let metadata = fs::metadata(&canonical)
        .await
        .map_err(|e| ApiError::not_found(format!("workspace directory not found: {e}")))?;
    if !metadata.is_dir() {
        return Err(ApiError::bad_request("workspace path must be a directory"));
    }
    Ok(canonical)
}

fn workspace_name(path: &StdPath) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

fn workspace_git_branch(path: &PathBuf) -> Option<String> {
    let branch = run_git(&["rev-parse", "--abbrev-ref", "HEAD"], path)
        .ok()?
        .trim()
        .to_string();
    if branch.is_empty() {
        return None;
    }
    if branch != "HEAD" {
        return Some(branch);
    }

    let short_sha = run_git(&["rev-parse", "--short", "HEAD"], path)
        .ok()?
        .trim()
        .to_string();
    (!short_sha.is_empty()).then(|| format!("@{short_sha}"))
}

async fn workspace_context(
    Query(query): Query<WorkspacePathQuery>,
) -> Result<Json<WorkspaceContextResponse>, ApiError> {
    let workspace_root = canonical_workspace_path(&query.path).await?;
    let git_path = workspace_root.clone();
    let git_branch = tokio::task::spawn_blocking(move || workspace_git_branch(&git_path))
        .await
        .map_err(|_| ApiError::internal("workspace git inspection failed"))?;
    Ok(Json(WorkspaceContextResponse {
        workspace_display: display_path_with_tilde(&workspace_root),
        workspace_name: workspace_name(&workspace_root),
        workspace_root: workspace_root.to_string_lossy().to_string(),
        git_branch,
    }))
}

async fn workspace_complete(
    Query(query): Query<WorkspacePathQuery>,
) -> Result<Json<WorkspaceCompletionResponse>, ApiError> {
    let raw_path = query.path.trim();
    let input = workspace_input_path(raw_path)?;
    let use_tilde = raw_path == "~" || raw_path.starts_with("~/");
    let navigation_parent = input
        .parent()
        .filter(|parent| *parent != input)
        .map(|parent| workspace_completion_path(parent, use_tilde));

    let (parent, prefix) = if raw_path == "~" || raw_path.ends_with('/') {
        (input, String::new())
    } else {
        let parent = input
            .parent()
            .map(StdPath::to_path_buf)
            .ok_or_else(|| ApiError::bad_request("workspace path has no parent directory"))?;
        let prefix = input
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        (parent, prefix)
    };
    let parent = match fs::canonicalize(parent).await {
        Ok(parent) => parent,
        Err(_) => {
            return Ok(Json(WorkspaceCompletionResponse {
                directories: vec![],
                parent: navigation_parent,
            }));
        }
    };
    let mut entries = match fs::read_dir(&parent).await {
        Ok(entries) => entries,
        Err(_) => {
            return Ok(Json(WorkspaceCompletionResponse {
                directories: vec![],
                parent: navigation_parent,
            }));
        }
    };
    let mut directories = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        ApiError::internal(format!("failed to read workspace directory: {error}"))
    })? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(&prefix) {
            continue;
        }
        let is_directory = entry
            .metadata()
            .await
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        if is_directory {
            directories.push(workspace_completion_path(&entry.path(), use_tilde));
        }
    }
    directories.sort_unstable_by_key(|path| path.to_lowercase());
    directories.truncate(50);
    Ok(Json(WorkspaceCompletionResponse {
        directories,
        parent: navigation_parent,
    }))
}

async fn get_workspace(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let workspace_root = state.runtime.workspace_root();
    let ws = workspace_root.to_string_lossy().to_string();
    let workspace_display = display_path_with_tilde(workspace_root);
    Json(serde_json::json!({
        "workspace_root": ws,
        "workspace_display": workspace_display
    }))
}

async fn get_init() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "prompt": "Analyze the project and create AGENTS.md" }))
}

async fn list_tools() -> Json<Vec<serde_json::Value>> {
    Json(vec![])
}

async fn list_skills(State(state): State<Arc<AppState>>) -> Json<SkillListResponse> {
    let catalog = state.runtime.skills();
    let skills = catalog
        .all()
        .iter()
        .map(|s| SkillDto {
            name: s.name.clone(),
            description: s.description.clone(),
            directory: s.directory.to_string_lossy().to_string(),
            location: s.location.to_string_lossy().to_string(),
            is_bundled: s.directory.to_string_lossy().starts_with("__builtin__"),
            companion_files: s
                .companion_files
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            content: s.content.clone(),
            document: s.document.clone(),
        })
        .collect();
    Json(SkillListResponse { skills })
}

async fn get_skill(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<SkillDto>, ApiError> {
    let catalog = state.runtime.skills();
    let s = catalog
        .get(&name)
        .ok_or_else(|| ApiError::not_found(format!("Skill '{name}' not found")))?;
    Ok(Json(SkillDto {
        name: s.name.clone(),
        description: s.description.clone(),
        directory: s.directory.to_string_lossy().to_string(),
        location: s.location.to_string_lossy().to_string(),
        is_bundled: s.directory.to_string_lossy().starts_with("__builtin__"),
        companion_files: s
            .companion_files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
        content: s.content.clone(),
        document: s.document.clone(),
    }))
}

async fn get_skill_file(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(query): Query<SkillFileQuery>,
) -> Result<Json<SkillFileResponse>, ApiError> {
    let catalog = state.runtime.skills();
    let rel_path = query.path.as_deref().unwrap_or("");
    let content = catalog
        .read_skill_file(&name, rel_path, 1_000_000)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(SkillFileResponse { content }))
}

fn normalized_api_type(value: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let api_type = match value.to_ascii_lowercase().as_str() {
        "openai_chat_completions" | "openai" | "chat" => ApiType::OpenAiChatCompletions,
        "openai_responses" | "responses" => ApiType::OpenAiResponses,
        "anthropic" | "claude" => ApiType::Anthropic,
        "google_gemini" | "gemini" | "google" => ApiType::GoogleGemini,
        _ => {
            return Err(ApiError::bad_request(format!(
                "unsupported api_type '{value}'"
            )));
        }
    };

    Ok(Some(api_type.as_str().to_owned()))
}

fn required_provider_field(value: &str, field: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ApiError::bad_request(format!("{field} is required")));
    }
    Ok(value.to_owned())
}

fn validate_provider_id(value: &str) -> Result<String, ApiError> {
    let provider_id = required_provider_field(value, "provider_id")?;
    if !provider_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(ApiError::bad_request(
            "provider_id may contain only letters, numbers, '-', '_' and '.'",
        ));
    }
    Ok(provider_id)
}

fn build_provider_config(
    request: &CreateProviderRequest,
) -> Result<(String, String, ProviderConfig), ApiError> {
    let provider_id = validate_provider_id(&request.provider_id)?;
    let display_name = required_provider_field(&request.display_name, "display_name")?;
    let base_url = required_provider_field(&request.base_url, "base_url")?;
    let api_type = normalized_api_type(request.api_type.as_deref())?;
    if let Some(user_agent) = request.user_agent.as_deref() {
        tidev_config::provider::validate_user_agent(user_agent)
            .map_err(|error| ApiError::bad_request(format!("invalid user_agent: {error}")))?;
    }
    let api_key = required_provider_field(&request.api_key, "api_key")?;

    if request.models.is_empty() {
        return Err(ApiError::bad_request(
            "at least one model is required for a provider",
        ));
    }

    let mut models = BTreeMap::new();
    for request_model in &request.models {
        let model_id = required_provider_field(&request_model.model_id, "model_id")?;
        let model_display_name =
            required_provider_field(&request_model.display_name, "model display_name")?;
        if request_model.context_window == 0 {
            return Err(ApiError::bad_request(format!(
                "model '{model_id}' context_window must be greater than zero"
            )));
        }
        if request_model.max_output_tokens == 0 {
            return Err(ApiError::bad_request(format!(
                "model '{model_id}' max_output_tokens must be greater than zero"
            )));
        }
        if request_model
            .temperature
            .is_some_and(|temperature| !temperature.is_finite())
        {
            return Err(ApiError::bad_request(format!(
                "model '{model_id}' temperature must be finite"
            )));
        }
        if models.contains_key(&model_id) {
            return Err(ApiError::conflict(format!(
                "duplicate model_id '{model_id}'"
            )));
        }

        models.insert(
            model_id,
            ModelConfig {
                display_name: model_display_name,
                context_window: request_model.context_window,
                max_output_tokens: request_model.max_output_tokens,
                api_type: normalized_api_type(request_model.api_type.as_deref())?,
                base_url: request_model
                    .base_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
                temperature: request_model.temperature,
                system_prompt: None,
                supports_streaming: request_model.supports_streaming,
                supports_images: request_model.supports_images,
                supports_parallel_tool_calls: request_model.supports_parallel_tool_calls,
                extra_body: None,
                request_model_id: request_model
                    .request_model_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
            },
        );
    }

    Ok((
        provider_id,
        api_key,
        ProviderConfig {
            display_name,
            base_url,
            api_type,
            user_agent: request.user_agent.clone(),
            models,
        },
    ))
}

fn provider_model_dto(model_id: &str, model: &ModelConfig) -> ProviderModelDto {
    ProviderModelDto {
        id: model_id.to_owned(),
        display_name: model.display_name.clone(),
        request_model_id: model.request_model_id.clone(),
        context_window: model.context_window,
        max_output_tokens: model.max_output_tokens,
        api_type: model.api_type.clone(),
        base_url: model.base_url.clone(),
        temperature: model.temperature,
        supports_images: model.supports_images,
        supports_streaming: model.supports_streaming,
        supports_parallel_tool_calls: model.supports_parallel_tool_calls,
    }
}

fn provider_dto(
    config: &tidev_config::AppConfig,
    auth: &tidev_config::AuthStore,
    provider_id: &str,
) -> Option<ProviderDto> {
    let provider = config.provider(provider_id)?;
    let is_bundled = config.bundled_providers.contains_key(provider_id);
    Some(ProviderDto {
        id: provider_id.to_owned(),
        display_name: provider.display_name.clone(),
        source: if is_bundled { "bundled" } else { "user" },
        can_delete: !is_bundled && config.providers.contains_key(provider_id),
        connected: auth.api_key(provider_id).is_some(),
        base_url: provider.base_url.clone(),
        api_type: provider.api_type.clone(),
        user_agent: provider.user_agent.clone(),
        models: provider
            .models
            .iter()
            .map(|(model_id, model)| provider_model_dto(model_id, model))
            .collect(),
    })
}

async fn list_providers(State(state): State<Arc<AppState>>) -> Json<ProvidersResponse> {
    let config = state.runtime.config();
    let auth = state.runtime.auth();
    let providers = config
        .provider_ids()
        .iter()
        .filter_map(|provider_id| provider_dto(&config, &auth, provider_id))
        .collect();
    Json(ProvidersResponse { providers })
}

async fn create_provider(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateProviderRequest>,
) -> Result<Json<ProviderDto>, ApiError> {
    let (provider_id, api_key, provider) = build_provider_config(&request)?;
    let config_before = state.runtime.config();
    if config_before.provider_exists(&provider_id) {
        return Err(ApiError::conflict(format!(
            "provider '{provider_id}' already exists"
        )));
    }

    let mut update_error = None;
    let provider_id_for_update = provider_id.clone();
    let provider_for_update = provider.clone();
    state.runtime.update_config(|config| {
        update_error = config
            .set_user_provider(provider_id_for_update, provider_for_update)
            .err();
    });
    if let Some(error) = update_error {
        return Err(error.into());
    }
    if let Err(error) = state.runtime.save_config() {
        state
            .runtime
            .update_config(|config| *config = config_before.clone());
        return Err(error.into());
    }

    let auth_before = state.runtime.auth();
    state
        .runtime
        .update_auth(|auth| auth.set_api_key(&provider_id, &api_key));
    if let Err(error) = state.runtime.save_auth() {
        state.runtime.update_auth(|auth| *auth = auth_before);
        state
            .runtime
            .update_config(|config| *config = config_before);
        let _ = state.runtime.save_config();
        return Err(error.into());
    }

    let config = state.runtime.config();
    let auth = state.runtime.auth();
    let provider = provider_dto(&config, &auth, &provider_id)
        .ok_or_else(|| ApiError::internal("created provider could not be loaded"))?;
    Ok(Json(provider))
}

async fn delete_provider(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
) -> Result<Json<ProviderMutationResponse>, ApiError> {
    let config_before = state.runtime.config();
    if config_before.bundled_providers.contains_key(&provider_id) {
        return Err(ApiError::forbidden(format!(
            "bundled provider '{provider_id}' cannot be deleted"
        )));
    }
    if !config_before.providers.contains_key(&provider_id) {
        return Err(ApiError::not_found(format!(
            "provider '{provider_id}' not found"
        )));
    }

    let active_provider_id = state.runtime.active_provider_id();
    if config_before.default_provider == provider_id || active_provider_id == provider_id {
        return Err(ApiError::conflict(format!(
            "provider '{provider_id}' is currently active or configured as the default"
        )));
    }
    if config_before.agent.default_subagent_provider == provider_id
        || config_before.agent.models.values().any(|model| {
            model
                .split_once('/')
                .is_some_and(|(provider, _)| provider == provider_id)
        })
    {
        return Err(ApiError::conflict(format!(
            "provider '{provider_id}' is referenced by agent configuration"
        )));
    }

    let mut update_error = None;
    let provider_id_for_update = provider_id.clone();
    state.runtime.update_config(|config| {
        update_error = config.remove_user_provider(&provider_id_for_update).err();
    });
    if let Some(error) = update_error {
        return Err(error.into());
    }
    if let Err(error) = state.runtime.save_config() {
        state
            .runtime
            .update_config(|config| *config = config_before.clone());
        return Err(error.into());
    }

    let auth_before = state.runtime.auth();
    state.runtime.update_auth(|auth| {
        auth.remove_api_key(&provider_id);
    });
    if let Err(error) = state.runtime.save_auth() {
        state.runtime.update_auth(|auth| *auth = auth_before);
        state
            .runtime
            .update_config(|config| *config = config_before);
        let _ = state.runtime.save_config();
        return Err(error.into());
    }

    Ok(Json(ProviderMutationResponse {
        success: true,
        connected: None,
    }))
}

async fn connect_provider(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Json(request): Json<ConnectProviderRequest>,
) -> Result<Json<ProviderMutationResponse>, ApiError> {
    if state.runtime.config().provider(&provider_id).is_none() {
        return Err(ApiError::not_found(format!(
            "provider '{provider_id}' not found"
        )));
    }
    let api_key = required_provider_field(&request.api_key, "api_key")?;
    let auth_before = state.runtime.auth();
    state
        .runtime
        .update_auth(|auth| auth.set_api_key(&provider_id, &api_key));
    if let Err(error) = state.runtime.save_auth() {
        state.runtime.update_auth(|auth| *auth = auth_before);
        return Err(error.into());
    }
    Ok(Json(ProviderMutationResponse {
        success: true,
        connected: Some(true),
    }))
}

async fn disconnect_provider(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
) -> Result<Json<ProviderMutationResponse>, ApiError> {
    if state.runtime.config().provider(&provider_id).is_none() {
        return Err(ApiError::not_found(format!(
            "provider '{provider_id}' not found"
        )));
    }
    let auth_before = state.runtime.auth();
    state.runtime.update_auth(|auth| {
        auth.remove_api_key(&provider_id);
    });
    if let Err(error) = state.runtime.save_auth() {
        state.runtime.update_auth(|auth| *auth = auth_before);
        return Err(error.into());
    }
    Ok(Json(ProviderMutationResponse {
        success: true,
        connected: Some(false),
    }))
}

async fn get_default_model(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let model = state.runtime.active_model();
    Json(serde_json::json!({
        "provider_id": model.provider_id,
        "provider_display_name": model.provider_display_name,
        "model_id": model.model_id,
        "model_display_name": model.display_name,
    }))
}

async fn set_default_model(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if let (Some(pid), Some(mid)) = (
        body.get("provider_id").and_then(|v| v.as_str()),
        body.get("model_id").and_then(|v| v.as_str()),
    ) {
        if let Ok(mut m) = state
            .runtime
            .config()
            .resolve_active_model(&state.runtime.auth())
        {
            // Best-effort: update active model id
            m.provider_id = pid.to_string();
            m.model_id = mid.to_string();
            state.runtime.set_active_model(m);
        }
    }
    let model = state.runtime.active_model();
    Ok(Json(serde_json::json!({
        "success": true,
        "provider_id": model.provider_id,
        "model_id": model.model_id,
        "provider_display_name": model.provider_display_name,
        "model_display_name": model.display_name,
    })))
}

async fn get_agent_models(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let model = state.runtime.active_model();
    let config = state.runtime.config();
    Json(serde_json::json!({
        "default_model": {
            "provider_id": model.provider_id,
            "model_id": model.model_id,
            "provider_display_name": model.provider_display_name,
            "model_display_name": model.display_name,
        },
        "agent_models": config.agent.models,
        "agent_thinking_levels": config.agent.thinking_levels,
    }))
}

async fn set_agent_model(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SetAgentModelRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let agent_type = request.agent_type.trim().to_ascii_lowercase();
    if AgentType::parse(&agent_type).is_none() {
        return Err(ApiError::bad_request(format!(
            "unknown subagent type '{}', expected explorer, librarian, oracle, or fixer",
            request.agent_type
        )));
    }

    let model_str = request.model_str.trim().to_string();
    let thinking_level = request
        .thinking_level
        .unwrap_or_default()
        .trim()
        .to_string();
    state.runtime.update_config(|config| {
        if model_str.is_empty() {
            config.agent.models.remove(&agent_type);
        } else {
            config.agent.models.insert(agent_type.clone(), model_str);
        }

        if thinking_level.is_empty() {
            config.agent.thinking_levels.remove(&agent_type);
        } else {
            config
                .agent
                .thinking_levels
                .insert(agent_type.clone(), thinking_level);
        }
    });
    state.runtime.save_config()?;
    Ok(Json(serde_json::json!({ "success": true })))
}

async fn get_subagent_config(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let config = state.runtime.config();
    Json(serde_json::json!({ "enabled": config.subagent.enabled }))
}

async fn set_subagent_config(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SetSubagentEnabledRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state.runtime.update_config(|config| {
        config.subagent.enabled = request.enabled;
    });
    state.runtime.save_config()?;
    Ok(Json(serde_json::json!({
        "success": true,
        "enabled": request.enabled,
    })))
}

async fn get_memory_model() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "role": "memory", "model_str": null }))
}

async fn set_memory_model() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "success": true }))
}

async fn get_model_thinking_level() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "thinking_level": "normal", "thinking_options": [] }))
}

async fn set_model_thinking_level() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "success": true }))
}

#[derive(Default)]
struct UsageTotals {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    total_tokens: u64,
    request_count: u64,
}

impl UsageTotals {
    fn add(&mut self, record: &tidev_core::UsageRecord) {
        self.input_tokens += record.input_tokens;
        self.output_tokens += record.output_tokens;
        self.cache_read_tokens += record.cache_read_tokens;
        self.cache_write_tokens += record.cache_write_tokens;
        self.total_tokens += record.total_tokens;
        self.request_count += 1;
    }

    fn cache_hit_rate(&self) -> f64 {
        if self.input_tokens == 0 {
            0.0
        } else {
            self.cache_read_tokens as f64 / self.input_tokens as f64 * 100.0
        }
    }
}

struct UsageGroup {
    provider_id: String,
    provider_display_name: String,
    model_id: String,
    model_display_name: String,
    totals: UsageTotals,
}

struct ProviderGroup {
    provider_id: String,
    provider_display_name: String,
    totals: UsageTotals,
}

fn stats_session_count(records: &[tidev_core::UsageRecord]) -> u64 {
    records
        .iter()
        .map(|record| record.session_id.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len() as u64
}

fn parse_stats_time(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn filter_usage_records(
    records: Vec<tidev_core::UsageRecord>,
    query: &StatsQuery,
) -> Vec<tidev_core::UsageRecord> {
    let start = parse_stats_time(query.start.as_deref());
    let end = parse_stats_time(query.end.as_deref());
    if start.is_none() && end.is_none() {
        return records;
    }
    records
        .into_iter()
        .filter(|record| {
            let timestamp = parse_stats_time(Some(&record.created_at));
            timestamp.is_some_and(|timestamp| {
                start.is_none_or(|start| timestamp >= start)
                    && end.is_none_or(|end| timestamp < end)
            })
        })
        .collect()
}

fn load_stats_records(
    state: &AppState,
    query: &StatsQuery,
) -> Result<Vec<tidev_core::UsageRecord>, ApiError> {
    let start = parse_stats_time(query.start.as_deref());
    let end = parse_stats_time(query.end.as_deref());
    let records = state
        .runtime
        .session_manager()
        .store()
        .load_usage_records_in_range(start, end)?;
    Ok(filter_usage_records(records, query))
}

fn stats_summary_json(records: &[tidev_core::UsageRecord]) -> serde_json::Value {
    let mut totals = UsageTotals::default();
    for record in records {
        totals.add(record);
    }
    serde_json::json!({
        "total_input_tokens": totals.input_tokens,
        "total_output_tokens": totals.output_tokens,
        "total_cache_read_tokens": totals.cache_read_tokens,
        "total_cache_write_tokens": totals.cache_write_tokens,
        "total_tokens": totals.total_tokens,
        "total_requests": totals.request_count,
        "cache_hit_rate": totals.cache_hit_rate(),
        "total_sessions": stats_session_count(records),
        "first_usage_date": records.first().map(|record| record.created_at.clone()),
    })
}

fn stats_bucket(created_at: &str, granularity: &str) -> String {
    let Ok(parsed) = DateTime::parse_from_rfc3339(created_at) else {
        return created_at.to_owned();
    };
    let utc = parsed.with_timezone(&Utc);
    let bucket = match granularity {
        "hour" => utc
            .with_minute(0)
            .and_then(|value| value.with_second(0))
            .and_then(|value| value.with_nanosecond(0)),
        "day" => Some(
            Utc.from_utc_datetime(
                &utc.date_naive()
                    .and_hms_opt(0, 0, 0)
                    .expect("midnight is always a valid time"),
            ),
        ),
        "week" => {
            let date =
                utc.date_naive() - Duration::days(utc.weekday().num_days_from_monday() as i64);
            Some(
                Utc.from_utc_datetime(
                    &date
                        .and_hms_opt(0, 0, 0)
                        .expect("midnight is always a valid time"),
                ),
            )
        }
        "month" => Some(
            Utc.from_utc_datetime(
                &NaiveDate::from_ymd_opt(utc.year(), utc.month(), 1)
                    .expect("the first day of a month is always valid")
                    .and_hms_opt(0, 0, 0)
                    .expect("midnight is always a valid time"),
            ),
        ),
        _ => Some(utc),
    };
    bucket
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| created_at.to_owned())
}

fn valid_granularity(value: Option<&str>) -> String {
    match value {
        Some("hour") | Some("day") | Some("week") | Some("month") => value.unwrap().to_owned(),
        _ => "hour".to_owned(),
    }
}

async fn stats_summary(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let records = load_stats_records(&state, &query)?;
    Ok(Json(stats_summary_json(&records)))
}

fn stats_timeseries_json(
    records: &[tidev_core::UsageRecord],
    granularity: &str,
) -> serde_json::Value {
    let mut buckets: HashMap<String, UsageTotals> = HashMap::new();
    for record in records {
        buckets
            .entry(stats_bucket(&record.created_at, granularity))
            .or_default()
            .add(record);
    }
    let mut entries = buckets.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let entries = entries
        .into_iter()
        .map(|(time_bucket, totals)| {
            serde_json::json!({
                "time_bucket": time_bucket,
                "input_tokens": totals.input_tokens,
                "output_tokens": totals.output_tokens,
                "cache_read_tokens": totals.cache_read_tokens,
                "cache_write_tokens": totals.cache_write_tokens,
                "total_tokens": totals.total_tokens,
                "request_count": totals.request_count,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "granularity": granularity,
        "entries": entries,
        "summary": stats_summary_json(records),
    })
}

async fn stats_timeseries(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let records = load_stats_records(&state, &query)?;
    let granularity = valid_granularity(query.granularity.as_deref());
    Ok(Json(stats_timeseries_json(&records, &granularity)))
}

fn stats_models_json(records: &[tidev_core::UsageRecord]) -> serde_json::Value {
    let mut groups: HashMap<(String, String), UsageGroup> = HashMap::new();
    for record in records {
        let key = (record.provider_id.clone(), record.model_id.clone());
        let group = groups.entry(key).or_insert_with(|| UsageGroup {
            provider_id: record.provider_id.clone(),
            provider_display_name: record.provider_display_name.clone(),
            model_id: record.model_id.clone(),
            model_display_name: record.model_display_name.clone(),
            totals: UsageTotals::default(),
        });
        group.totals.add(record);
    }
    let mut entries = groups.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| right.totals.total_tokens.cmp(&left.totals.total_tokens));
    serde_json::json!({
        "entries": entries.into_iter().map(|group| serde_json::json!({
            "provider_id": group.provider_id,
            "provider_display_name": group.provider_display_name,
            "model_id": group.model_id,
            "model_display_name": group.model_display_name,
            "input_tokens": group.totals.input_tokens,
            "output_tokens": group.totals.output_tokens,
            "cache_read_tokens": group.totals.cache_read_tokens,
            "cache_write_tokens": group.totals.cache_write_tokens,
            "total_tokens": group.totals.total_tokens,
            "request_count": group.totals.request_count,
        })).collect::<Vec<_>>(),
    })
}

async fn stats_models(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let records = load_stats_records(&state, &query)?;
    Ok(Json(stats_models_json(&records)))
}

fn stats_providers_json(records: &[tidev_core::UsageRecord]) -> serde_json::Value {
    let mut groups: HashMap<String, ProviderGroup> = HashMap::new();
    for record in records {
        let group = groups
            .entry(record.provider_id.clone())
            .or_insert_with(|| ProviderGroup {
                provider_id: record.provider_id.clone(),
                provider_display_name: record.provider_display_name.clone(),
                totals: UsageTotals::default(),
            });
        group.totals.add(record);
    }
    let mut entries = groups.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| right.totals.total_tokens.cmp(&left.totals.total_tokens));
    serde_json::json!({
        "entries": entries.into_iter().map(|group| serde_json::json!({
            "provider_id": group.provider_id,
            "provider_display_name": group.provider_display_name,
            "input_tokens": group.totals.input_tokens,
            "output_tokens": group.totals.output_tokens,
            "cache_read_tokens": group.totals.cache_read_tokens,
            "cache_write_tokens": group.totals.cache_write_tokens,
            "total_tokens": group.totals.total_tokens,
            "request_count": group.totals.request_count,
        })).collect::<Vec<_>>(),
    })
}

async fn stats_providers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let records = load_stats_records(&state, &query)?;
    Ok(Json(stats_providers_json(&records)))
}

fn stats_sessions_json(
    records: &[tidev_core::UsageRecord],
    limit: i64,
    offset: i64,
) -> serde_json::Value {
    let mut groups: HashMap<String, (tidev_core::UsageRecord, UsageTotals)> = HashMap::new();
    for record in records {
        let entry = groups
            .entry(record.session_id.clone())
            .or_insert_with(|| (record.clone(), UsageTotals::default()));
        entry.1.add(record);
    }
    let mut entries = groups.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| right.1.total_tokens.cmp(&left.1.total_tokens));
    let total = entries.len();
    let offset = offset.max(0) as usize;
    let limit = limit.clamp(1, 200) as usize;
    let entries = entries
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(record, totals)| {
            serde_json::json!({
                "session_id": record.session_id,
                "title": record.title,
                "provider_id": record.provider_id,
                "model_id": record.model_id,
                "model_display_name": record.model_display_name,
                "message_count": totals.request_count,
                "input_tokens": totals.input_tokens,
                "output_tokens": totals.output_tokens,
                "cache_read_tokens": totals.cache_read_tokens,
                "cache_write_tokens": totals.cache_write_tokens,
                "total_tokens": totals.total_tokens,
                "created_at": record.session_created_at,
                "updated_at": record.session_updated_at,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({ "entries": entries, "total": total })
}

async fn stats_sessions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let records = load_stats_records(&state, &query)?;
    Ok(Json(stats_sessions_json(
        &records,
        query.limit.unwrap_or(10),
        query.offset.unwrap_or(0),
    )))
}

const STATS_ACTIVITY_DAY_COUNT: i64 = 365;

fn stats_activity_level(value: u64, scale: u64) -> u8 {
    if value == 0 || scale == 0 {
        return 0;
    }
    let normalized = ((value as f64 + 1.0).ln() / (scale as f64 + 1.0).ln()).min(1.0);
    (normalized * 4.0).ceil().clamp(1.0, 4.0) as u8
}

fn stats_activity_response(
    days: Vec<tidev_core::UsageActivityDay>,
    end_date: chrono::NaiveDate,
) -> StatsActivityResponse {
    let start_date = end_date - Duration::days(STATS_ACTIVITY_DAY_COUNT - 1);
    let mut days_by_date = days
        .into_iter()
        .map(|day| (day.date.clone(), day))
        .collect::<HashMap<_, _>>();
    let mut values = days_by_date
        .values()
        .map(|day| day.request_count)
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    values.sort_unstable();
    let scale = values
        .get(((values.len() as f64 * 0.95).ceil() as usize).saturating_sub(1))
        .copied()
        .unwrap_or(0)
        .max(1);

    let mut total_requests = 0;
    let mut total_tokens = 0;
    let mut cells = Vec::with_capacity(STATS_ACTIVITY_DAY_COUNT as usize);
    for offset in 0..STATS_ACTIVITY_DAY_COUNT {
        let date = start_date + Duration::days(offset);
        let date_key = date.format("%Y-%m-%d").to_string();
        let day = days_by_date.remove(&date_key);
        let request_count = day.as_ref().map_or(0, |day| day.request_count);
        let token_count = day.as_ref().map_or(0, |day| day.total_tokens);
        total_requests += request_count;
        total_tokens += token_count;
        cells.push(StatsActivityCell {
            date: date_key,
            request_count,
            total_tokens: token_count,
            level: stats_activity_level(request_count, scale),
        });
    }

    StatsActivityResponse {
        start_date: start_date.format("%Y-%m-%d").to_string(),
        end_date: end_date.format("%Y-%m-%d").to_string(),
        total_requests,
        total_tokens,
        cells,
    }
}

async fn stats_activity(
    State(state): State<Arc<AppState>>,
) -> Result<Json<StatsActivityResponse>, ApiError> {
    let end_date = Utc::now().date_naive();
    let start_date = end_date - Duration::days(STATS_ACTIVITY_DAY_COUNT - 1);
    let start = Utc.from_utc_datetime(
        &start_date
            .and_hms_opt(0, 0, 0)
            .expect("midnight is always valid"),
    );
    let end = Utc.from_utc_datetime(
        &(end_date + Duration::days(1))
            .and_hms_opt(0, 0, 0)
            .expect("midnight is always valid"),
    );
    let days = state
        .runtime
        .session_manager()
        .store()
        .load_usage_activity_days(start, end)?;
    Ok(Json(stats_activity_response(days, end_date)))
}

const STATS_MODEL_MIX_LIMIT: u64 = 5;

fn stats_insight_granularity(query: &StatsQuery) -> String {
    let granularity = valid_granularity(query.granularity.as_deref());
    let start = parse_stats_time(query.start.as_deref());
    let end = parse_stats_time(query.end.as_deref());
    let range_exceeds_31_days = start
        .zip(end)
        .is_some_and(|(start, end)| end - start > Duration::days(31));
    if granularity == "hour" && (start.is_none() || end.is_none() || range_exceeds_31_days) {
        "day".to_owned()
    } else {
        granularity
    }
}

fn stats_rhythm_response(cells: Vec<tidev_core::UsageRhythmCell>) -> StatsRhythmResponse {
    let mut cells_by_time = cells
        .into_iter()
        .map(|cell| ((cell.weekday, cell.hour), cell))
        .collect::<HashMap<_, _>>();
    let mut values = cells_by_time
        .values()
        .map(|cell| cell.request_count)
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    values.sort_unstable();
    let scale = values
        .get(((values.len() as f64 * 0.95).ceil() as usize).saturating_sub(1))
        .copied()
        .unwrap_or(0)
        .max(1);

    let mut response_cells = Vec::with_capacity(7 * 24);
    for weekday in 0..7 {
        for hour in 0..24 {
            let cell = cells_by_time.remove(&(weekday, hour));
            let request_count = cell.as_ref().map_or(0, |cell| cell.request_count);
            response_cells.push(StatsRhythmCell {
                weekday,
                hour,
                request_count,
                total_tokens: cell.as_ref().map_or(0, |cell| cell.total_tokens),
                level: stats_activity_level(request_count, scale),
            });
        }
    }
    StatsRhythmResponse {
        cells: response_cells,
    }
}

fn stats_model_mix_response(
    buckets: Vec<tidev_core::UsageModelMixBucket>,
) -> StatsModelMixResponse {
    #[derive(Debug)]
    struct SeriesMeta {
        provider_display_name: String,
        model_display_name: String,
        is_other: bool,
        total_tokens: u64,
    }

    let mut series_by_key = BTreeMap::<String, SeriesMeta>::new();
    let mut shares_by_time = BTreeMap::<String, BTreeMap<String, u64>>::new();
    for bucket in buckets {
        let key = if bucket.is_other {
            "other".to_owned()
        } else {
            format!("{}:{}", bucket.provider_id, bucket.model_id)
        };
        let series = series_by_key.entry(key.clone()).or_insert(SeriesMeta {
            provider_display_name: bucket.provider_display_name.clone(),
            model_display_name: bucket.model_display_name.clone(),
            is_other: bucket.is_other,
            total_tokens: 0,
        });
        series.total_tokens += bucket.total_tokens;
        *shares_by_time
            .entry(bucket.time_bucket)
            .or_default()
            .entry(key)
            .or_default() += bucket.total_tokens;
    }

    let mut series = series_by_key.into_iter().collect::<Vec<_>>();
    series.sort_by(|(left_key, left), (right_key, right)| {
        left.is_other
            .cmp(&right.is_other)
            .then_with(|| right.total_tokens.cmp(&left.total_tokens))
            .then_with(|| left_key.cmp(right_key))
    });
    let series = series
        .into_iter()
        .map(|(key, series)| StatsModelMixSeries {
            key,
            provider_display_name: series.provider_display_name,
            model_display_name: series.model_display_name,
            is_other: series.is_other,
        })
        .collect::<Vec<_>>();
    let points = shares_by_time
        .into_iter()
        .map(|(time_bucket, totals)| {
            let total_tokens = totals.values().sum::<u64>();
            let shares = totals
                .into_iter()
                .map(|(key, value)| {
                    let share = if total_tokens == 0 {
                        0.0
                    } else {
                        value as f64 / total_tokens as f64 * 100.0
                    };
                    (key, share)
                })
                .collect();
            StatsModelMixPoint {
                time_bucket,
                shares,
            }
        })
        .collect();
    StatsModelMixResponse { series, points }
}

async fn stats_insights(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<StatsInsightsResponse>, ApiError> {
    let start = parse_stats_time(query.start.as_deref());
    let end = parse_stats_time(query.end.as_deref());
    let granularity = stats_insight_granularity(&query);
    let store = state.runtime.session_manager().store();
    let active_sessions = store
        .load_usage_active_sessions(start, end, &granularity)?
        .into_iter()
        .map(|entry| StatsActiveSessionPoint {
            time_bucket: entry.time_bucket,
            active_sessions: entry.active_sessions,
        })
        .collect();
    let rhythm = stats_rhythm_response(store.load_usage_rhythm(start, end)?);
    let model_mix = stats_model_mix_response(store.load_usage_model_mix(
        start,
        end,
        &granularity,
        STATS_MODEL_MIX_LIMIT,
    )?);
    let request_size_distribution = store
        .load_usage_request_size_distribution(start, end)?
        .into_iter()
        .map(|bucket| StatsRequestSizeBucket {
            lower_bound: bucket.lower_bound,
            upper_bound: bucket.upper_bound,
            request_count: bucket.request_count,
            total_tokens: bucket.total_tokens,
        })
        .collect();

    Ok(Json(StatsInsightsResponse {
        granularity,
        active_sessions,
        rhythm,
        model_mix,
        request_size_distribution,
    }))
}

async fn stats_overview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let records = load_stats_records(&state, &query)?;
    let granularity = valid_granularity(query.granularity.as_deref());
    Ok(Json(serde_json::json!({
        "summary": stats_summary_json(&records),
        "timeseries": stats_timeseries_json(&records, &granularity),
        "models": stats_models_json(&records),
        "providers": stats_providers_json(&records),
        "sessions": stats_sessions_json(
            &records,
            query.limit.unwrap_or(10),
            query.offset.unwrap_or(0),
        ),
    })))
}

async fn system_restart(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    state.cancel.cancel();
    Json(serde_json::json!({ "success": true }))
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

async fn list_mcp_servers(State(state): State<Arc<AppState>>) -> Json<Vec<McpServerDto>> {
    let mcp = state.runtime.mcp_manager();
    let summaries = mcp.summaries();
    let all_tools = mcp.all_definitions();

    let mut dtos = Vec::new();
    for summary in summaries {
        let config = mcp.server_config(&summary.name);
        let is_disabled = summary.disabled || config.as_ref().is_some_and(|c| c.is_disabled());
        let (status_str, error_str) = match &summary.status {
            tidev_core::mcp::McpConnectionStatus::Connected => ("connected", None),
            tidev_core::mcp::McpConnectionStatus::Connecting => ("connecting", None),
            tidev_core::mcp::McpConnectionStatus::Disconnected => {
                if is_disabled {
                    ("disabled", None)
                } else {
                    ("disconnected", None)
                }
            }
            tidev_core::mcp::McpConnectionStatus::Failed(err) => ("failed", Some(err.clone())),
        };

        let tools = all_tools
            .iter()
            .filter(|tool| {
                tool.mcp_target()
                    .is_some_and(|(server, _)| server == summary.name)
            })
            .map(|tool| {
                let (_, raw_tool_name) = tool.mcp_target().unwrap_or(("", tool.name.as_str()));
                McpToolDto {
                    name: raw_tool_name.to_string(),
                    description: tool.description.clone(),
                    parameters: tool.parameters.clone(),
                }
            })
            .collect();

        dtos.push(McpServerDto {
            name: summary.name,
            kind: summary.kind,
            status: status_str.to_string(),
            error: error_str,
            disabled: is_disabled,
            config,
            tools,
        });
    }

    Json(dtos)
}

async fn upsert_mcp_server(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UpsertMcpServerRequest>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::bad_request("Server name cannot be empty"));
    }

    let mcp = state.runtime.mcp_manager().clone();
    if let Some(ref orig) = request.original_name {
        if orig != &name {
            let _ = mcp.remove_server(orig).await;
            state.runtime.update_config(|cfg| {
                cfg.mcp.servers.remove(orig);
            });
        }
    }

    mcp.upsert_server(name.clone(), request.config.clone())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state.runtime.update_config(|cfg| {
        cfg.mcp.servers.insert(name, request.config);
    });
    state.runtime.save_config()?;

    Ok(Json(AcceptedResponse { accepted: true }))
}

async fn delete_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state
        .runtime
        .mcp_manager()
        .remove_server(&name)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state.runtime.update_config(|cfg| {
        cfg.mcp.servers.remove(&name);
    });
    state.runtime.save_config()?;

    Ok(Json(AcceptedResponse { accepted: true }))
}

async fn connect_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    let mut target_config = None;
    state.runtime.update_config(|cfg| {
        if let Some(srv) = cfg.mcp.servers.get_mut(&name) {
            srv.set_disabled(false);
            target_config = Some(srv.clone());
        }
    });
    let _ = state.runtime.save_config();

    if let Some(config) = target_config {
        let _ = state
            .runtime
            .mcp_manager()
            .upsert_server(name.clone(), config)
            .await;
    }

    state
        .runtime
        .mcp_manager()
        .refresh_server(&name)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

async fn disconnect_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    let mut target_config = None;
    state.runtime.update_config(|cfg| {
        if let Some(srv) = cfg.mcp.servers.get_mut(&name) {
            srv.set_disabled(true);
            target_config = Some(srv.clone());
        }
    });
    let _ = state.runtime.save_config();

    if let Some(config) = target_config {
        let _ = state
            .runtime
            .mcp_manager()
            .upsert_server(name.clone(), config)
            .await;
    }

    state
        .runtime
        .mcp_manager()
        .disconnect_server(&name)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

async fn refresh_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state
        .runtime
        .mcp_manager()
        .refresh_server(&name)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

pub(crate) fn configured_auth_token(state: &AppState) -> Option<String> {
    state
        .runtime
        .auth()
        .web
        .auth_token
        .filter(|token| !token.trim().is_empty())
}

pub(crate) fn request_auth_token(headers: &HeaderMap, uri: &Uri) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_prompt_request_with_lowercase_mode() {
        let json_data = r#"{"content":"hello","mode":"plan"}"#;
        let req: PromptRequest = serde_json::from_str(json_data).unwrap();
        assert_eq!(req.content, "hello");
        assert_eq!(req.mode, Some(Mode::Plan));

        let json_data_build = r#"{"content":"run","mode":"build"}"#;
        let req_build: PromptRequest = serde_json::from_str(json_data_build).unwrap();
        assert_eq!(req_build.content, "run");
        assert_eq!(req_build.mode, Some(Mode::Build));
        assert!(req_build.attachments.is_empty());
    }

    #[test]
    fn prompt_image_attachment_request_preserves_bytes_and_size() {
        let request: PromptImageAttachmentRequest = serde_json::from_str(
            r#"{"type":"image","filename":"capture.png","mime":"image/png","data":[1,2,3]}"#,
        )
        .unwrap();

        let attachment = request.into_message_attachment().unwrap();
        match attachment {
            MessageAttachment::Image {
                filename,
                mime,
                data,
                file_size,
            } => {
                assert_eq!(filename, "capture.png");
                assert_eq!(mime, "image/png");
                assert_eq!(data, vec![1, 2, 3]);
                assert_eq!(file_size, 3);
            }
            other => panic!("expected image attachment, got {other:?}"),
        }
    }

    #[test]
    fn prompt_image_attachment_request_rejects_non_image_mime() {
        let request = PromptImageAttachmentRequest {
            attachment_type: "image".to_owned(),
            filename: "capture.png".to_owned(),
            mime: "text/plain".to_owned(),
            data: vec![1],
        };

        assert!(request.into_message_attachment().is_err());
    }

    #[test]
    fn workspace_path_expands_home_shortcut() {
        let home = workspace_input_path("~").unwrap();
        assert!(home.is_absolute());
        assert_eq!(workspace_input_path("~/tidev").unwrap(), home.join("tidev"));
        assert!(workspace_input_path("tidev").is_err());
    }

    #[test]
    fn provider_request_normalizes_api_type_and_model_defaults() {
        let request: CreateProviderRequest = serde_json::from_value(serde_json::json!({
            "provider_id": "custom",
            "display_name": "Custom",
            "base_url": "https://example.com/v1",
            "api_type": "openai",
            "user_agent": "gateway-client/1.0",
            "api_key": "secret",
            "models": [{
                "model_id": "model",
                "display_name": "Model",
                "context_window": 128000,
                "max_output_tokens": 16000,
                "temperature": 0.7
            }]
        }))
        .unwrap();

        let (provider_id, api_key, provider) = build_provider_config(&request).unwrap();
        assert_eq!(provider_id, "custom");
        assert_eq!(api_key, "secret");
        assert_eq!(
            provider.api_type.as_deref(),
            Some("openai_chat_completions")
        );
        assert_eq!(provider.user_agent.as_deref(), Some("gateway-client/1.0"));
        assert!(provider.models["model"].supports_streaming);
        assert!(provider.models["model"].supports_parallel_tool_calls);
    }

    #[test]
    fn provider_request_rejects_invalid_id_and_duplicate_models() {
        let invalid_id: CreateProviderRequest = serde_json::from_value(serde_json::json!({
            "provider_id": "custom/provider",
            "display_name": "Custom",
            "base_url": "https://example.com/v1",
            "api_key": "secret",
            "models": [{
                "model_id": "model",
                "display_name": "Model",
                "context_window": 128000,
                "max_output_tokens": 16000
            }]
        }))
        .unwrap();
        assert!(build_provider_config(&invalid_id).is_err());

        let duplicate_models: CreateProviderRequest = serde_json::from_value(serde_json::json!({
            "provider_id": "custom",
            "display_name": "Custom",
            "base_url": "https://example.com/v1",
            "api_key": "secret",
            "models": [
                {
                    "model_id": "model",
                    "display_name": "Model",
                    "context_window": 128000,
                    "max_output_tokens": 16000
                },
                {
                    "model_id": "model",
                    "display_name": "Model 2",
                    "context_window": 128000,
                    "max_output_tokens": 16000
                }
            ]
        }))
        .unwrap();
        assert!(build_provider_config(&duplicate_models).is_err());
    }

    #[test]
    fn bundled_provider_is_never_deletable_even_without_a_key() {
        let config = tidev_config::AppConfig::default();
        let auth = tidev_config::AuthStore::default();
        let provider = provider_dto(&config, &auth, "deepseek").unwrap();
        assert_eq!(provider.source, "bundled");
        assert!(!provider.can_delete);
    }
}
