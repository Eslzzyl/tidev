use std::net::SocketAddr;

use axum::Router;
use tokio::net::TcpListener;
use tokio::signal;

use super::{
    routes::{create_router, static_file::StaticConfig},
    state::AppState,
};

/// Web server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub static_config: StaticConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 26502,
            static_config: StaticConfig::default(),
        }
    }
}

/// Start the web server with graceful shutdown
pub async fn start_server(state: AppState, config: ServerConfig) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    let cancel_token = state.cancel_token.clone();

    let app = create_router(state, config.static_config);

    let listener = TcpListener::bind(&addr).await?;
    crate::log_info!("Web server listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(cancel_token))
        .await?;
    Ok(())
}

/// Shutdown signal handler for graceful shutdown
async fn shutdown_signal(cancel_token: tokio_util::sync::CancellationToken) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            crate::log_info!("Received Ctrl+C, shutting down gracefully...");
        }
        _ = terminate => {
            crate::log_info!("Received SIGTERM, shutting down gracefully...");
        }
    }

    // Signal all SSE connections to close
    cancel_token.cancel();
}

/// Create router for testing or embedding
pub fn create_app(state: AppState, static_config: StaticConfig) -> Router {
    create_router(state, static_config)
}
