//! System-level routes (restart, health, etc.)

use axum::{Json, extract::State, http::StatusCode};

use crate::state::AppState;

/// `POST /api/system/restart`
///
/// Gracefully shuts down the server and re-executes the process
/// with the original CLI arguments. This allows applying updates
/// or configuration changes that require a restart.
///
/// Returns `202 Accepted` immediately. The actual restart happens after
/// all in-flight requests complete and the SSE connections are closed.
pub async fn restart_handler(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    state
        .restart_requested
        .store(true, std::sync::atomic::Ordering::Relaxed);
    state.cancel_token.cancel();
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "status": "restarting" })),
    )
}
