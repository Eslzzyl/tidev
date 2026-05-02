use std::net::SocketAddr;

use axum::Router;
use tokio::net::TcpListener;

use super::{routes::create_router, state::AppState};

/// Web server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 26502,
        }
    }
}

/// Start the web server
pub async fn start_server(state: AppState, config: ServerConfig) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;

    let app = create_router(state);

    let listener = TcpListener::bind(&addr).await?;
    eprintln!("Web server listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

/// Create router for testing or embedding
pub fn create_app(state: AppState) -> Router {
    create_router(state)
}
