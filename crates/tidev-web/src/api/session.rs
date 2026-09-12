use super::*;

pub(super) async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<SessionListResponse>, ApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let cursor = match (query.before_updated_at.as_deref(), query.before_session_id) {
        (None, None) => None,
        (Some(updated_at), Some(session_id)) => {
            let updated_at = DateTime::parse_from_rfc3339(updated_at)
                .map_err(|_| ApiError::bad_request("invalid session cursor timestamp"))?
                .with_timezone(&Utc);
            Some((updated_at, session_id))
        }
        _ => {
            return Err(ApiError::bad_request(
                "session cursor requires both before_updated_at and before_session_id",
            ));
        }
    };
    let store = state.runtime.session_manager().store();
    let mut sessions = store.list_sessions_by_activity(
        limit + 1,
        cursor,
        query.q.as_deref(),
        query.workspace_root.as_deref(),
    )?;
    let next_cursor = if sessions.len() > limit as usize {
        sessions.pop();
        sessions.last().map(|session| SessionListCursorDto {
            updated_at: session.updated_at.to_rfc3339(),
            session_id: session.session_id,
        })
    } else {
        None
    };
    let workspace_roots = store.list_session_workspace_roots()?;
    let items = sessions
        .into_iter()
        .map(|session| session_dto(&state.runtime, session))
        .collect();
    Ok(Json(SessionListResponse {
        items,
        next_cursor,
        workspace_roots,
    }))
}

pub(super) async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<SessionCreatedResponse>, ApiError> {
    let title = request
        .title
        .as_deref()
        .unwrap_or("New conversation")
        .trim();
    if title.is_empty() {
        return Err(ApiError::bad_request("session title cannot be empty"));
    }
    let session_id = match request.workspace_root.as_deref() {
        Some(workspace_root) if !workspace_root.trim().is_empty() => {
            let workspace_root = canonical_workspace_path(workspace_root).await?;
            state
                .runtime
                .create_session_with_workspace(title, workspace_root)
                .await?
        }
        _ => state.runtime.create_default_session(title)?,
    };
    if let (Some(ref provider_id), Some(ref model_id)) = (request.provider_id, request.model_id) {
        let config = state.runtime.config();
        let auth = state.runtime.auth();
        if let Ok(model) = config.resolve_model_by_ids(&auth, provider_id, model_id) {
            let _ = state.runtime.session_manager().update_session_model(
                session_id,
                &model.provider_id,
                &model.provider_display_name,
                &model.model_id,
                &model.display_name,
            );
        }
    }
    let session = state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .ok_or_else(|| ApiError::not_found("created session is unavailable"))?;
    Ok(Json(SessionCreatedResponse {
        session: session_dto(&state.runtime, session),
    }))
}

pub(super) async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<SessionDto>, ApiError> {
    let session = state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    Ok(Json(session_dto(&state.runtime, session)))
}

pub(super) async fn update_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Json(request): Json<UpdateSessionRequest>,
) -> Result<Json<SessionDto>, ApiError> {
    let mut updated = false;
    if let Some(ref title) = request.title {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Err(ApiError::bad_request("session title cannot be empty"));
        }
        state.runtime.update_session_title(session_id, trimmed)?;
        updated = true;
    }
    if let (Some(ref provider_id), Some(ref model_id)) = (request.provider_id, request.model_id) {
        let config = state.runtime.config();
        let auth = state.runtime.auth();
        let model = config.resolve_model_by_ids(&auth, provider_id, model_id)?;
        state.runtime.session_manager().update_session_model(
            session_id,
            &model.provider_id,
            &model.provider_display_name,
            &model.model_id,
            &model.display_name,
        )?;
        updated = true;
    }
    if !updated {
        return Err(ApiError::bad_request("no fields to update"));
    }
    get_session(State(state), Path(session_id)).await
}

pub(super) async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    if state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .is_none()
    {
        return Err(ApiError::not_found("session not found"));
    }
    if state.runtime.is_session_busy(session_id) {
        return Err(ApiError::conflict(
            "stop the active conversation before deleting it",
        ));
    }
    state
        .runtime
        .session_manager()
        .store()
        .delete_session(session_id)?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

pub(super) async fn messages(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<MessagesResponse>, ApiError> {
    if state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .is_none()
    {
        return Err(ApiError::not_found("session not found"));
    }
    let messages = state
        .runtime
        .session_manager()
        .load_session_messages(session_id)?
        .into_iter()
        .map(|item| MessageDto {
            message: item.message,
            app_data: item.app_data,
        })
        .collect();
    Ok(Json(MessagesResponse { messages }))
}

pub(super) async fn session_diffs(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Query(query): Query<SessionDiffQuery>,
) -> Result<Json<SessionDiffsResponse>, ApiError> {
    let session = state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .ok_or_else(|| ApiError::not_found("session not found"))?;

    let empty_response = || {
        Json(SessionDiffsResponse {
            files: Vec::new(),
            total_additions: 0,
            total_deletions: 0,
        })
    };

    let Some(from_hash) = session.snapshot_start_hash.as_deref() else {
        return Ok(empty_response());
    };

    let messages = state
        .runtime
        .session_manager()
        .load_session_messages(session_id)?;
    let persisted_to_hash = messages.iter().rev().find_map(|message| {
        message
            .app_data
            .file_diffs
            .as_deref()
            .filter(|value| !value.is_empty())
            .and(message.app_data.snapshot_hash.as_deref())
    });
    let to_hash = query
        .to_hash
        .as_deref()
        .unwrap_or(persisted_to_hash.unwrap_or_default());
    if to_hash.is_empty() {
        return Ok(empty_response());
    }
    if to_hash.len() > 128 || !to_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request("invalid snapshot hash"));
    }

    let workspace = state.runtime.workspace_for(&session.workspace_root).await?;
    let Some(snapshot) = workspace.snapshot() else {
        return Ok(empty_response());
    };

    let diffs = snapshot.diff_full(from_hash, to_hash).await?;
    let total_additions = diffs.iter().map(|file| file.additions).sum();
    let total_deletions = diffs.iter().map(|file| file.deletions).sum();
    let files = diffs
        .into_iter()
        .map(|file| SessionFileDiffResponse {
            path: file.file,
            status: file.status,
            additions: file.additions,
            deletions: file.deletions,
            diff: file.patch,
        })
        .collect();

    Ok(Json(SessionDiffsResponse {
        files,
        total_additions,
        total_deletions,
    }))
}

