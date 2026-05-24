use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use tidev_engine::config::{ModelConfig, ProviderConfig};
use crate::web::{
    error::{AppError, WebResult},
    state::AppState,
};

/// Model summary in provider response
#[derive(Serialize)]
pub struct ProviderModelInfo {
    pub id: String,
    pub display_name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub temperature: Option<f32>,
    pub supports_images: bool,
    pub supports_streaming: bool,
}

/// Provider info response
#[derive(Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub display_name: String,
    pub source: String,
    pub connected: bool,
    pub base_url: String,
    pub models: Vec<ProviderModelInfo>,
}

/// List providers response
#[derive(Serialize)]
pub struct ListProvidersResponse {
    pub providers: Vec<ProviderInfo>,
}

/// Connect provider request
#[derive(Deserialize)]
pub struct ConnectProviderRequest {
    pub api_key: String,
}

/// Create model request
#[derive(Deserialize)]
pub struct CreateModelRequest {
    pub model_id: String,
    pub display_name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub supports_images: bool,
}

/// Create provider request
#[derive(Deserialize)]
pub struct CreateProviderRequest {
    pub provider_id: String,
    pub display_name: String,
    pub base_url: String,
    pub api_key: String,
    pub models: Vec<CreateModelRequest>,
}

/// List all providers
pub async fn list_providers(
    State(state): State<AppState>,
) -> WebResult<Json<ListProvidersResponse>> {
    log::debug!("Listing all providers");

    let config = state.config.read().await;
    let auth = state.auth.read().await;

    let providers: Vec<ProviderInfo> = config
        .provider_ids()
        .into_iter()
        .filter_map(|provider_id| {
            let provider = config.provider(&provider_id)?;
            let source = config.provider_source(&provider_id)?;
            let connected = auth.api_key(&provider_id).is_some();

            let models: Vec<ProviderModelInfo> = provider
                .models
                .iter()
                .map(|(id, model)| ProviderModelInfo {
                    id: id.clone(),
                    display_name: model.display_name.clone(),
                    context_window: model.context_window,
                    max_output_tokens: model.max_output_tokens,
                    temperature: model.temperature,
                    supports_images: model.supports_images,
                    supports_streaming: model.supports_streaming,
                })
                .collect();

            Some(ProviderInfo {
                id: provider_id,
                display_name: provider.display_name.clone(),
                source: match source {
                    tidev_engine::config::ProviderSource::Bundled => "bundled".to_string(),
                    tidev_engine::config::ProviderSource::User => "user".to_string(),
                },
                connected,
                base_url: provider.base_url.clone(),
                models,
            })
        })
        .collect();

    log::info!("Listed {} providers", providers.len());
    Ok(Json(ListProvidersResponse { providers }))
}

/// Connect a provider (set API key)
pub async fn connect_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(body): Json<ConnectProviderRequest>,
) -> WebResult<StatusCode> {
    log::debug!("Connecting provider: {}", provider_id);

    // Validate provider exists
    {
        let config = state.config.read().await;
        if config.provider(&provider_id).is_none() {
            return Err(AppError::NotFound(format!(
                "Provider '{}' not found",
                provider_id
            )));
        }
    }

    // Validate API key is not empty
    if body.api_key.trim().is_empty() {
        return Err(AppError::BadRequest("API key cannot be empty".to_string()));
    }

    // Set API key
    {
        let mut auth = state.auth.write().await;
        auth.set_api_key(&provider_id, body.api_key);
        auth.save(&state.config_paths)
            .map_err(|e| AppError::Internal(format!("Failed to save auth: {}", e)))?;
    }

    log::info!("Connected provider: {}", provider_id);
    Ok(StatusCode::NO_CONTENT)
}

/// Disconnect a provider (remove API key)
pub async fn disconnect_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> WebResult<StatusCode> {
    log::debug!("Disconnecting provider: {}", provider_id);

    // Remove API key
    {
        let mut auth = state.auth.write().await;
        auth.remove_api_key(&provider_id);
        auth.save(&state.config_paths)
            .map_err(|e| AppError::Internal(format!("Failed to save auth: {}", e)))?;
    }

    log::info!("Disconnected provider: {}", provider_id);
    Ok(StatusCode::NO_CONTENT)
}

