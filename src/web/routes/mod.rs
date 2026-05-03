pub mod events;
pub mod files;
pub mod messages;
pub mod models;
pub mod sessions;
pub mod static_file;
pub mod todos;
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
        // Workspace
        .route("/workspace", get(sessions::get_workspace))
        // Sessions
        .route("/sessions", get(sessions::list_sessions).post(sessions::create_session))
        .route(
            "/sessions/{id}",
            get(sessions::get_session).delete(sessions::delete_session),
        )
        // Todos
        .route("/sessions/{id}/todos", get(todos::get_todos))
        // Messages
        .route(
            "/sessions/{id}/messages",
            get(messages::list_messages).post(messages::send_message),
        )
        .route("/sessions/{id}/abort", post(messages::abort_request))
        .route("/sessions/{id}/revert", post(messages::revert_to_message))
        // Models
        .route("/models", get(models::list_models))
        // Tools
        .route("/tools", get(tools::list_tools))
        // Files (for @-mention)
        .route("/files/search", get(files::search_files))
        // CORS
        .layer(CorsLayer::permissive())
}

use static_file::StaticConfig;

/// Create the complete router
pub fn create_router(state: AppState, static_config: StaticConfig) -> Router {
    let api = api_routes();
    let static_files = static_file::static_routes(static_config);

    Router::new()
        .nest("/api", api)
        .merge(static_files)
        .with_state(state)
}
