use std::collections::HashMap;

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::web::{error::AppError, state::AppState};

/// Set default model request
#[derive(Deserialize)]
pub struct SetDefaultModelRequest {
    pub provider_id: String,
    pub model_id: String,
}

/// Set default model response
#[derive(Serialize)]
pub struct SetDefaultModelResponse {
    pub success: bool,
    pub provider_id: String,
    pub model_id: String,
    pub provider_display_name: String,
    pub model_display_name: String,
}

/// Set the default model for new sessions
pub async fn set_default_model(
    State(state): State<AppState>,
    Json(body): Json<SetDefaultModelRequest>,
) -> Result<Json<SetDefaultModelResponse>, AppError> {
    crate::log_info!(
        "Setting default model to {}/{}",
        body.provider_id,
        body.model_id
    );

    // Verify the provider and model exist
    let config = state.config.read().await;
    let provider = config.provider(&body.provider_id).ok_or_else(|| {
        AppError::BadRequest(format!("Provider '{}' not found", body.provider_id))
    })?;
    let model = provider.models.get(&body.model_id).ok_or_else(|| {
        AppError::BadRequest(format!(
            "Model '{}' not found for provider '{}'",
            body.model_id, body.provider_id
        ))
    })?;

    let provider_display_name = provider.display_name.clone();
    let model_display_name = model.display_name.clone();
    drop(config);

    // Update config
    let mut config = state.config.write().await;
    config.default_provider = body.provider_id.clone();
    config.default_model = body.model_id.clone();

    // Save config to file
    if let Err(e) = config.save(&state.config_paths) {
        crate::log_error!("Failed to save config: {}", e);
        return Err(AppError::Internal(format!("Failed to save config: {}", e)));
    }
    drop(config);

    crate::log_info!(
        "Default model set to {} ({}) / {} ({})",
        body.provider_id,
        provider_display_name,
        body.model_id,
        model_display_name
    );

    Ok(Json(SetDefaultModelResponse {
        success: true,
        provider_id: body.provider_id,
        model_id: body.model_id,
        provider_display_name,
        model_display_name,
    }))
}

/// Get current default model
#[derive(Serialize)]
pub struct GetDefaultModelResponse {
    pub provider_id: String,
    pub model_id: String,
    pub provider_display_name: String,
    pub model_display_name: String,
}

pub async fn get_default_model(
    State(state): State<AppState>,
) -> Result<Json<GetDefaultModelResponse>, AppError> {
    let config = state.config.read().await;

    let provider_id = config.default_provider.clone();
    let model_id = config.default_model.clone();

    // Get display names
    let (provider_display_name, model_display_name) =
        if let Some(provider) = config.provider(&provider_id) {
            let model_name = provider
                .models
                .get(&model_id)
                .map(|m| m.display_name.clone())
                .unwrap_or_else(|| model_id.clone());
            (provider.display_name.clone(), model_name)
        } else {
            (provider_id.clone(), model_id.clone())
        };

    Ok(Json(GetDefaultModelResponse {
        provider_id,
        model_id,
        provider_display_name,
        model_display_name,
    }))
}

/// Response for GET /api/config/agent-models
#[derive(Serialize)]
pub struct GetAgentModelsResponse {
    pub default_model: GetDefaultModelResponse,
    pub agent_models: HashMap<String, String>,
}

pub async fn get_agent_models(
    State(state): State<AppState>,
) -> Result<Json<GetAgentModelsResponse>, AppError> {
    let config = state.config.read().await;

    let provider_id = config.default_provider.clone();
    let model_id = config.default_model.clone();

    let (provider_display_name, model_display_name) =
        if let Some(provider) = config.provider(&provider_id) {
            let model_name = provider
                .models
                .get(&model_id)
                .map(|m| m.display_name.clone())
                .unwrap_or_else(|| model_id.clone());
            (provider.display_name.clone(), model_name)
        } else {
            (provider_id.clone(), model_id.clone())
        };

    let default_model = GetDefaultModelResponse {
        provider_id,
        model_id,
        provider_display_name,
        model_display_name,
    };

    // Clone agent model overrides into a HashMap
    let agent_models: HashMap<String, String> = config.agent.models.clone().into_iter().collect();

    Ok(Json(GetAgentModelsResponse {
        default_model,
        agent_models,
    }))
}

/// Request to set (or clear) an agent model override.
#[derive(Deserialize)]
pub struct SetAgentModelRequest {
    pub agent_type: String,
    /// "provider_id/model_id" or empty string to clear the override.
    pub model_str: String,
}

#[derive(Serialize)]
pub struct SetAgentModelResponse {
    pub success: bool,
}

pub async fn set_agent_model(
    State(state): State<AppState>,
    Json(body): Json<SetAgentModelRequest>,
) -> Result<Json<SetAgentModelResponse>, AppError> {
    let agent_type = body.agent_type.trim().to_ascii_lowercase();

    // Validate the agent type is known
    crate::agent::AgentType::parse(&agent_type).ok_or_else(|| {
        AppError::BadRequest(format!(
            "Unknown agent type '{}'. Valid types: general, explorer, librarian, oracle, designer, fixer",
            body.agent_type
        ))
    })?;

    // If model_str is not empty, validate the provider and model exist
    if !body.model_str.is_empty() {
        let parts: Vec<&str> = body.model_str.splitn(2, '/').collect();
        if parts.len() != 2 {
            return Err(AppError::BadRequest(
                "model_str must be in 'provider_id/model_id' format".to_string(),
            ));
        }
        let provider_id = parts[0];
        let model_id = parts[1];

        let config = state.config.read().await;
        let provider = config
            .provider(provider_id)
            .ok_or_else(|| AppError::BadRequest(format!("Provider '{}' not found", provider_id)))?;
        if !provider.models.contains_key(model_id) {
            return Err(AppError::BadRequest(format!(
                "Model '{}' not found for provider '{}'",
                model_id, provider_id
            )));
        }
        drop(config);
    }

    // Update config and persist
    let mut config = state.config.write().await;
    config.set_agent_model(&state.config_paths, &agent_type, &body.model_str)?;
    drop(config);

    Ok(Json(SetAgentModelResponse { success: true }))
}
