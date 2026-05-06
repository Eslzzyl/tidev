//! Terminal API routes for web terminal.
//!
//! - `POST /api/terminal/start`     — Start a new terminal session
//! - `POST /api/terminal/input`     — Send raw input to a terminal session
//! - `POST /api/terminal/resize`    — Resize the PTY (cols × rows)
//! - `GET  /api/terminal/events`    — SSE stream for terminal output
//! - `DELETE /api/terminal/{id}`    — Close a terminal session

use axum::{
    Router,
    extract::{Path, Query, State},
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    routing::{delete, get, post},
    Json,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use super::super::state::AppState;

pub fn terminal_routes() -> Router<AppState> {
    Router::new()
        .route("/terminal/start", post(start_terminal))
        .route("/terminal/input", post(terminal_input))
        .route("/terminal/resize", post(terminal_resize))
        .route("/terminal/events", get(terminal_events))
        .route("/terminal/{session_id}", delete(close_terminal))
}

// ── Request / Response types ──────────────────────────────────────────────

#[derive(Serialize)]
struct StartResponse {
    session_id: String,
}

#[derive(Deserialize)]
struct InputRequest {
    session_id: String,
    data: String,
}

#[derive(Deserialize)]
struct ResizeRequest {
    session_id: String,
    cols: u16,
    rows: u16,
}

#[derive(Deserialize)]
struct EventsQuery {
    session_id: String,
}

// ── Handlers ──────────────────────────────────────────────────────────────

async fn start_terminal(
    State(state): State<AppState>,
) -> Result<Json<StartResponse>, crate::web::error::AppError> {
    let size = portable_pty::PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    };

    let session_id = state
        .terminal_manager
        .start_session(state.terminal_tx.clone(), size)
        .await
        .map_err(|e| crate::web::error::AppError::Internal(e))?;

    Ok(Json(StartResponse {
        session_id: session_id.to_string(),
    }))
}

async fn terminal_input(
    State(state): State<AppState>,
    Json(req): Json<InputRequest>,
) -> Result<(), crate::web::error::AppError> {
    let session_id = Uuid::parse_str(&req.session_id)
        .map_err(|e| {
            crate::web::error::AppError::BadRequest(format!("Invalid session_id: {e}"))
        })?;

    state
        .terminal_manager
        .write_input(session_id, req.data.as_bytes())
        .await
        .map_err(|e| crate::web::error::AppError::Internal(e))?;

    Ok(())
}

async fn terminal_resize(
    State(state): State<AppState>,
    Json(req): Json<ResizeRequest>,
) -> Result<(), crate::web::error::AppError> {
    let session_id = Uuid::parse_str(&req.session_id)
        .map_err(|e| {
            crate::web::error::AppError::BadRequest(format!("Invalid session_id: {e}"))
        })?;

    state
        .terminal_manager
        .resize(session_id, req.cols, req.rows)
        .await
        .map_err(|e| crate::web::error::AppError::Internal(e))?;

    Ok(())
}

async fn terminal_events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, crate::web::error::AppError> {
    let session_id = Uuid::parse_str(&query.session_id).map_err(|e| {
        crate::web::error::AppError::BadRequest(format!("Invalid session_id: {e}"))
    })?;

    // Flush any buffered output that was produced before this subscriber
    // connected (e.g. the initial shell prompt).
    let buf = state.terminal_manager.get_buffer(session_id).await;
    let initial = if buf.is_empty() {
        None
    } else {
        let text = String::from_utf8_lossy(&buf).to_string();
        Some(Ok(Event::default()
            .event("terminal.output")
            .data(text)))
    };

    let rx = state.terminal_tx.subscribe();

    let live = BroadcastStream::new(rx).filter_map(move |result| {
        let sid = session_id;
        async move {
            match result {
                Ok(output) if output.session_id == sid => {
                    if output.closed {
                        Some(Ok(Event::default()
                            .event("terminal.close")
                            .data("")))
                    } else {
                        let text = String::from_utf8_lossy(&output.data).to_string();
                        Some(Ok(Event::default()
                            .event("terminal.output")
                            .data(text)))
                    }
                }
                _ => None,
            }
        }
    });

    // Prepend buffered output (e.g. initial prompt) before live stream.
    let init_events: Vec<Result<Event, Infallible>> = initial.into_iter().collect();
    let stream = futures_util::stream::iter(init_events).chain(live);
    Ok(Sse::new(stream))
}

async fn close_terminal(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Ok(id) = Uuid::parse_str(&session_id) {
        state.terminal_manager.close_session(id).await;
    }
    axum::http::StatusCode::OK
}
