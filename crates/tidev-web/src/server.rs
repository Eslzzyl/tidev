use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use tokio::net::TcpListener;
use tokio::signal;
use tokio_util::sync::CancellationToken;

use crate::api::AppState;
use crate::frontend::{Frontend, FrontendConfig};

/// Options for starting the tidev Web server.
#[derive(Clone, Debug)]
pub struct WebOptions {
    pub host: String,
    pub port: u16,
    pub workspace: Option<PathBuf>,
}

impl Default for WebOptions {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port: 26502,
            workspace: None,
        }
    }
}

/// Build the Runtime and start the Web server.
pub async fn run(options: WebOptions) -> Result<()> {
    let cancel = CancellationToken::new();
    let signal_task = tokio::spawn(shutdown_signal(cancel.clone()));
    let workspace = options.workspace.unwrap_or(std::env::current_dir()?);
    let runtime = tidev_core::Runtime::builder()
        .workspace_root(workspace)
        .console_logging(true)
        .build()
        .await?;
    let frontend = Frontend::start(FrontendConfig::default(), cancel.clone()).await;
    let frontend_mode = frontend.mode();

    if cancel.is_cancelled() {
        frontend.shutdown().await;
        runtime.shutdown().await;
        signal_task.abort();
        return Ok(());
    }

    let state = AppState {
        runtime: runtime.clone(),
        frontend_mode,
        cancel: cancel.clone(),
    };
    let app = Router::new()
        .nest("/api", crate::api::router())
        .merge(frontend.router())
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(Arc::new(state));

    let addr: SocketAddr = format!("{}:{}", options.host, options.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    log::info!("tidev web listening on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(cancel.cancelled_owned())
        .await?;

    frontend.shutdown().await;
    runtime.shutdown().await;
    let _ = signal_task.await;
    Ok(())
}

async fn shutdown_signal(cancel: CancellationToken) {
    tokio::select! {
        result = signal::ctrl_c() => {
            if let Err(error) = result {
                log::warn!("failed to listen for Ctrl+C: {error}");
            }
        }
        _ = cancel.cancelled() => {}
    }
    cancel.cancel();
}
