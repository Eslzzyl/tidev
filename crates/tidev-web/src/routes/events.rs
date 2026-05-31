use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Query, State},
    response::sse::{Event, Sse},
};
use futures_util::StreamExt;
use futures_util::stream::Stream;
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::{error::AppError, event_bus::AppEvent, state::AppState};

/// Query parameters for SSE connection
#[derive(Deserialize)]
pub struct EventsQuery {
    session: Uuid,
    /// Optional auth token (for SSE which can't set custom headers)
    token: Option<String>,
}

/// Convert an AppEvent into the SSE event-type string used on the wire.
fn event_type_str(event: &AppEvent) -> &'static str {
    match event {
        AppEvent::Heartbeat => "heartbeat",
        AppEvent::MessageChunk { .. } => "message.chunk",
        AppEvent::ReasoningChunk { .. } => "reasoning.chunk",
        AppEvent::MessageComplete { .. } => "message.complete",
        AppEvent::UsageStats { .. } => "usage.stats",
        AppEvent::ToolCall { .. } => "tool.call",
        AppEvent::ToolResult { .. } => "tool.result",
        AppEvent::PermissionRequest { .. } => "permission.request",
        AppEvent::Aborted { .. } => "aborted",
        AppEvent::Error { .. } => "error",
        AppEvent::Retrying { .. } => "retrying",
        AppEvent::MessagesUpdated { .. } => "messages.updated",
        AppEvent::CompactionChunk { .. } => "compaction.chunk",
        AppEvent::ShellOutput { .. } => "shell.output",
        AppEvent::SubagentStatus { .. } => "subagent.status",
        AppEvent::SubagentToolResult { .. } => "subagent.tool_result",
        AppEvent::SubagentCompleted { .. } => "subagent.completed",
        AppEvent::StreamEnd { .. } => "stream.end",
    }
}

/// Build an SSE Event from an AppEvent.
fn sse_from_event(event: &AppEvent) -> Result<Event, serde_json::Error> {
    let json = serde_json::to_string(event)?;
    Ok(Event::default().event(event_type_str(event)).data(json))
}

/// SSE endpoint for real-time events
///
/// This endpoint is public (bypasses auth middleware) because EventSource
/// cannot set custom headers. Auth is handled inline via query param.
pub async fn events_stream(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    // Validate auth token if configured
    let auth = state.auth.read().await;
    if let Some(configured) = auth.web_token() {
        let provided = query.token.as_deref().unwrap_or("");
        if provided != configured {
            return Err(AppError::Unauthorized(
                "Invalid or missing auth token".into(),
            ));
        }
    }
    drop(auth);

    let session_id = query.session;
    log::info!("SSE connection established for session {}", session_id);

    // Atomically subscribe + drain the per-session event buffer.
    // This ensures we don't lose events published before the subscription.
    let (rx, buffered) = state.event_bus.subscribe_and_drain(session_id);
    let cancel_token = state.cancel_token.clone();

    let stream = async_stream::stream! {
        // --- Phase 1: replay buffered events (events that were published
        //     before this SSE client subscribed) --------------------------
        for event in &buffered {
            match sse_from_event(event) {
                Ok(e) => yield Ok(e),
                Err(_) => continue,
            }
        }

        // --- Phase 2: stream live events from the broadcast channel ------
        let mut broadcast_stream = BroadcastStream::new(rx);

        loop {
            tokio::select! {
                // Check for shutdown signal
                _ = cancel_token.cancelled() => {
                    log::debug!("SSE connection closing due to shutdown for session {}", session_id);
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
                        AppEvent::ReasoningChunk { session_id: sid, .. } => *sid == session_id,
                        AppEvent::MessageComplete { session_id: sid, .. } => *sid == session_id,
                        AppEvent::UsageStats { session_id: sid, .. } => *sid == session_id,
                        AppEvent::ToolCall { session_id: sid, .. } => *sid == session_id,
                        AppEvent::ToolResult { session_id: sid, .. } => *sid == session_id,
                        AppEvent::PermissionRequest { session_id: sid, .. } => *sid == session_id,
                        AppEvent::Aborted { session_id: sid, .. } => *sid == session_id,
                        AppEvent::Error { session_id: sid, .. } => *sid == session_id,
                        AppEvent::Retrying { session_id: sid, .. } => *sid == session_id,
                        AppEvent::ShellOutput { session_id: sid, .. } => *sid == session_id,
                        AppEvent::MessagesUpdated { session_id: sid } => *sid == session_id,
                        AppEvent::CompactionChunk { session_id: sid, .. } => *sid == session_id,
                        AppEvent::SubagentStatus { session_id: sid, .. } => *sid == session_id,
                        AppEvent::SubagentToolResult { session_id: sid, .. } => *sid == session_id,
                        AppEvent::SubagentCompleted { session_id: sid, .. } => *sid == session_id,
                        AppEvent::StreamEnd { session_id: sid, .. } => *sid == session_id,
                    };

                    if !matches_session {
                        continue;
                    }

                    match sse_from_event(&event) {
                        Ok(e) => yield Ok(e),
                        Err(_) => continue,
                    }
                }
            }
        }

        log::info!("SSE connection closed for session {}", session_id);
    };

    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("{}"),
    ))
}
