use super::*;

pub(super) fn normalized_api_type(value: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let api_type = match value.to_ascii_lowercase().as_str() {
        "openai_chat_completions" | "openai" | "chat" => ApiType::OpenAiChatCompletions,
        "openai_responses" | "responses" => ApiType::OpenAiResponses,
        "anthropic" | "claude" => ApiType::Anthropic,
        "google_gemini" | "gemini" | "google" => ApiType::GoogleGemini,
        _ => {
            return Err(ApiError::bad_request(format!(
                "unsupported api_type '{value}'"
            )));
        }
    };

    Ok(Some(api_type.as_str().to_owned()))
}

pub(super) fn required_provider_field(value: &str, field: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ApiError::bad_request(format!("{field} is required")));
    }
    Ok(value.to_owned())
}

pub(super) fn validate_provider_id(value: &str) -> Result<String, ApiError> {
    let provider_id = required_provider_field(value, "provider_id")?;
    if !provider_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(ApiError::bad_request(
            "provider_id may contain only letters, numbers, '-', '_' and '.'",
        ));
    }
    Ok(provider_id)
}

pub(super) fn build_provider_config(
    request: &CreateProviderRequest,
) -> Result<(String, String, ProviderConfig), ApiError> {
    let provider_id = validate_provider_id(&request.provider_id)?;
    let display_name = required_provider_field(&request.display_name, "display_name")?;
    let base_url = required_provider_field(&request.base_url, "base_url")?;
    let api_type = normalized_api_type(request.api_type.as_deref())?;
    if let Some(user_agent) = request.user_agent.as_deref() {
        tidev_config::provider::validate_user_agent(user_agent)
            .map_err(|error| ApiError::bad_request(format!("invalid user_agent: {error}")))?;
    }
    tidev_config::provider::validate_headers(&request.headers, request.session_header.as_deref())
        .map_err(|error| ApiError::bad_request(format!("invalid request headers: {error}")))?;
    let api_key = required_provider_field(&request.api_key, "api_key")?;

    if request.models.is_empty() {
        return Err(ApiError::bad_request(
            "at least one model is required for a provider",
        ));
    }

    let mut models = BTreeMap::new();
    for request_model in &request.models {
        let model_id = required_provider_field(&request_model.model_id, "model_id")?;
        let model_display_name =
            required_provider_field(&request_model.display_name, "model display_name")?;
        if request_model.context_window == 0 {
            return Err(ApiError::bad_request(format!(
                "model '{model_id}' context_window must be greater than zero"
            )));
        }
        if request_model.max_output_tokens == 0 {
            return Err(ApiError::bad_request(format!(
                "model '{model_id}' max_output_tokens must be greater than zero"
            )));
        }
        if request_model
            .temperature
            .is_some_and(|temperature| !temperature.is_finite())
        {
            return Err(ApiError::bad_request(format!(
                "model '{model_id}' temperature must be finite"
            )));
        }
        if models.contains_key(&model_id) {
            return Err(ApiError::conflict(format!(
                "duplicate model_id '{model_id}'"
            )));
        }

        models.insert(
            model_id,
            ModelConfig {
                display_name: model_display_name,
                context_window: request_model.context_window,
                max_output_tokens: request_model.max_output_tokens,
                api_type: normalized_api_type(request_model.api_type.as_deref())?,
                base_url: request_model
                    .base_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
                temperature: request_model.temperature,
                system_prompt: None,
                supports_streaming: request_model.supports_streaming,
                supports_images: request_model.supports_images,
                supports_parallel_tool_calls: request_model.supports_parallel_tool_calls,
                extra_body: None,
                request_model_id: request_model
                    .request_model_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
            },
        );
    }

    Ok((
        provider_id,
        api_key,
        ProviderConfig {
            display_name,
            base_url,
            api_type,
            user_agent: request.user_agent.clone(),
            headers: request.headers.clone(),
            session_header: request.session_header.clone(),
            models,
        },
    ))
}

pub(super) fn provider_model_dto(model_id: &str, model: &ModelConfig) -> ProviderModelDto {
    ProviderModelDto {
        id: model_id.to_owned(),
        display_name: model.display_name.clone(),
        request_model_id: model.request_model_id.clone(),
        context_window: model.context_window,
        max_output_tokens: model.max_output_tokens,
        api_type: model.api_type.clone(),
        base_url: model.base_url.clone(),
        temperature: model.temperature,
        supports_images: model.supports_images,
        supports_streaming: model.supports_streaming,
        supports_parallel_tool_calls: model.supports_parallel_tool_calls,
    }
}

