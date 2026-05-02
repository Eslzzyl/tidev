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

/// List all available models
pub async fn list_models(State(state): State<AppState>) -> WebResult<Json<ListModelsResponse>> {
    let config = state.config.read().await;

    let mut models = Vec::new();

    for (provider_id, provider) in &config.providers {
        for (model_id, model) in &provider.models {
            models.push(ModelInfo {
                id: model_id.clone(),
                display_name: model.display_name.clone(),
                provider_id: provider_id.clone(),
                provider_name: provider.display_name.clone(),
                supports_vision: model.supports_images,
                supports_streaming: model.supports_streaming,
            });
        }
    }

    Ok(Json(ListModelsResponse { models }))
}
