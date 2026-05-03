use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

use crate::web::{
    error::{AppError, WebResult},
    state::AppState,
};

/// Session info for API
#[derive(Serialize)]
pub struct SessionInfo {
    pub session_id: Uuid,
    pub parent_session_id: Option<Uuid>,
    pub workspace_root: String,
    pub provider_id: String,
    pub provider_display_name: String,
    pub model_id: String,
    pub model_display_name: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::storage::SessionRecord> for SessionInfo {
    fn from(record: crate::storage::SessionRecord) -> Self {
        Self {
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
        }
    }
}

/// Session list response
#[derive(Serialize)]
pub struct SessionsResponse {
    pub sessions: Vec<SessionInfo>,
}

/// Create session request
#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub workspace_root: String,
    #[serde(default)]
    pub title: Option<String>,
}

/// Create session response
#[derive(Serialize)]
pub struct CreateSessionResponse {
    pub session_id: Uuid,
}

/// Fork session request
#[derive(Deserialize)]
pub struct ForkSessionRequest {
    pub message_id: Uuid,
    #[serde(default)]
    pub title: Option<String>,
}

/// Fork session response
#[derive(Serialize)]
pub struct ForkSessionResponse {
    pub session_id: Uuid,
    pub message_count: usize,
}

/// Session detail response
#[derive(Serialize)]
pub struct SessionDetail {
    pub session_id: Uuid,
    pub parent_session_id: Option<Uuid>,
    pub workspace_root: String,
    pub provider_id: String,
    pub provider_display_name: String,
    pub model_id: String,
    pub model_display_name: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub context_summary: Option<String>,
    pub context_retained_from: usize,
    pub revert_message_id: Option<Uuid>,
}

/// List all sessions for the current workspace
pub async fn list_sessions(State(state): State<AppState>) -> WebResult<Json<SessionsResponse>> {
    crate::log_debug!(
        "Listing sessions for workspace: {}",
        state.workspace_root.display()
    );
    let store = state.store.lock().await;
    let records = store.load_sessions_for_workspace(&state.workspace_root)?;
    let sessions: Vec<SessionInfo> = records.into_iter().map(Into::into).collect();
    crate::log_info!("Listed {} sessions for workspace", sessions.len());
    Ok(Json(SessionsResponse { sessions }))
}

/// Create a new session
pub async fn create_session(
    State(state): State<AppState>,
    Json(body): Json<CreateSessionRequest>,
) -> WebResult<(StatusCode, Json<CreateSessionResponse>)> {
    crate::log_info!("Creating new session in workspace: {}", body.workspace_root);

    // Get default provider and model from config
    let config = state.config.read().await;

    // Find first enabled provider with an enabled model
    let (provider_id, provider, model_id, model) = config
        .providers
        .iter()
        .find_map(|(pid, p)| {
            p.models
                .iter()
                .next()
                .map(|(mid, m)| (pid.clone(), p.clone(), mid.clone(), m.clone()))
        })
        .ok_or_else(|| AppError::Internal("No provider/model found".to_string()))?;

    let session_id = Uuid::new_v4();
    let title = body.title.unwrap_or_else(|| "New Session".to_string());

    let store = state.store.lock().await;
    store.create_session(
        session_id,
        Path::new(&body.workspace_root),
        &provider_id,
        &provider.display_name,
        &model_id,
        &model.display_name,
        &title,
    )?;
    drop(store);

    crate::log_info!(
        "Created session {} with provider {} and model {}",
        session_id,
        provider_id,
        model_id
    );
    Ok((
        StatusCode::CREATED,
        Json(CreateSessionResponse { session_id }),
    ))
}

/// Get session details
pub async fn get_session(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
) -> WebResult<Json<SessionDetail>> {
    crate::log_debug!("Getting session details for {}", session_id);
    let store = state.store.lock().await;
    let record = store.load_session_record(session_id)?.ok_or_else(|| {
        crate::log_warn!("Session {} not found", session_id);
        AppError::NotFound(format!("Session {} not found", session_id))
    })?;
    let revert_message_id = store.load_revert_message_id(session_id)?;
    drop(store);

    crate::log_debug!("Retrieved session {} details", session_id);
    Ok(Json(SessionDetail {
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
        context_summary: record.context_summary,
        context_retained_from: record.context_retained_from,
        revert_message_id,
    }))
}

