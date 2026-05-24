use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::state::AppState;

/// Public paths that don't require authentication.
/// These are checked by the middleware (applied to the /api sub-router,
/// so paths do NOT include the /api prefix).
const PUBLIC_PREFIXES: &[&str] = &["/auth/", "/events"];

/// Axum middleware that checks Bearer token authentication.
///
/// - If no token is configured in AuthStore, all requests pass through.
/// - If a token is configured, all `/api/*` requests (except `/api/auth/*`)
///   must include `Authorization: Bearer <token>`.
pub async fn auth_middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();

    // Always allow public endpoints (auth status/verify, events)
    if is_public_path(path) {
        return next.run(req).await;
    }

    // Read configured token from AuthStore
    let configured = {
        let auth = state.auth.read().await;
        auth.web_token().map(|s| s.to_string())
    };

    let configured = match configured {
        Some(t) if !t.is_empty() => t,
        _ => return next.run(req).await, // no token configured → allow
    };

    // Check Authorization header
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let provided = auth_header.strip_prefix("Bearer ").unwrap_or("");

    if provided == configured {
        next.run(req).await
    } else {
        let body = serde_json::json!({
            "error": "Unauthorized: invalid or missing auth token"
        });
        (StatusCode::UNAUTHORIZED, axum::Json(body)).into_response()
    }
}

fn is_public_path(path: &str) -> bool {
    PUBLIC_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
}
