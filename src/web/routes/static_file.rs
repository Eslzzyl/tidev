use axum::{
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use std::path::PathBuf;
use tower_http::services::ServeDir;

use crate::web::state::AppState;

/// Create static file routes
pub fn static_routes() -> Router<AppState> {
    // In development, serve from web/dist directory
    // In production, files are embedded
    let dist_path = PathBuf::from("web/dist");

    if dist_path.exists() {
        Router::new()
            .route("/health", get(health_check))
            .fallback_service(ServeDir::new(&dist_path))
    } else {
        // Fallback: serve a simple HTML page indicating the frontend is not built
        Router::new()
            .route("/", get(index_fallback))
            .route("/health", get(health_check))
    }
}

/// Health check endpoint
async fn health_check() -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "status": "ok",
        "service": "tidev-web"
    }))
}

/// Fallback index page when frontend is not built
async fn index_fallback() -> impl IntoResponse {
    Html(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>TiDev Web</title>
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
    <h1>TiDev Web</h1>
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
