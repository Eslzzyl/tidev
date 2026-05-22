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
    context::ContextManager,
    prompts::{self, SessionMode},
    session::{BackendEvent, Conversation, Message, MessageRole, ToolCall},
    web::{
        error::{AppError, WebResult},
        event_bus::AppEvent,
        state::AppState,
    },
};

/// Tool call in the API
#[derive(Serialize)]
pub struct ApiToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

impl From<&ToolCall> for ApiToolCall {
    fn from(tc: &ToolCall) -> Self {
        Self {
            id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: tc.arguments.clone(),
        }
    }
}

/// Token usage in the API
#[derive(Serialize)]
pub struct ApiTokenUsage {
    pub total_tokens: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
}

/// File diff in the API
#[derive(Serialize)]
pub struct ApiFileDiff {
    pub path: String,
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
}

/// Todo item in the API
#[derive(Serialize)]
pub struct ApiTodoItem {
    pub content: String,
    pub status: String,
}

/// Message in the API
#[derive(Serialize)]
pub struct ApiMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
    /// When the message finished (for assistant messages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    /// Thinking/reasoning content for assistant messages
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Tool call ID for tool role messages
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool name for tool role messages
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Tool calls for assistant messages
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ApiToolCall>>,
    /// Unified diff patch for write/edit tool results
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    /// File path affected by the tool
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filepath: Option<String>,
    /// Whether the command was rewritten by RTK
    pub rtk_rewritten: bool,
    /// Token usage for assistant messages
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<ApiTokenUsage>,
    /// Tokens per second for this assistant message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_per_second: Option<f32>,
    /// File diffs for this message (from write/edit/delete tool results)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_diffs: Option<Vec<ApiFileDiff>>,
}

/// List messages response
#[derive(Serialize)]
pub struct ListMessagesResponse {
    pub messages: Vec<ApiMessage>,
    /// Session-level todos (aggregated from all todowrite calls)
    pub todos: Vec<ApiTodoItem>,
}

/// Send message request
#[derive(Deserialize)]
pub struct SendMessageRequest {
    pub content: String,
    #[serde(default)]
    pub thinking_level: Option<String>,
    /// Optional model override. If not provided, uses session's default model.
    #[serde(default)]
    pub model_id: Option<String>,
    /// Optional provider override. Required if model_id is provided.
    #[serde(default)]
    pub provider_id: Option<String>,
    /// Session mode: "plan" or "build". If not provided, defaults to "build".
    #[serde(default)]
    pub mode: Option<String>,
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
    let mut messages_db = store.load_messages(session_id)?;

    // Respect revert state: hide messages after the revert point,
    // matching TUI's Conversation::visible_message_count() logic.
    if let Some(revert_id) = store.load_revert_message_id(session_id)?
        && let Some(pos) = messages_db.iter().position(|m| m.id == revert_id) {
            messages_db.truncate(pos + 1);
        }

    // Load session-level todos
    let todos_db = store.load_todos(session_id)?;
    let todos: Vec<ApiTodoItem> = todos_db
        .into_iter()
        .map(|t| ApiTodoItem {
            content: t.content,
            status: t.status,
        })
        .collect();

    // Check if session exists by trying to load the record
    let _ = store.load_session_record(session_id)?.ok_or_else(|| {
        crate::log_warn!("Session {} not found when listing messages", session_id);
        AppError::NotFound(format!("Session {} not found", session_id))
    })?;
    drop(store);