/// Delete a session
pub async fn delete_session(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
) -> WebResult<StatusCode> {
    crate::log_info!("Deleting session {}", session_id);
    let store = state.store.lock().await;
    store.delete_session(session_id)?;
    crate::log_info!("Deleted session {}", session_id);
    Ok(StatusCode::NO_CONTENT)
}

/// Fork a session from a specific message.
/// Creates a new session containing all messages up to (and including) the specified message.
pub async fn fork_session(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
    Json(body): Json<ForkSessionRequest>,
) -> WebResult<(StatusCode, Json<ForkSessionResponse>)> {
    crate::log_info!(
        "Fork request for session {} from message {}",
        session_id,
        body.message_id
    );

    let store = state.store.lock().await;

    // Load source session
    let source_session = store
        .load_session_record(session_id)?
        .ok_or_else(|| AppError::NotFound(format!("Session {} not found", session_id)))?;

    // Load all messages
    let messages = store.load_messages(session_id)?;

    // Find the target message
    let target_idx = messages
        .iter()
        .position(|m| m.id == body.message_id)
        .ok_or_else(|| AppError::NotFound(format!("Message {} not found", body.message_id)))?;

    // Verify target is a user message
    if !matches!(messages[target_idx].role, crate::session::MessageRole::User) {
        return Err(AppError::BadRequest(
            "Can only fork from user messages".to_string(),
        ));
    }

    let new_session_id = Uuid::new_v4();
    let title = body
        .title
        .unwrap_or_else(|| format!("Fork of {}", source_session.title));

    // Create new session with parent reference
    store.create_session_with_parent(
        new_session_id,
        source_session.session_id,
        std::path::Path::new(&source_session.workspace_root),
        &source_session.provider_id,
        &source_session.provider_display_name,
        &source_session.model_id,
        &source_session.model_display_name,
        &title,
    )?;

    // Copy messages up to (and including) the target message
    let messages_to_copy = &messages[..=target_idx];
    for original in messages_to_copy {
        let mut new_message = original.clone();
        new_message.id = Uuid::new_v4();
        store.append_message(new_session_id, &new_message)?;
    }

    let message_count = messages_to_copy.len();
    drop(store);

    crate::log_info!(
        "Forked session {} → new session {} with {} messages",
        session_id,
        new_session_id,
        message_count
    );

    Ok((
        StatusCode::CREATED,
        Json(ForkSessionResponse {
            session_id: new_session_id,
            message_count,
        }),
    ))
}

/// Rename session request
#[derive(Deserialize)]
pub struct RenameSessionRequest {
    pub title: String,
}

/// Rename session response
#[derive(Serialize)]
pub struct RenameSessionResponse {
    pub success: bool,
    pub title: String,
}

/// Init prompt response
#[derive(Serialize)]
pub struct InitPromptResponse {
    pub prompt: String,
}

/// Get the init prompt for creating AGENTS.md
pub async fn get_init_prompt() -> Json<InitPromptResponse> {
    Json(InitPromptResponse {
        prompt: crate::prompts::init_command().to_string(),
    })
}

/// Rename a session
pub async fn rename_session(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
    Json(body): Json<RenameSessionRequest>,
) -> WebResult<Json<RenameSessionResponse>> {
    crate::log_info!("Renaming session {} to '{}'", session_id, body.title);

    let title = if body.title.trim().is_empty() {
        "Untitled session".to_string()
    } else {
        body.title.trim().to_string()
    };

    let store = state.store.lock().await;
    store.update_session_title(session_id, &title)?;
    drop(store);

    crate::log_info!("Session {} renamed to '{}'", session_id, title);
    Ok(Json(RenameSessionResponse {
        success: true,
        title,
    }))
}

/// Workspace info response
#[derive(Serialize)]
pub struct WorkspaceInfo {
    pub workspace_root: String,
}

/// Get current workspace info
pub async fn get_workspace(State(state): State<AppState>) -> WebResult<Json<WorkspaceInfo>> {
    crate::log_debug!("Getting workspace info: {}", state.workspace_root.display());
    Ok(Json(WorkspaceInfo {
        workspace_root: state.workspace_root.display().to_string(),
    }))
}
