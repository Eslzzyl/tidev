use axum::{
    Json,
    extract::{Path as AxumPath, State},
};
use serde::Serialize;
use uuid::Uuid;

use crate::web::{error::WebResult, state::AppState};

/// Todo item for API
#[derive(Serialize)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
}

/// Todo list response
#[derive(Serialize)]
pub struct TodosResponse {
    pub todos: Vec<TodoItem>,
}

/// Get todos for a session
pub async fn get_todos(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
) -> WebResult<Json<TodosResponse>> {
    log::debug!("Getting todos for session {}", session_id);
    let store = state.store.lock().await;
    let todos = store.load_todos(session_id)?;
    drop(store);

    let todos: Vec<TodoItem> = todos
        .into_iter()
        .map(|t| TodoItem {
            content: t.content,
            status: t.status,
        })
        .collect();

    log::debug!("Retrieved {} todos for session {}", todos.len(), session_id);
    Ok(Json(TodosResponse { todos }))
}