    let messages: Vec<ApiMessage> = messages_db
        .into_iter()
        .map(|msg| {
            // Parse file_diffs from the stored JSON string
            let file_diffs = msg.file_diffs.as_deref().and_then(|json_str| {
                serde_json::from_str::<Vec<crate::snapshot::FileDiff>>(json_str).ok()
            });

            crate::log_debug!(
                "list_messages: msg role={} cache_read_tokens={:?} total_tokens={:?}",
                msg.role.db_value(),
                msg.cache_read_tokens,
                msg.total_tokens,
            );

            ApiMessage {
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
                completed_at: msg.completed_at.map(|t| t.to_rfc3339()),
                reasoning: if msg.reasoning.is_empty() {
                    None
                } else {
                    Some(msg.reasoning)
                },
                tool_call_id: msg.tool_call_id,
                tool_name: msg.tool_name,
                tool_calls: if msg.tool_calls.is_empty() {
                    None
                } else {
                    Some(msg.tool_calls.iter().map(ApiToolCall::from).collect())
                },
                diff: msg.metadata.diff.clone(),
                filepath: msg.metadata.filepath.clone(),
                rtk_rewritten: msg.rtk_rewritten,
                token_usage: (msg.total_tokens.or(msg.cache_read_tokens)).map(|_| ApiTokenUsage {
                    total_tokens: msg.total_tokens.unwrap_or(0),
                    input_tokens: msg.input_tokens.unwrap_or(0),
                    output_tokens: msg.output_tokens.unwrap_or(0),
                    cache_read_tokens: msg.cache_read_tokens,
                    cache_write_tokens: msg.cache_write_tokens,
                }),
                tokens_per_second: msg.tokens_per_second,
                file_diffs: file_diffs.map(|diffs| {
                    diffs
                        .into_iter()
                        .map(|d| ApiFileDiff {
                            path: d.file,
                            status: d.status.unwrap_or_else(|| "modified".to_string()),
                            additions: d.additions,
                            deletions: d.deletions,
                        })
                        .collect()
                }),
            }
        })
        .collect();

    crate::log_debug!(
        "Listed {} messages for session {}",
        messages.len(),
        session_id
    );
    Ok(Json(ListMessagesResponse { messages, todos }))
}

