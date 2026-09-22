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
use axum::routing::{delete, get, post, put};
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
use tidev_utils::build_info;
use tidev_utils::path::{display_path_with_tilde, expand_tilde};
use tokio::fs;
use tokio::sync::mpsc::UnboundedReceiver;
use uuid::Uuid;

use crate::frontend::FrontendMode;

mod filesystem;
mod git;
mod instructions;
pub(crate) mod mcp;
mod providers;
mod session;
mod stats;
mod workspace;

use filesystem::*;
use git::*;
use instructions::*;
use mcp::*;
use providers::*;
use session::*;
use stats::*;
use workspace::*;

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

#[derive(Debug, Serialize)]
struct BuildInfoResponse {
    version: &'static str,
    commit: &'static str,
    dirty: bool,
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
    provider_id: Option<String>,
    model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkspacePathQuery {
    path: String,
}

#[derive(Debug, Deserialize)]
struct UpdateSessionRequest {
    title: Option<String>,
    provider_id: Option<String>,
    model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PromptRequest {
    message_id: Option<Uuid>,
    content: String,
    mode: Option<Mode>,
    thinking_level: Option<String>,
    #[serde(default)]
    attachments: Vec<PromptImageAttachmentRequest>,
    provider_id: Option<String>,
    model_id: Option<String>,
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
struct SetFastModeRequest {
    fast_mode: bool,
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
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    session_header: Option<String>,
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

#[derive(Debug, Deserialize, Default)]
struct SessionDiffQuery {
    to_hash: Option<String>,
}

#[derive(Debug, Serialize)]
struct SessionFileDiffResponse {
    path: String,
    status: Option<String>,
    additions: usize,
    deletions: usize,
    diff: String,
}

#[derive(Debug, Serialize)]
struct SessionDiffsResponse {
    files: Vec<SessionFileDiffResponse>,
    total_additions: usize,
    total_deletions: usize,
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
    is_gpt: bool,
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
    session_header: Option<String>,
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
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub allowed_tools: Option<String>,
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
        .route("/build-info", get(build_info))
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
        .route("/sessions/{session_id}/diffs", get(session_diffs))
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
        .route(
            "/config/global-instructions",
            get(get_global_instructions)
                .put(update_global_instructions)
                .delete(delete_global_instructions),
        )
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
        .route("/config/fast-mode", get(get_fast_mode).post(set_fast_mode))
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

async fn build_info() -> Json<BuildInfoResponse> {
    Json(BuildInfoResponse {
        version: build_info::DISPLAY_VERSION,
        commit: build_info::COMMIT,
        dirty: build_info::is_dirty(),
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
    // Subscribe before taking the snapshot. Requests emitted in the small
    // interval are delivered by the receiver; the frontend de-duplicates by
    // request ID if they are also present in the snapshot.
    let pending = state.runtime.pending_frontend_requests();
    Sse::new(request_stream(pending, receiver, state.cancel.clone()))
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
                is_gpt: model.is_gpt(),
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
        is_gpt: model.is_gpt(),
        thinking_levels,
        thinking_level: model.thinking_level.to_string(),
    };
    let provider_id = model.provider_id.clone();
    let model_id = model.model_id.clone();
    state.runtime.update_config(|config| {
        config.default_provider = provider_id;
        config.default_model = model_id;
    });
    state.runtime.save_config()?;
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

async fn get_fast_mode(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "fast_mode": state.runtime.config().ui.fast_mode,
    }))
}

async fn set_fast_mode(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SetFastModeRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let previous = state.runtime.config().ui.fast_mode;
    state
        .runtime
        .update_config(|config| config.ui.fast_mode = request.fast_mode);
    if let Err(error) = state.runtime.save_config() {
        state
            .runtime
            .update_config(|config| config.ui.fast_mode = previous);
        return Err(error.into());
    }
    Ok(Json(serde_json::json!({
        "fast_mode": request.fast_mode,
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
    pending: Vec<FrontendRequest>,
    mut receiver: UnboundedReceiver<FrontendRequest>,
    cancel: tokio_util::sync::CancellationToken,
) -> impl futures_core::Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        for request in pending {
            let Ok(data) = serde_json::to_string(&request) else { continue };
            yield Ok(Event::default().event("frontend_request").data(data));
        }
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
            "headers": {"x-tenant-id": "team-a"},
            "session_header": "x-conversation-id",
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
        assert_eq!(
            provider.headers.get("x-tenant-id").map(String::as_str),
            Some("team-a")
        );
        assert_eq!(
            provider.session_header.as_deref(),
            Some("x-conversation-id")
        );
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
