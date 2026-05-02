use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::unbounded_channel;
use uuid::Uuid;

use crate::{
    config::reasoning::ThinkingLevelType,
    session::{BackendEvent, Message, MessageRole},
    web::{
        error::{AppError, WebResult},
        event_bus::AppEvent,
        state::AppState,
    },
};

/// Message in the API
#[derive(Serialize)]
pub struct ApiMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

/// List messages response
#[derive(Serialize)]
pub struct ListMessagesResponse {
    pub messages: Vec<ApiMessage>,
}

/// Send message request
#[derive(Deserialize)]
pub struct SendMessageRequest {
    pub content: String,
    #[serde(default)]
    pub thinking_level: Option<String>,
}

/// Send message response
#[derive(Serialize)]
pub struct SendMessageResponse {
    pub request_id: u64,
}

/// Abort request
#[derive(Deserialize)]
pub struct AbortRequest {
    pub request_id: u64,
}

/// List messages for a session
pub async fn list_messages(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
) -> WebResult<Json<ListMessagesResponse>> {
    crate::log_debug!("Listing messages for session {}", session_id);
    let store = state.store.lock().await;

    // Load messages for the session
    let messages_db = store.load_messages(session_id)?;

    // Check if session exists by trying to load the record
    let _ = store
        .load_session_record(session_id)?
        .ok_or_else(|| {
            crate::log_warn!("Session {} not found when listing messages", session_id);
            AppError::NotFound(format!("Session {} not found", session_id))
        })?;
    drop(store);

    let messages: Vec<ApiMessage> = messages_db
        .into_iter()
        .map(|msg| ApiMessage {
            id: msg.id.to_string(),
            role: match msg.role {
                MessageRole::User => "user".to_string(),
                MessageRole::Assistant => "assistant".to_string(),
                MessageRole::System => "system".to_string(),
                MessageRole::Tool => "tool".to_string(),
                MessageRole::Error => "error".to_string(),
                MessageRole::Shell => "shell".to_string(),
            },
            content: msg.content,
            created_at: msg.created_at.to_rfc3339(),
        })
        .collect();

    crate::log_debug!("Listed {} messages for session {}", messages.len(), session_id);
    Ok(Json(ListMessagesResponse { messages }))
}