pub(super) async fn todos(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<TodosResponse>, ApiError> {
    if state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .is_none()
    {
        return Err(ApiError::not_found("session not found"));
    }
    let todos = state
        .runtime
        .session_manager()
        .store()
        .load_todos(session_id)?
        .into_iter()
        .map(|todo| TodoDto {
            content: todo.content,
            status: todo.status,
        })
        .collect();
    Ok(Json(TodosResponse { todos }))
}

pub(super) async fn submit_prompt(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Json(request): Json<PromptRequest>,
) -> Result<Json<PromptResponse>, ApiError> {
    if request.content.trim().is_empty() && request.attachments.is_empty() {
        return Err(ApiError::bad_request("prompt content cannot be empty"));
    }
    if state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .is_none()
    {
        return Err(ApiError::not_found("session not found"));
    }
    let target_model = if let (Some(pid), Some(mid)) = (&request.provider_id, &request.model_id) {
        let config = state.runtime.config();
        let auth = state.runtime.auth();
        let model = config.resolve_model_by_ids(&auth, pid, mid)?;
        state.runtime.session_manager().update_session_model(
            session_id,
            &model.provider_id,
            &model.provider_display_name,
            &model.model_id,
            &model.display_name,
        )?;
        model
    } else if let Ok(Some(session)) = state.runtime.session_manager().load_session(session_id)
        && let Ok(model) = state.runtime.config().resolve_model_by_ids(
            &state.runtime.auth(),
            &session.provider_id,
            &session.model_id,
        )
    {
        model
    } else {
        state.runtime.active_model()
    };

    let attachments = request
        .attachments
        .into_iter()
        .map(PromptImageAttachmentRequest::into_message_attachment)
        .collect::<Result<Vec<_>, _>>()?;
    if !attachments.is_empty() && !target_model.supports_images {
        return Err(ApiError::bad_request(
            "current model does not support image attachments",
        ));
    }
    let mut submission =
        PromptSubmission::new(request.content, request.mode.unwrap_or(Mode::Build));
    submission.attachments = attachments;
    if let Some(message_id) = request.message_id {
        submission.message_id = message_id;
    }
    submission.thinking_level = request
        .thinking_level
        .as_deref()
        .map(tidev_llm::reasoning::ThinkingLevelType::from_string);
    let receipt = state
        .runtime
        .submit_prompt_submission(session_id, submission)
        .await?;
    Ok(Json(PromptResponse {
        message_id: receipt.message_id,
        duplicate: receipt.duplicate,
    }))
}

pub(super) async fn cancel_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state.runtime.cancel_session(session_id).await;
    Ok(Json(AcceptedResponse { accepted: true }))
}

pub(super) async fn retry_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Json(request): Json<RetryRequest>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    if state
        .runtime
        .session_manager()
        .load_session(session_id)?
        .is_none()
    {
        return Err(ApiError::not_found("session not found"));
    }
    state
        .runtime
        .retry_session(session_id, request.message_id)
        .await?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

pub(super) async fn revert(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Json(request): Json<RevertRequest>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state.runtime.revert(session_id, request.message_id).await?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

pub(super) async fn redo(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state.runtime.cancel_session(session_id).await;
    state.runtime.redo(session_id).await?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

pub(super) async fn fork_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Json(request): Json<ForkRequest>,
) -> Result<Json<SessionDto>, ApiError> {
    let new_session_id =
        state
            .runtime
            .fork_session(session_id, request.message_id, request.title)?;
    let record = state
        .runtime
        .session_manager()
        .load_session(new_session_id)?
        .ok_or_else(|| ApiError::not_found("forked session not found"))?;
    Ok(Json(SessionDto {
        session_id: record.session_id,
        parent_session_id: record.parent_session_id,
        workspace_root: record.workspace_root,
        provider_id: record.provider_id,
        provider_display_name: record.provider_display_name,
        model_id: record.model_id,
        model_display_name: record.model_display_name,
        title: record.title,
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
        status: record.status,
        ended_at: record.ended_at.map(|time| time.to_rfc3339()),
        context_summary: record.context_summary,
        context_retained_from: record.context_retained_from,
        busy: false,
    }))
}

pub(super) async fn compact(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state.runtime.compact_session(session_id, None).await?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

pub(super) async fn search_files(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FileSearchQuery>,
) -> Json<serde_json::Value> {
    let q = query.query.or(query.q).unwrap_or_default();
    let workspace = state.runtime.workspace_root().clone();
    let index = state.runtime.file_search_index();
    index.ensure_background_indexing(&workspace);
    let suggestions = index.search(&q);
    let limited = suggestions.into_iter().take(20).collect::<Vec<_>>();
    Json(serde_json::json!({ "files": limited }))
}