pub(super) fn provider_dto(
    config: &tidev_config::AppConfig,
    auth: &tidev_config::AuthStore,
    provider_id: &str,
) -> Option<ProviderDto> {
    let provider = config.provider(provider_id)?;
    let is_bundled = config.bundled_providers.contains_key(provider_id);
    Some(ProviderDto {
        id: provider_id.to_owned(),
        display_name: provider.display_name.clone(),
        source: if is_bundled { "bundled" } else { "user" },
        can_delete: !is_bundled && config.providers.contains_key(provider_id),
        connected: auth.api_key(provider_id).is_some(),
        base_url: provider.base_url.clone(),
        api_type: provider.api_type.clone(),
        user_agent: provider.user_agent.clone(),
        session_header: provider.session_header.clone(),
        models: provider
            .models
            .iter()
            .map(|(model_id, model)| provider_model_dto(model_id, model))
            .collect(),
    })
}

pub(super) async fn list_providers(State(state): State<Arc<AppState>>) -> Json<ProvidersResponse> {
    let config = state.runtime.config();
    let auth = state.runtime.auth();
    let providers = config
        .provider_ids()
        .iter()
        .filter_map(|provider_id| provider_dto(&config, &auth, provider_id))
        .collect();
    Json(ProvidersResponse { providers })
}

pub(super) async fn create_provider(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateProviderRequest>,
) -> Result<Json<ProviderDto>, ApiError> {
    let (provider_id, api_key, provider) = build_provider_config(&request)?;
    let config_before = state.runtime.config();
    if config_before.provider_exists(&provider_id) {
        return Err(ApiError::conflict(format!(
            "provider '{provider_id}' already exists"
        )));
    }

    let mut update_error = None;
    let provider_id_for_update = provider_id.clone();
    let provider_for_update = provider.clone();
    state.runtime.update_config(|config| {
        update_error = config
            .set_user_provider(provider_id_for_update, provider_for_update)
            .err();
    });
    if let Some(error) = update_error {
        return Err(error.into());
    }
    if let Err(error) = state.runtime.save_config() {
        state
            .runtime
            .update_config(|config| *config = config_before.clone());
        return Err(error.into());
    }

    let auth_before = state.runtime.auth();
    state
        .runtime
        .update_auth(|auth| auth.set_api_key(&provider_id, &api_key));
    if let Err(error) = state.runtime.save_auth() {
        state.runtime.update_auth(|auth| *auth = auth_before);
        state
            .runtime
            .update_config(|config| *config = config_before);
        let _ = state.runtime.save_config();
        return Err(error.into());
    }

    let config = state.runtime.config();
    let auth = state.runtime.auth();
    let provider = provider_dto(&config, &auth, &provider_id)
        .ok_or_else(|| ApiError::internal("created provider could not be loaded"))?;
    Ok(Json(provider))
}

pub(super) async fn delete_provider(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
) -> Result<Json<ProviderMutationResponse>, ApiError> {
    let config_before = state.runtime.config();
    if config_before.bundled_providers.contains_key(&provider_id) {
        return Err(ApiError::forbidden(format!(
            "bundled provider '{provider_id}' cannot be deleted"
        )));
    }
    if !config_before.providers.contains_key(&provider_id) {
        return Err(ApiError::not_found(format!(
            "provider '{provider_id}' not found"
        )));
    }

    let active_provider_id = state.runtime.active_provider_id();
    if config_before.default_provider == provider_id || active_provider_id == provider_id {
        return Err(ApiError::conflict(format!(
            "provider '{provider_id}' is currently active or configured as the default"
        )));
    }
    if config_before.agent.default_subagent_provider == provider_id
        || config_before.agent.models.values().any(|model| {
            model
                .split_once('/')
                .is_some_and(|(provider, _)| provider == provider_id)
        })
    {
        return Err(ApiError::conflict(format!(
            "provider '{provider_id}' is referenced by agent configuration"
        )));
    }

    let mut update_error = None;
    let provider_id_for_update = provider_id.clone();
    state.runtime.update_config(|config| {
        update_error = config.remove_user_provider(&provider_id_for_update).err();
    });
    if let Some(error) = update_error {
        return Err(error.into());
    }
    if let Err(error) = state.runtime.save_config() {
        state
            .runtime
            .update_config(|config| *config = config_before.clone());
        return Err(error.into());
    }

    let auth_before = state.runtime.auth();
    state.runtime.update_auth(|auth| {
        auth.remove_api_key(&provider_id);
    });
    if let Err(error) = state.runtime.save_auth() {
        state.runtime.update_auth(|auth| *auth = auth_before);
        state
            .runtime
            .update_config(|config| *config = config_before);
        let _ = state.runtime.save_config();
        return Err(error.into());
    }

    Ok(Json(ProviderMutationResponse {
        success: true,
        connected: None,
    }))
}