/// Send a message to the session
pub async fn send_message(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
    Json(body): Json<SendMessageRequest>,
) -> WebResult<(StatusCode, Json<SendMessageResponse>)> {
    crate::log_info!(
        "Sending message to session {} (content length: {})",
        session_id,
        body.content.len()
    );

    // Check if session exists
    let record = {
        let store = state.store.lock().await;
        store.load_session_record(session_id)?.ok_or_else(|| {
            crate::log_warn!("Session {} not found when sending message", session_id);
            AppError::NotFound(format!("Session {} not found", session_id))
        })?
    };

    // Generate request ID
    let request_id = rand::random::<u64>();
    crate::log_debug!(
        "Generated request ID {} for session {}",
        request_id,
        session_id
    );

    // Track this request
    state.track_request(session_id, request_id).await;

    // Parse session mode
    let mode = body
        .mode
        .as_deref()
        .and_then(|m| match m {
            "plan" => Some(SessionMode::Plan),
            "build" => Some(SessionMode::Build),
            _ => None,
        })
        .unwrap_or(SessionMode::Build);

    // Get existing messages for context (before appending user message)
    let existing_messages = {
        let store = state.store.lock().await;
        store.load_messages(session_id)?
    };

    // Detect mode transition and inject switch reminder
    let prev_mode = existing_messages
        .iter()
        .rev()
        .find(|m| m.mode.is_some())
        .and_then(|m| m.mode);

    let content = if let Some(prev) = prev_mode {
        if prev != mode {
            let reminder = match mode {
                SessionMode::Plan => prompts::plan_switch_reminder(),
                SessionMode::Build => prompts::build_switch_reminder(),
            };
            format!("{}\n\n{}", reminder, body.content)
        } else {
            body.content.clone()
        }
    } else {
        body.content.clone()
    };

    // Add user message to database (agent loop will inject
    // instructions and memory context before the LLM turn).
    let user_message = Message {
        id: Uuid::new_v4(),
        role: MessageRole::User,
        content: content.clone(),
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
        mode: Some(mode),
        rtk_rewritten: false,
        thinking_level: None,
    };
    {
        let store = state.store.lock().await;
        store.append_message(session_id, &user_message)?;
    }

    // Get config and tools
    let thinking_level = body
        .thinking_level
        .as_deref()
        .map(ThinkingLevelType::from_string)
        .unwrap_or_default();

    // Get model config
    let config = state.config.read().await;

    // Determine which provider and model to use
    let (provider_id, provider, model_id, model_config) =
        if let (Some(pid), Some(mid)) = (&body.provider_id, &body.model_id) {
            // Use the explicitly requested model
            let provider = config
                .provider(pid)
                .cloned()
                .ok_or_else(|| AppError::BadRequest(format!("Provider '{}' not found", pid)))?;
            let model_config = provider.models.get(mid).cloned().ok_or_else(|| {
                AppError::BadRequest(format!("Model '{}' not found for provider '{}'", mid, pid))
            })?;
            (pid.clone(), provider, mid.clone(), model_config)
        } else {
            // Use session's current model
            let provider = config
                .provider(&record.provider_id)
                .cloned()
                .ok_or_else(|| AppError::Internal("Provider not found".to_string()))?;
            let model_config = provider
                .models
                .get(&record.model_id)
                .cloned()
                .ok_or_else(|| AppError::Internal("Model not found".to_string()))?;
            (
                record.provider_id.clone(),
                provider,
                record.model_id.clone(),
                model_config,
            )
        };

    // Get API key from auth store
    let auth = state.auth.read().await;
    let api_key = auth
        .providers
        .get(&provider_id)
        .and_then(|entry| entry.api_key.clone())
        .filter(|value| !value.trim().is_empty());
    drop(auth);

    let mut model = crate::config::ActiveModel {
        provider_id: provider_id.clone(),
        provider_display_name: provider.display_name.clone(),
        model_id: model_id.clone(),
        request_model_id: model_config
            .request_model_id
            .clone()
            .unwrap_or_else(|| model_id.clone()),
        display_name: model_config.display_name.clone(),
        base_url: provider.base_url.clone(),
        api_key,
        api_type: match provider.api_type.as_deref() {
            Some("anthropic") => crate::config::ApiType::Anthropic,
            Some("openai_responses") => crate::config::ApiType::OpenAiResponses,
            Some("google_gemini") => crate::config::ApiType::GoogleGemini,
            _ => crate::config::ApiType::OpenAiChatCompletions,
        },
        temperature: model_config.temperature,
        context_window: model_config.context_window,
        max_output_tokens: model_config.max_output_tokens,
        supports_images: model_config.supports_images,
        system_prompt: model_config.system_prompt.clone().unwrap_or_default(),
        extra_body: model_config.extra_body.clone(),
        thinking_level: thinking_level.clone(),
    };

    // ── Load or compose the static system prompt ─────────────────────────
    {
        let store = state.store.lock().await;
        let stored_system_prompt = store.load_session_system_prompt(session_id)?;
        if !stored_system_prompt.is_empty() {
            model.system_prompt = stored_system_prompt;
        } else {
            // New session — compose static prompt and persist.
            let composed = state
                .agent
                .compose_static_system_prompt(&model.system_prompt);
            if let Err(e) = store.update_session_system_prompt(session_id, &composed) {
                crate::log_warn!("failed to persist static system prompt: {}", e);
            }
            model.system_prompt = composed;
        }
    }

    // Create channel for agent events (BackendEvent → AppEvent forwarding)
    let (tx, mut rx) = unbounded_channel::<BackendEvent>();
    let event_bus = state.event_bus.clone();

    // Clone the agent runtime and other shared state for the spawned tasks
    let mut agent = state.agent.clone();
    let state_for_spawn = state.clone();

    tokio::spawn(async move {
        // Create a context manager for message preprocessing
        let mut context_manager = ContextManager::new();

        crate::log_info!("Starting agent loop for session {}", session_id);

        if let Err(e) = agent
            .run_agent_loop(crate::agent::runtime::AgentLoopConfig {
                session_id,
                model,
                context_manager: &mut context_manager,
                mode,
                thinking_level,
                event_tx: tx,
                cancel_token: None,
            })
            .await
        {
            crate::log_error!("Agent loop failed for session {}: {}", session_id, e);
        }

        // Clean up request tracking
        state_for_spawn.remove_request(session_id).await;
        crate::log_info!("Agent loop completed for session {}", session_id);
    });

    // Spawn task to forward BackendEvents to SSE AppEvents.
    // Persistence of assistant messages and tool results is handled
    // internally by AgentRuntime::run_agent_loop.
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
                BackendEvent::ReasoningDelta {
                    session_id,
                    request_id,
                    content,
                } => Some(AppEvent::ReasoningChunk {
                    session_id,
                    request_id,
                    content,
                }),
                BackendEvent::Finished {
                    session_id,
                    request_id,
                    ..
                } => Some(AppEvent::MessageComplete {
                    session_id,
                    request_id,
                }),
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
                    diff: result.metadata.diff.clone(),
                    filepath: result.metadata.filepath.clone(),
                    rtk_rewritten: result.rtk_rewritten,
                }),
                BackendEvent::UsageStats {
                    session_id,
                    request_id,
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    duration_ms,
                    ..
                } => {
                    let tokens_per_second = duration_ms.and_then(|ms| {
                        if ms > 0 {
                            Some(total_tokens as f32 / (ms as f32 / 1000.0))
                        } else {
                            None
                        }
                    });
                    Some(AppEvent::UsageStats {
                        session_id,
                        request_id,
                        input_tokens,
                        output_tokens,
                        total_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        tokens_per_second,
                    })
                }
                BackendEvent::ShellOutput {
                    session_id,
                    content,
                    finished,
                    exit_code,
                } => Some(AppEvent::ShellOutput {
                    session_id,
                    content,
                    finished,
                    exit_code,
                }),
                BackendEvent::Retrying {
                    session_id,
                    request_id,
                    attempt,
                    max_attempts,
                    reason,
                    retry_after_secs,
                } => Some(AppEvent::Retrying {
                    session_id,
                    request_id,
                    attempt,
                    max_attempts,
                    reason,
                    retry_after_secs,
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
    crate::log_info!(
        "Abort request for session {} request {}",
        session_id,
        body.request_id
    );

    // Check if there's an active request
    let active = state.get_active_request(session_id).await;

    if active == Some(body.request_id) {
        state.remove_request(session_id).await;
        state.event_bus.publish(AppEvent::Aborted {
            session_id,
            request_id: body.request_id,
        });
        crate::log_info!(
            "Aborted request {} for session {}",
            body.request_id,
            session_id
        );
    } else {
        crate::log_warn!(
            "No active request {} found for session {}",
            body.request_id,
            session_id
        );
    }

    Ok(StatusCode::OK)
}

/// Revert request
#[derive(Deserialize)]
pub struct RevertRequest {
    pub message_id: Uuid,
}

/// Revert response
#[derive(Serialize)]
pub struct RevertResponse {
    pub success: bool,
    pub reverted_to_message_id: String,
    pub hidden_message_count: usize,
}

/// Revert session state to a specific user message.
/// This will restore files to the state before that message was sent,
/// and hide all messages after it (they can be restored via redo).
pub async fn revert_to_message(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
    Json(body): Json<RevertRequest>,
) -> WebResult<Json<RevertResponse>> {
    crate::log_info!(
        "Revert request for session {} to message {}",
        session_id,
        body.message_id
    );

    let store = state.store.lock().await;

    // Load session to verify it exists
    let _session = store
        .load_session_record(session_id)?
        .ok_or_else(|| AppError::NotFound(format!("Session {} not found", session_id)))?;

    // Load all messages for the session
    let messages = store.load_messages(session_id)?;

    // Find the target message and verify it's a user message
    let target_message = messages
        .iter()
        .find(|m| m.id == body.message_id)
        .ok_or_else(|| AppError::NotFound(format!("Message {} not found", body.message_id)))?;

    if !matches!(target_message.role, MessageRole::User) {
        return Err(AppError::BadRequest(
            "Can only revert to user messages".to_string(),
        ));
    }

    // Collect patches from messages after the target
    let patches = crate::shared::undo::collect_patches_after_message(&messages, body.message_id)?;
    crate::log_info!("Collected {} patches to revert", patches.len());

    // Capture redo snapshot (current state) if not already saved
    let redo_snapshot = match store.load_redo_snapshot(session_id)? {
        Some(existing) => {
            crate::log_info!("Using existing redo snapshot");
            existing
        }
        None => {
            crate::log_info!("Capturing new redo snapshot");
            drop(store); // Release lock before async operation
            match state.snapshot.track().await {
                Ok(Some(hash)) => {
                    crate::log_info!("Captured redo snapshot hash={}", hash);
                    hash
                }
                Ok(None) => {
                    crate::log_info!("No changes to snapshot");
                    String::new()
                }
                Err(error) => {
                    crate::log_warn!("Failed to capture redo snapshot: {}", error);
                    String::new()
                }
            }
        }
    };

    // Restore files using redo snapshot first (to undo previous reverts)
    let store = state.store.lock().await;
    if let Some(existing_snapshot) = store.load_redo_snapshot(session_id)? {
        crate::log_info!("Restoring redo snapshot");
        if let Err(error) = state.snapshot.restore(&existing_snapshot).await {
            crate::log_warn!("Failed to restore redo snapshot: {}", error);
        }
    }

    // Apply revert patches
    if !patches.is_empty() {
        crate::log_info!("Reverting {} patches", patches.len());
        if let Err(error) = state.snapshot.revert(&patches).await {
            crate::log_warn!("Revert partially failed: {}", error);
        }
    }

    // Calculate how many messages will be hidden
    let target_index = messages
        .iter()
        .position(|m| m.id == body.message_id)
        .unwrap_or(0);
    let hidden_count = messages.len().saturating_sub(target_index + 1);

    // Update revert marker in database
    store.set_revert_message_id(
        session_id,
        Some(body.message_id),
        if redo_snapshot.is_empty() {
            None
        } else {
            Some(&redo_snapshot)
        },
    )?;

    drop(store);

    // Publish event to notify clients
    state
        .event_bus
        .publish(AppEvent::MessagesUpdated { session_id });

    crate::log_info!(
        "Revert completed: session {} reverted to message {}, {} messages hidden",
        session_id,
        body.message_id,
        hidden_count
    );

    Ok(Json(RevertResponse {
        success: true,
        reverted_to_message_id: body.message_id.to_string(),
        hidden_message_count: hidden_count,
    }))
}

/// Redo response
#[derive(Serialize)]
pub struct RedoResponse {
    pub success: bool,
    pub message: String,
}

/// Redo the last undo — restore files from the redo snapshot and clear revert state.
pub async fn redo_last_undo(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
) -> WebResult<Json<RedoResponse>> {
    crate::log_info!("Redo request for session {}", session_id);

    let store = state.store.lock().await;

    // Load the redo snapshot
    let redo_snapshot = store.load_redo_snapshot(session_id)?;
    let Some(snapshot_hash) = redo_snapshot else {
        drop(store);
        return Ok(Json(RedoResponse {
            success: true,
            message: "Nothing to redo".to_string(),
        }));
    };

    // Restore files from the redo snapshot
    if !snapshot_hash.is_empty() {
        crate::log_info!("Restoring redo snapshot {}", snapshot_hash);
        if let Err(error) = state.snapshot.restore(&snapshot_hash).await {
            crate::log_warn!("Redo restore failed: {}", error);
        }
    }

    // Clear the revert state
    store.set_revert_message_id(session_id, None, None)?;
    drop(store);

    // Publish event to notify clients
    state
        .event_bus
        .publish(AppEvent::MessagesUpdated { session_id });

    crate::log_info!("Redo completed for session {}", session_id);

    Ok(Json(RedoResponse {
        success: true,
        message: "Redo complete".to_string(),
    }))
}

/// Response for compact session
#[derive(Serialize)]
pub struct CompactSessionResponse {
    pub request_id: u64,
}

/// Compact session context (analogous to TUI's /compact command).
pub async fn compact_session(
    State(mut state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
) -> WebResult<(StatusCode, Json<CompactSessionResponse>)> {
    crate::log_info!("Compacting session {}", session_id);

    // Load session record
    let record = {
        let store = state.store.lock().await;
        store
            .load_session_record(session_id)?
            .ok_or_else(|| AppError::NotFound(format!("Session {} not found", session_id)))?
    };

    // Load existing messages
    let messages = {
        let store = state.store.lock().await;
        store.load_messages(session_id)?
    };

    // Generate request ID
    let request_id = rand::random::<u64>();

    // Build conversation
    let mut conversation = Conversation::new(
        session_id,
        &record.workspace_root,
        &record.provider_id,
        &record.provider_display_name,
        &record.model_id,
        &record.model_display_name,
        &record.title,
    );
    conversation.messages = messages;
    if let Some(summary) = &record.context_summary {
        conversation.set_context_state(Some(summary.clone()), record.context_retained_from);
    }

    // Build context manager from existing state
    let mut context_manager =
        ContextManager::from_state(record.context_summary.clone(), record.context_retained_from);

    // Resolve active model
    let (provider_id, provider, model_id, model_config) = {
        let config = state.config.read().await;
        let provider = config
            .provider(&record.provider_id)
            .cloned()
            .ok_or_else(|| AppError::Internal("Provider not found".to_string()))?;
        let model_config = provider
            .models
            .get(&record.model_id)
            .cloned()
            .ok_or_else(|| AppError::Internal("Model not found".to_string()))?;
        (
            record.provider_id.clone(),
            provider,
            record.model_id.clone(),
            model_config,
        )
    };

    let active_model = crate::config::ActiveModel {
        provider_id: provider_id.clone(),
        provider_display_name: provider.display_name.clone(),
        model_id: model_id.clone(),
        request_model_id: model_config
            .request_model_id
            .clone()
            .unwrap_or_else(|| model_id.clone()),
        display_name: model_config.display_name.clone(),
        base_url: provider.base_url.clone(),
        api_key: None,
        api_type: match provider.api_type.as_deref() {
            Some("anthropic") => crate::config::ApiType::Anthropic,
            Some("openai_responses") => crate::config::ApiType::OpenAiResponses,
            Some("google_gemini") => crate::config::ApiType::GoogleGemini,
            _ => crate::config::ApiType::OpenAiChatCompletions,
        },
        temperature: model_config.temperature,
        context_window: model_config.context_window,
        max_output_tokens: model_config.max_output_tokens,
        supports_images: model_config.supports_images,
        system_prompt: model_config.system_prompt.clone().unwrap_or_default(),
        extra_body: model_config.extra_body.clone(),
        thinking_level: ThinkingLevelType::None,
    };

    // Sync the tool registry's active model so the tool list is
    // byte-for-byte identical to normal requests (preserving prefix cache).
    state.agent.tools.set_active_model(active_model.clone());
    let store = state.store.clone();
    let llm = state.llm_client.clone();
    let event_bus = state.event_bus.clone();
    let tools = state.agent.tool_definitions();

    // Spawn compaction in background
    tokio::spawn(async move {
        crate::log_info!(
            "Starting compaction background task for session {}",
            session_id
        );

        let prior_summary = context_manager.summary.clone();
        let prior_retained_from = context_manager.retained_from;

        let result = context_manager
            .compact(crate::context::CompactionConfig {
                llm: &llm,
                model: &active_model,
                conversation: &conversation,
                manual: true,
                stream_ctx: None,
                tools: &tools,
                mode: crate::prompts::SessionMode::Build,
            })
            .await;

        match result {
            Ok(true) => {
                if let Some(summary) = context_manager.summary.clone() {
                    // Create a System message with the compaction summary
                    let mut system_msg = Message::compaction(summary);
                    system_msg.metadata.prior_summary = prior_summary;
                    system_msg.metadata.prior_retained_from = Some(prior_retained_from);

                    // Persist the message
                    {
                        let store = store.lock().await;
                        if let Err(e) = store.append_message(session_id, &system_msg) {
                            crate::log_warn!("Failed to persist compaction message: {}", e);
                        }
                        if let Err(e) = store.update_session_context_state(
                            session_id,
                            context_manager.summary.as_deref(),
                            context_manager.retained_from,
                        ) {
                            crate::log_warn!("Failed to persist compacted context state: {}", e);
                        }
                    }
                    crate::log_info!("Compaction completed for session {}", session_id);
                } else {
                    crate::log_info!("Compaction produced no summary for session {}", session_id);
                }
            }
            Ok(false) => {
                crate::log_info!("Compaction skipped (not needed) for session {}", session_id);
            }
            Err(e) => {
                crate::log_warn!("Compaction failed for session {}: {}", session_id, e);
            }
        }

        // Notify SSE clients that messages have changed
        event_bus.publish(AppEvent::MessagesUpdated { session_id });
    });

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(CompactSessionResponse { request_id }),
    ))
}