/// Create a custom provider
pub async fn create_provider(
    State(state): State<AppState>,
    Json(body): Json<CreateProviderRequest>,
) -> WebResult<StatusCode> {
    log::debug!("Creating provider: {}", body.provider_id);

    // Validate provider_id
    let provider_id = body.provider_id.trim().to_ascii_lowercase();
    if provider_id.is_empty() {
        return Err(AppError::BadRequest(
            "Provider ID cannot be empty".to_string(),
        ));
    }

    // Check provider_id format (lowercase letters, numbers, -, _)
    if !provider_id
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_'))
    {
        return Err(AppError::BadRequest(
            "Provider ID may only contain lowercase letters, numbers, '-' or '_'".to_string(),
        ));
    }

    // Check if provider already exists
    {
        let config = state.config.read().await;
        if config.provider_exists(&provider_id) {
            return Err(AppError::BadRequest(format!(
                "Provider '{}' already exists",
                provider_id
            )));
        }
    }

    // Validate required fields
    if body.display_name.trim().is_empty() {
        return Err(AppError::BadRequest(
            "Display name cannot be empty".to_string(),
        ));
    }

    let base_url = body.base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        return Err(AppError::BadRequest("Base URL cannot be empty".to_string()));
    }
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        return Err(AppError::BadRequest(
            "Base URL must start with http:// or https://".to_string(),
        ));
    }

    if body.api_key.trim().is_empty() {
        return Err(AppError::BadRequest("API key cannot be empty".to_string()));
    }

    if body.models.is_empty() {
        return Err(AppError::BadRequest(
            "At least one model must be configured".to_string(),
        ));
    }

    // Build models map
    let mut models = std::collections::BTreeMap::new();
    for model_req in body.models {
        let model_id = model_req.model_id.trim().to_string();
        if model_id.is_empty() {
            return Err(AppError::BadRequest("Model ID cannot be empty".to_string()));
        }

        let display_name = if model_req.display_name.trim().is_empty() {
            model_id.clone()
        } else {
            model_req.display_name.trim().to_string()
        };

        let temperature = match model_req.temperature {
            Some(t) if !(0.0..=2.0).contains(&t) => Some(1.0),
            other => other,
        };

        models.insert(
            model_id,
            ModelConfig {
                display_name,
                context_window: model_req.context_window,
                max_output_tokens: model_req.max_output_tokens,
                temperature,
                system_prompt: None,
                supports_streaming: true,
                supports_images: model_req.supports_images,
                extra_body: None,
                request_model_id: None,
            },
        );
    }

    // Create provider config
    let provider_config = ProviderConfig {
        display_name: body.display_name.trim().to_string(),
        base_url,
        api_type: None,
        models,
    };

    // Add to config
    {
        let mut config = state.config.write().await;
        config
            .providers
            .insert(provider_id.clone(), provider_config);
        config
            .save(&state.config_paths)
            .map_err(|e| AppError::Internal(format!("Failed to save config: {}", e)))?;
    }

    // Set API key
    {
        let mut auth = state.auth.write().await;
        auth.set_api_key(&provider_id, body.api_key);
        auth.save(&state.config_paths)
            .map_err(|e| AppError::Internal(format!("Failed to save auth: {}", e)))?;
    }

    log::info!("Created provider: {}", provider_id);
    Ok(StatusCode::CREATED)
}

/// Delete a custom provider
pub async fn delete_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> WebResult<StatusCode> {
    log::debug!("Deleting provider: {}", provider_id);

    // Check provider exists and is user-defined
    {
        let config = state.config.read().await;
        match config.provider_source(&provider_id) {
            Some(tidev_engine::config::ProviderSource::Bundled) => {
                return Err(AppError::BadRequest(
                    "Cannot delete bundled providers".to_string(),
                ));
            }
            None => {
                return Err(AppError::NotFound(format!(
                    "Provider '{}' not found",
                    provider_id
                )));
            }
            _ => {}
        }
    }

    // Remove provider from config
    {
        let mut config = state.config.write().await;
        config.providers.remove(&provider_id);
        config
            .save(&state.config_paths)
            .map_err(|e| AppError::Internal(format!("Failed to save config: {}", e)))?;
    }

    // Remove API key
    {
        let mut auth = state.auth.write().await;
        auth.remove_api_key(&provider_id);
        auth.save(&state.config_paths)
            .map_err(|e| AppError::Internal(format!("Failed to save auth: {}", e)))?;
    }

    log::info!("Deleted provider: {}", provider_id);
    Ok(StatusCode::NO_CONTENT)
}