pub(super) async fn connect_provider(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Json(request): Json<ConnectProviderRequest>,
) -> Result<Json<ProviderMutationResponse>, ApiError> {
    if state.runtime.config().provider(&provider_id).is_none() {
        return Err(ApiError::not_found(format!(
            "provider '{provider_id}' not found"
        )));
    }
    let api_key = required_provider_field(&request.api_key, "api_key")?;
    let auth_before = state.runtime.auth();
    state
        .runtime
        .update_auth(|auth| auth.set_api_key(&provider_id, &api_key));
    if let Err(error) = state.runtime.save_auth() {
        state.runtime.update_auth(|auth| *auth = auth_before);
        return Err(error.into());
    }
    Ok(Json(ProviderMutationResponse {
        success: true,
        connected: Some(true),
    }))
}

pub(super) async fn disconnect_provider(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
) -> Result<Json<ProviderMutationResponse>, ApiError> {
    if state.runtime.config().provider(&provider_id).is_none() {
        return Err(ApiError::not_found(format!(
            "provider '{provider_id}' not found"
        )));
    }
    let auth_before = state.runtime.auth();
    state.runtime.update_auth(|auth| {
        auth.remove_api_key(&provider_id);
    });
    if let Err(error) = state.runtime.save_auth() {
        state.runtime.update_auth(|auth| *auth = auth_before);
        return Err(error.into());
    }
    Ok(Json(ProviderMutationResponse {
        success: true,
        connected: Some(false),
    }))
}

pub(super) async fn get_default_model(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let model = state.runtime.active_model();
    Json(serde_json::json!({
        "provider_id": model.provider_id,
        "provider_display_name": model.provider_display_name,
        "model_id": model.model_id,
        "model_display_name": model.display_name,
    }))
}

pub(super) async fn set_default_model(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SelectModelRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let config = state.runtime.config();
    let auth = state.runtime.auth();
    let model = config.resolve_model_by_ids(&auth, &request.provider_id, &request.model_id)?;
    state.runtime.update_config(|config| {
        config.default_provider = request.provider_id;
        config.default_model = request.model_id;
    });
    state.runtime.save_config()?;
    state.runtime.set_active_model(model.clone());
    Ok(Json(serde_json::json!({
        "success": true,
        "provider_id": model.provider_id,
        "model_id": model.model_id,
        "provider_display_name": model.provider_display_name,
        "model_display_name": model.display_name,
    })))
}

pub(super) async fn get_agent_models(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let model = state.runtime.active_model();
    let config = state.runtime.config();
    Json(serde_json::json!({
        "default_model": {
            "provider_id": model.provider_id,
            "model_id": model.model_id,
            "provider_display_name": model.provider_display_name,
            "model_display_name": model.display_name,
        },
        "agent_models": config.agent.models,
        "agent_thinking_levels": config.agent.thinking_levels,
    }))
}

pub(super) async fn set_agent_model(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SetAgentModelRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let agent_type = request.agent_type.trim().to_ascii_lowercase();
    if AgentType::parse(&agent_type).is_none() {
        return Err(ApiError::bad_request(format!(
            "unknown subagent type '{}', expected explorer, librarian, oracle, or fixer",
            request.agent_type
        )));
    }

    let model_str = request.model_str.trim().to_string();
    let thinking_level = request
        .thinking_level
        .unwrap_or_default()
        .trim()
        .to_string();
    state.runtime.update_config(|config| {
        if model_str.is_empty() {
            config.agent.models.remove(&agent_type);
        } else {
            config.agent.models.insert(agent_type.clone(), model_str);
        }

        if thinking_level.is_empty() {
            config.agent.thinking_levels.remove(&agent_type);
        } else {
            config
                .agent
                .thinking_levels
                .insert(agent_type.clone(), thinking_level);
        }
    });
    state.runtime.save_config()?;
    Ok(Json(serde_json::json!({ "success": true })))
}

pub(super) async fn get_subagent_config(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let config = state.runtime.config();
    Json(serde_json::json!({ "enabled": config.subagent.enabled }))
}

pub(super) async fn set_subagent_config(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SetSubagentEnabledRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state.runtime.update_config(|config| {
        config.subagent.enabled = request.enabled;
    });
    state.runtime.save_config()?;
    Ok(Json(serde_json::json!({
        "success": true,
        "enabled": request.enabled,
    })))
}

pub(super) async fn get_memory_model() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "role": "memory", "model_str": null }))
}

pub(super) async fn set_memory_model() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "success": true }))
}

pub(super) async fn get_model_thinking_level() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "thinking_level": "normal", "thinking_options": [] }))
}

pub(super) async fn set_model_thinking_level() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "success": true }))
}
