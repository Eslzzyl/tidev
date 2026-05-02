use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Query, State},
    response::sse::{Event, Sse},
};
use futures_util::stream::Stream;
use futures_util::StreamExt;
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::web::{
    event_bus::AppEvent,
    state::AppState,
};

/// Query parameters for SSE connection
#[derive(Deserialize)]
pub struct EventsQuery {
    session: Uuid,
}

/// SSE endpoint for real-time events
pub async fn events_stream(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let session_id = query.session;
    crate::log_info!("SSE connection established for session {}", session_id);
    let rx = state.event_bus.subscribe();
    let cancel_token = state.cancel_token.clone();

    let stream = async_stream::stream! {
        let mut broadcast_stream = BroadcastStream::new(rx);

        loop {
            tokio::select! {
                // Check for shutdown signal
                _ = cancel_token.cancelled() => {
                    crate::log_debug!("SSE connection closing due to shutdown for session {}", session_id);
                    break;
                }
                // Wait for next event
                result = broadcast_stream.next() => {
                    let event = match result {
                        Some(Ok(event)) => event,
                        Some(Err(_)) => continue,
                        None => break,
                    };

                    // Filter events by session_id (except heartbeat)
                    let matches_session = match &event {
                        AppEvent::Heartbeat => true,
                        AppEvent::MessageChunk { session_id: sid, .. } => *sid == session_id,
                        AppEvent::MessageComplete { session_id: sid, .. } => *sid == session_id,
                        AppEvent::ToolCall { session_id: sid, .. } => *sid == session_id,
                        AppEvent::ToolResult { session_id: sid, .. } => *sid == session_id,
                        AppEvent::PermissionRequest { session_id: sid, .. } => *sid == session_id,
                        AppEvent::Aborted { session_id: sid, .. } => *sid == session_id,
                        AppEvent::Error { session_id: sid, .. } => *sid == session_id,
                    };

                    if !matches_session {
                        continue;
                    }

                    // Convert AppEvent to SSE Event
                    let event_type = match &event {
                        AppEvent::Heartbeat => "heartbeat",
                        AppEvent::MessageChunk { .. } => "message.chunk",
                        AppEvent::MessageComplete { .. } => "message.complete",
                        AppEvent::ToolCall { .. } => "tool.call",
                        AppEvent::ToolResult { .. } => "tool.result",
                        AppEvent::PermissionRequest { .. } => "permission.request",
                        AppEvent::Aborted { .. } => "aborted",
                        AppEvent::Error { .. } => "error",
                    };

                    let json = match serde_json::to_string(&event) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };

                    yield Ok(Event::default()
                        .event(event_type)
                        .data(json));
                }
            }
        }

        crate::log_info!("SSE connection closed for session {}", session_id);
    };

    Sse::new(stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(Duration::from_secs(10))
                .text("{}"),
        )
}
