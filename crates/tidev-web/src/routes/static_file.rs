use axum::{
    Router,
    response::{Html, IntoResponse},
    routing::get,
};
use std::path::PathBuf;
use tower_http::services::ServeDir;

use crate::{
    assets::{handle_embedded_request, has_embedded_assets},
    state::AppState,
};

/// Static file serving strategy
#[derive(Debug, Clone, Copy, Default)]
pub enum StaticMode {
    /// Development: serve from filesystem only when explicitly requested
    #[default]
    Dev,
    /// Production: embedded assets
    Embedded,
}

impl StaticMode {
    /// Detect the best mode based on build configuration
    pub fn detect() -> Self {
        if cfg!(debug_assertions) {
            // Debug build: show dev page by default
            StaticMode::Dev
        } else {
            // Release build: use embedded assets
            StaticMode::Embedded
        }
    }
}

/// Configuration for static file serving
#[derive(Debug, Clone)]
pub struct StaticConfig {
    pub mode: StaticMode,
    /// Filesystem path (for DevFs mode)
    pub fs_path: PathBuf,
    /// Force using filesystem even in dev mode (set by --dev-fs flag)
    pub use_fs: bool,
}

impl Default for StaticConfig {
    fn default() -> Self {
        Self {
            mode: StaticMode::detect(),
            fs_path: PathBuf::from("web/dist"),
            use_fs: false,
        }
    }
}

/// Create static file routes based on configuration
pub fn static_routes(config: StaticConfig) -> Router<AppState> {
    match config.mode {
        StaticMode::Dev => {
            // In dev mode, only serve from filesystem if explicitly requested (--dev-fs)
            // Otherwise show the development options page
            if config.use_fs && config.fs_path.exists() {
                Router::new()
                    .route("/health", get(health_check))
                    .fallback_service(ServeDir::new(&config.fs_path))
            } else {
                Router::new()
                    .route("/", get(dev_fallback))
                    .route("/health", get(health_check))
            }
        }
        StaticMode::Embedded => {
            // Serve embedded assets
            if has_embedded_assets() {
                Router::new()
                    .route("/health", get(health_check))
                    .fallback(handle_embedded_request)
            } else {
                // Fallback if embedded assets not available
                Router::new()
                    .route("/", get(index_fallback))
                    .route("/health", get(health_check))
            }
        }
    }
}

/// Health check endpoint
async fn health_check() -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "status": "ok",
        "service": "tidev-web"
    }))
}

/// Development fallback page with options
async fn dev_fallback() -> impl IntoResponse {
    Html(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>tidev Web - Development</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            max-width: 800px;
            margin: 50px auto;
            padding: 20px;
            text-align: center;
            color: #333;
        }
        h1 { color: #171717; }
        .info {
            background: #f3f4f6;
            border-radius: 8px;
            padding: 20px;
            margin: 20px 0;
            text-align: left;
        }
        .option {
            background: #fff;
            border: 1px solid #e5e7eb;
            border-radius: 8px;
            padding: 15px;
            margin: 15px 0;
        }
        .option h3 { margin-top: 0; color: #2563eb; }
        code {
            background: #e5e7eb;
            padding: 2px 6px;
            border-radius: 4px;
            font-family: monospace;
        }
        pre {
            background: #1f2937;
            color: #e5e7eb;
            padding: 10px;
            border-radius: 6px;
            overflow-x: auto;
        }
    </style>
</head>
<body>
    <h1>tidev Web</h1>
    <p>Development mode. Choose how to run:</p>

    <div class="option">
        <h3>Option 1: Vite Dev Server (Recommended)</h3>
        <p>Run frontend separately with hot module replacement:</p>
        <pre>cd web && pnpm dev</pre>
        <p>Then open <a href="http://localhost:5173">http://localhost:5173</a></p>
    </div>

    <div class="option">
        <h3>Option 2: Serve Built Files</h3>
        <p>If you've already built the frontend:</p>
        <pre>cargo run -- web --dev-fs</pre>
        <p>Or build first:</p>
        <pre>cd web && pnpm build
cargo run -- web --dev-fs</pre>
    </div>

    <div class="info">
        <p><strong>API Status:</strong> Backend is running at this address</p>
        <p>API endpoints are available at <code>/api/*</code></p>
    </div>
</body>
</html>"#,
    )
}

/// Fallback index page when frontend is not built (production)
async fn index_fallback() -> impl IntoResponse {
    Html(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>tidev Web</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            max-width: 800px;
            margin: 50px auto;
            padding: 20px;
            text-align: center;
            color: #333;
        }
        h1 { color: #171717; }
        .info {
            background: #f3f4f6;
            border-radius: 8px;
            padding: 20px;
            margin: 20px 0;
            text-align: left;
        }
        code {
            background: #e5e7eb;
            padding: 2px 6px;
            border-radius: 4px;
            font-family: monospace;
        }
    </style>
</head>
<body>
    <h1>tidev Web</h1>
    <p>The web frontend is not built yet.</p>
    <div class="info">
        <p><strong>To build the frontend:</strong></p>
        <ol>
            <li>Navigate to the web directory: <code>cd web</code></li>
            <li>Install dependencies: <code>pnpm install</code></li>
            <li>Build: <code>pnpm build</code></li>
        </ol>
    </div>
    <p>API endpoints are available at <code>/api/*</code></p>
</body>
</html>"#,
    )
}
