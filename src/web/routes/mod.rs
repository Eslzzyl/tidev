pub mod events;
pub mod messages;
pub mod models;
pub mod sessions;
pub mod static_file;
pub mod tools;

use axum::{
    Router,
    routing::{get, post},
};
use tower_http::cors::CorsLayer;

use super::state::AppState;

/// Create the API router
pub fn api_routes() -> Router<AppState> {
    Router::new()
        // SSE events
        .route("/events", get(events::events_stream))
        // Sessions
        .route("/sessions", get(sessions::list_sessions).post(sessions::create_session))
        .route(
            "/sessions/{id}",
            get(sessions::get_session).delete(sessions::delete_session),
        )
        // Messages
        .route(
            "/sessions/{id}/messages",
            get(messages::list_messages).post(messages::send_message),
        )
        .route("/sessions/{id}/abort", post(messages::abort_request))
        // Models
        .route("/models", get(models::list_models))
        // Tools
        .route("/tools", get(tools::list_tools))
        // CORS
        .layer(CorsLayer::permissive())
}

/// Create the complete router
pub fn create_router(state: AppState) -> Router {
    let api = api_routes();
    let static_files = static_file::static_routes();

    Router::new()
        .nest("/api", api)
        .merge(static_files)
        .with_state(state)
}
