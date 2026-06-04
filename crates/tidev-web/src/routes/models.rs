use axum::{Json, extract::State};
use serde::Serialize;

use crate::{error::WebResult, state::AppState};
use tidev_engine::config::reasoning::ThinkingMatcher;

/// Model info
#[derive(Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub provider_id: String,
    pub provider_name: String,
    pub supports_vision: bool,
    pub supports_streaming: bool,
    pub thinking_supported: bool,
    pub thinking_level: String,
    pub thinking_options: Vec<String>,
}

/// List models response
#[derive(Serialize)]
pub struct ListModelsResponse {
    pub models: Vec<ModelInfo>,
}

/// List all available models that have API keys configured
pub async fn list_models(State(state): State<AppState>) -> WebResult<Json<ListModelsResponse>> {
    log::debug!("Listing available models");
    let config = state.config.read().await;
    let auth = state.auth.read().await;

    let models: Vec<ModelInfo> = config
        .connected_models(&auth)
        .into_iter()
        .map(|summary| {
            let provider = config.provider(&summary.provider_id);
            let model = provider.and_then(|p| p.models.get(&summary.model_id));
            // Determine thinking level with same cascade as resolve_model_by_ids
            let thinking_level = if let Some(rid) = model.and_then(|m| m.request_model_id.as_ref())
            {
                ThinkingMatcher::match_for_model(rid)
            } else {
                ThinkingMatcher::match_for_model(&summary.model_display_name)
            };
            let thinking_supported = !thinking_level.is_none();
            let thinking_options = if thinking_supported {
                thinking_level_options(&summary.model_id)
            } else {
                vec![]
            };
            ModelInfo {
                id: summary.model_id,
                display_name: summary.model_display_name,
                provider_id: summary.provider_id,
                provider_name: summary.provider_display_name,
                supports_vision: model.map(|m| m.supports_images).unwrap_or(false),
                supports_streaming: model.map(|m| m.supports_streaming).unwrap_or(true),
                thinking_supported,
                thinking_level: thinking_level.to_string(),
                thinking_options,
            }
        })
        .collect();

    log::info!("Listed {} models with API keys configured", models.len());
    Ok(Json(ListModelsResponse { models }))
}

/// Return available thinking level options for a given model ID.
fn thinking_level_options(model_id: &str) -> Vec<String> {
    let id = model_id.to_ascii_lowercase();
    if id.contains("deepseek") && id.contains("4") {
        vec!["deepseek:Off", "deepseek:High", "deepseek:Max"]
            .into_iter()
            .map(String::from)
            .collect()
    } else if id.contains("qwen") && id.contains("3.") {
        vec!["qwen:Off", "qwen:On"]
            .into_iter()
            .map(String::from)
            .collect()
    } else if id.contains("glm") {
        vec!["glm:Off", "glm:On"]
            .into_iter()
            .map(String::from)
            .collect()
    } else if id.contains("gpt") && id.contains("5") {
        vec![
            "gpt5:Off",
            "gpt5:Low",
            "gpt5:Medium",
            "gpt5:High",
            "gpt5:XHigh",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    } else {
        vec![]
    }
}
