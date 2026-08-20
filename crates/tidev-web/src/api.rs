use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::{Deserialize, Serialize};

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

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health))
        .route("/events", get(events))
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    axum::Json(HealthResponse {
        status: "ok",
        service: "tidev-web",
        frontend: frontend_name(state.frontend_mode),
    })
}

async fn events(State(state): State<Arc<AppState>>, Query(query): Query<EventsQuery>) -> Response {
    let after = query.after.map(tidev_core::EventCursor);
    let subscription = state.runtime.subscribe_events(after).await;
    let replay = subscription.replay.clone();
    let mut receiver = subscription.into_receiver();
    let cancel = state.cancel.clone();

    let stream = async_stream::stream! {
        match replay {
            tidev_core::EventReplay::Events(events) => {
                for envelope in events {
                    if let Some(event) = sse_event(&envelope) {
                        yield Ok::<Event, Infallible>(event);
                    }
                }
            }
            tidev_core::EventReplay::ResyncRequired {
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
                    if let Some(event) = sse_event(&envelope) {
                        yield Ok(event);
                    }
                }
            }
        }
    };

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn sse_event(envelope: &tidev_core::EventEnvelope) -> Option<Event> {
    let data = serde_json::to_string(envelope).ok()?;
    Some(
        Event::default()
            .id(envelope.cursor.0.to_string())
            .event("backend_event")
            .data(data),
    )
}

fn frontend_name(mode: FrontendMode) -> &'static str {
    match mode {
        FrontendMode::Dev => "vite",
        FrontendMode::Embedded => "embedded",
        FrontendMode::Fallback => "fallback",
    }
}
