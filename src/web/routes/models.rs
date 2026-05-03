use axum::{Json, extract::State};
use serde::Serialize;

use crate::web::{error::WebResult, state::AppState};

/// Model info
#[derive(Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub provider_id: String,
    pub provider_name: String,
    pub supports_vision: bool,
    pub supports_streaming: bool,
}

/// List models response
#[derive(Serialize)]
pub struct ListModelsResponse {
    pub models: Vec<ModelInfo>,
}

/// List all available models that have API keys configured
pub async fn list_models(State(state): State<AppState>) -> WebResult<Json<ListModelsResponse>> {
    crate::log_debug!("Listing available models");
    let config = state.config.read().await;
    let auth = state.auth.read().await;

    let models: Vec<ModelInfo> = config
        .connected_models(&auth)
        .into_iter()
        .map(|summary| {
            let provider = config.provider(&summary.provider_id);
            let model = provider.and_then(|p| p.models.get(&summary.model_id));
            ModelInfo {
                id: summary.model_id,
                display_name: summary.model_display_name,
                provider_id: summary.provider_id,
                provider_name: summary.provider_display_name,
                supports_vision: model.map(|m| m.supports_images).unwrap_or(false),
                supports_streaming: model.map(|m| m.supports_streaming).unwrap_or(true),
            }
        })
        .collect();

    crate::log_info!("Listed {} models with API keys configured", models.len());
    Ok(Json(ListModelsResponse { models }))
}
