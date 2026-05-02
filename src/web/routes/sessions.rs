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
}

/// List all sessions
pub async fn list_sessions(
    State(state): State<AppState>,
) -> WebResult<Json<SessionsResponse>> {
    crate::log_debug!("Listing all sessions");
    let store = state.store.lock().await;
    let records = store.load_all_sessions()?;
    let sessions: Vec<SessionInfo> = records.into_iter().map(Into::into).collect();
    crate::log_info!("Listed {} sessions", sessions.len());
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

    crate::log_info!("Created session {} with provider {} and model {}", session_id, provider_id, model_id);
    Ok((StatusCode::CREATED, Json(CreateSessionResponse { session_id })))
}

/// Get session details
pub async fn get_session(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
) -> WebResult<Json<SessionDetail>> {
    crate::log_debug!("Getting session details for {}", session_id);
    let store = state.store.lock().await;
    let record = store
        .load_session_record(session_id)?
        .ok_or_else(|| {
            crate::log_warn!("Session {} not found", session_id);
            AppError::NotFound(format!("Session {} not found", session_id))
        })?;
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
