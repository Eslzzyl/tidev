use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};

use crate::web::{error::AppError, state::AppState};

/// GET /api/auth/status — check if authentication is required
#[derive(Serialize)]
pub struct AuthStatusResponse {
    pub auth_required: bool,
}

pub async fn auth_status(State(state): State<AppState>) -> Json<AuthStatusResponse> {
    let auth = state.auth.read().await;
    let auth_required = auth.web_token().is_some();
    Json(AuthStatusResponse { auth_required })
}

/// POST /api/auth/verify — verify a token (public, no auth needed)
#[derive(Deserialize)]
pub struct AuthVerifyRequest {
    pub token: String,
}

#[derive(Serialize)]
pub struct AuthVerifyResponse {
    pub valid: bool,
}

pub async fn auth_verify(
    State(state): State<AppState>,
    Json(body): Json<AuthVerifyRequest>,
) -> Json<AuthVerifyResponse> {
    let auth = state.auth.read().await;
    let configured = auth.web_token().unwrap_or("");
    let valid = !configured.is_empty() && body.token == configured;
    Json(AuthVerifyResponse { valid })
}

/// POST /api/auth/configure — set or change the web auth token
#[derive(Deserialize)]
pub struct AuthConfigureRequest {
    pub new_token: String,
}

#[derive(Serialize)]
pub struct AuthConfigureResponse {
    pub success: bool,
}

pub async fn auth_configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AuthConfigureRequest>,
) -> Result<Json<AuthConfigureResponse>, AppError> {
    // Check existing token if configured
    {
        let auth = state.auth.read().await;
        if let Some(existing) = auth.web_token() {
            let auth_header = headers
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let provided = auth_header.strip_prefix("Bearer ").unwrap_or("");
            if provided != existing {
                return Err(AppError::Unauthorized("Invalid current auth token".into()));
            }
        }
    }

    // Update token
    {
        let mut auth = state.auth.write().await;
        auth.set_web_token(body.new_token.clone());
        auth.save(&state.config_paths)
            .map_err(|e| AppError::Internal(format!("Failed to save auth store: {}", e)))?;
    }

    crate::log_info!("Web auth token updated");
    Ok(Json(AuthConfigureResponse { success: true }))
}
