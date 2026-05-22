pub mod auth;
pub mod config;
pub mod events;
pub mod files;
pub mod fs;
pub mod git;
pub mod messages;
pub mod models;
pub mod providers;
pub mod sessions;
pub mod shell;
pub mod skills;
pub mod static_file;
pub mod stats;
pub mod terminal;
pub mod todos;
pub mod tools;

use axum::{
    Router, middleware,
    routing::{delete, get, post},
};
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;

use super::state::AppState;

/// Create the API router
pub fn api_routes() -> Router<AppState> {
    Router::new()
        // Auth (public endpoints — always accessible)
        .route("/auth/status", get(auth::auth_status))
        .route("/auth/verify", post(auth::auth_verify))
        .route("/auth/configure", post(auth::auth_configure))
        // SSE events
        .route("/events", get(events::events_stream))
        // Workspace
        .route("/workspace", get(sessions::get_workspace))
        // Sessions
        .route(
            "/sessions",
            get(sessions::list_sessions).post(sessions::create_session),
        )
        .route(
            "/sessions/{id}",
            get(sessions::get_session).delete(sessions::delete_session),
        )
        .route("/sessions/{id}/fork", post(sessions::fork_session))
        // Todos
        .route("/sessions/{id}/todos", get(todos::get_todos))
        // Messages
        .route(
            "/sessions/{id}/messages",
            get(messages::list_messages).post(messages::send_message),
        )
        .route("/sessions/{id}/abort", post(messages::abort_request))
        .route("/sessions/{id}/revert", post(messages::revert_to_message))
        .route("/sessions/{id}/redo", post(messages::redo_last_undo))
        .route("/sessions/{id}/compact", post(messages::compact_session))
        .route("/sessions/{id}/rename", post(sessions::rename_session))
        // Shell
        .route("/sessions/{id}/shell", post(shell::execute_shell_command))
        // Init prompt
        .route("/init", get(sessions::get_init_prompt))
        // Models
        .route("/models", get(models::list_models))
        // Config
        .route(
            "/config/default-model",
            get(config::get_default_model).post(config::set_default_model),
        )
        .route(
            "/config/agent-models",
            get(config::get_agent_models).post(config::set_agent_model),
        )
        .route(
            "/config/memory-model",
            get(config::get_memory_model).post(config::set_memory_model),
        )
        .route(
            "/config/model-thinking-level",
            get(config::get_model_thinking_level).post(config::set_model_thinking_level),
        )
        // Providers
        .route(
            "/providers",
            get(providers::list_providers).post(providers::create_provider),
        )
        .route("/providers/{id}", delete(providers::delete_provider))
        .route(
            "/providers/{id}/connect",
            post(providers::connect_provider).delete(providers::disconnect_provider),
        )
        // Tools
        .route("/tools", get(tools::list_tools))
        // Skills
        .route("/skills", get(skills::list_skills))
        // Files (for @-mention)
        .route("/files/search", get(files::search_files))
        // Filesystem browser
        .route("/fs/list", get(fs::list_directory))
        .route("/fs/read", get(fs::read_file))
        .route("/fs/write", post(fs::write_file))
        .route("/fs/create", post(fs::create_item))
        .route("/fs/rename", post(fs::rename_item))
        .route("/fs/remove", delete(fs::remove_item))
        .route("/fs/read-base64", get(fs::read_file_base64))
        // Git
        .merge(git::git_routes())
        // Terminal
        .merge(terminal::terminal_routes())
        // Stats
        .route("/stats/summary", get(stats::get_summary))
        .route("/stats/timeseries", get(stats::get_timeseries))
        .route("/stats/models", get(stats::get_model_usage))
        .route("/stats/providers", get(stats::get_provider_usage))
        .route("/stats/sessions", get(stats::get_session_usage))
        // CORS
        .layer(CorsLayer::permissive())
}

use static_file::StaticConfig;

/// Create the complete router
pub fn create_router(state: AppState, static_config: StaticConfig) -> Router {
    let api = api_routes();
    let static_files = static_file::static_routes(static_config);

    // Apply auth middleware to API routes only
    let api = api.layer(middleware::from_fn_with_state(
        state.clone(),
        crate::web::auth::auth_middleware,
    ));

    Router::new()
        .nest("/api", api)
        .merge(static_files)
        .layer(CompressionLayer::new())
        .with_state(state)
}