/// Send a message to the session
pub async fn send_message(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
    Json(body): Json<SendMessageRequest>,
) -> WebResult<(StatusCode, Json<SendMessageResponse>)> {
    crate::log_info!("Sending message to session {} (content length: {})", session_id, body.content.len());

    // Check if session exists
    let record = {
        let store = state.store.lock().await;
        store
            .load_session_record(session_id)?
            .ok_or_else(|| {
                crate::log_warn!("Session {} not found when sending message", session_id);
                AppError::NotFound(format!("Session {} not found", session_id))
            })?
    };

    // Generate request ID
    let request_id = rand::random::<u64>();
    crate::log_debug!("Generated request ID {} for session {}", request_id, session_id);

    // Track this request
    state.track_request(session_id, request_id).await;

    // Add user message to database
    let user_message = Message {
        id: Uuid::new_v4(),
        role: MessageRole::User,
        content: body.content.clone(),
        attachments: vec![],
        reasoning: String::new(),
        tool_calls: vec![],
        tool_call_id: None,
        tool_name: None,
        metadata: Default::default(),
        created_at: Utc::now(),
        completed_at: Some(Utc::now()),
        streaming: false,
        input_tokens: None,
        output_tokens: None,
        total_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        model_id: None,
        tokens_per_second: None,
        snapshot_hash: None,
        patch_files: None,
        file_diffs: None,
        mode: None,
        rtk_rewritten: false,
        thinking_level: None,
    };
    {
        let store = state.store.lock().await;
        store.append_message(session_id, &user_message)?;
    }

    // Get existing messages for context
    let messages = {
        let store = state.store.lock().await;
        store.load_messages(session_id)?
    };

    // Get config and tools
    let _thinking_level = body
        .thinking_level
        .map(|_| ThinkingLevelType::default())
        .unwrap_or_default();

    // Get model config
    let config = state.config.read().await;
    let provider = config
        .providers
        .get(&record.provider_id)
        .cloned()
        .ok_or_else(|| AppError::Internal("Provider not found".to_string()))?;

    let model_config = provider
        .models
        .get(&record.model_id)
        .cloned()
        .ok_or_else(|| AppError::Internal("Model not found".to_string()))?;
    
    let model = crate::config::ActiveModel {
        provider_id: record.provider_id.clone(),
        provider_display_name: provider.display_name.clone(),
        model_id: record.model_id.clone(),
        request_model_id: model_config.request_model_id.clone().unwrap_or_else(|| record.model_id.clone()),
        display_name: model_config.display_name.clone(),
        base_url: provider.base_url.clone(),
        api_key: None, // Will be loaded from auth store in LLM client
        api_type: match provider.api_type.as_deref() {
            Some("anthropic") => crate::config::ApiType::Anthropic,
            Some("openai_responses") => crate::config::ApiType::OpenAiResponses,
            _ => crate::config::ApiType::OpenAiChatCompletions,
        },
        temperature: model_config.temperature,
        context_window: model_config.context_window,
        max_output_tokens: model_config.max_output_tokens,
        supports_images: model_config.supports_images,
        system_prompt: model_config.system_prompt.clone().unwrap_or_default(),
        extra_body: model_config.extra_body.clone(),
        thinking_level: ThinkingLevelType::default(),
    };

    // Build tools (simplified - just built-in for now)
    let tools = vec![];

    // Create channel for backend events
    let (tx, mut rx) = unbounded_channel::<BackendEvent>();
    let event_bus = state.event_bus.clone();

    // Spawn task to handle LLM stream
    let llm_client = state.llm_client.clone();
    let session_id_for_task = session_id;
    let request_id_for_task = request_id;

    tokio::spawn(async move {
        // Stream chat
        llm_client
            .stream_chat(
                session_id_for_task,
                request_id_for_task,
                model,
                messages,
                tools,
                tx,
                ThinkingLevelType::default(),
            )
            .await;
    });

    // Spawn task to forward events to event bus
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let app_event = match event {
                BackendEvent::Delta {
                    session_id,
                    request_id,
                    content,
                } => Some(AppEvent::MessageChunk {
                    session_id,
                    request_id,
                    content,
                }),
                BackendEvent::Finished { session_id, request_id, .. } => {
                    Some(AppEvent::MessageComplete { session_id, request_id })
                }
                BackendEvent::Failed {
                    session_id,
                    request_id,
                    error,
                } => Some(AppEvent::Error {
                    session_id,
                    request_id,
                    message: error,
                }),
                BackendEvent::ToolCallUpdated {
                    session_id,
                    request_id,
                    tool_call,
                } => Some(AppEvent::ToolCall {
                    session_id,
                    request_id,
                    tool_call_id: tool_call.id,
                    tool_name: tool_call.name,
                    arguments: tool_call.arguments,
                }),
                BackendEvent::ToolCompleted {
                    session_id,
                    request_id,
                    tool_call,
                    result,
                } => Some(AppEvent::ToolResult {
                    session_id,
                    request_id,
                    tool_call_id: tool_call.id,
                    output: result.output,
                }),
                _ => None,
            };

            if let Some(app_event) = app_event {
                event_bus.publish(app_event);
            }
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(SendMessageResponse { request_id }),
    ))
}

/// Abort a running request
pub async fn abort_request(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
    Json(body): Json<AbortRequest>,
) -> WebResult<StatusCode> {
    crate::log_info!("Abort request for session {} request {}", session_id, body.request_id);

    // Check if there's an active request
    let active = state.get_active_request(session_id).await;

    if active == Some(body.request_id) {
        state.remove_request(session_id).await;
        state.event_bus.publish(AppEvent::Aborted {
            session_id,
            request_id: body.request_id,
        });
        crate::log_info!("Aborted request {} for session {}", body.request_id, session_id);
    } else {
        crate::log_warn!("No active request {} found for session {}", body.request_id, session_id);
    }

    Ok(StatusCode::OK)
}
