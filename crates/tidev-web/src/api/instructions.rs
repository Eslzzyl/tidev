use super::*;

#[derive(Debug, Serialize)]
pub(super) struct GlobalInstructionsResponse {
    path: String,
    exists: bool,
    content: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateGlobalInstructionsRequest {
    content: String,
}

fn global_instructions_response(
    state: &AppState,
    content: Option<String>,
) -> GlobalInstructionsResponse {
    let path = state.runtime.global_instruction_path();
    GlobalInstructionsResponse {
        path: display_path_with_tilde(&path),
        exists: content.is_some(),
        content: content.unwrap_or_default(),
    }
}

pub(super) async fn get_global_instructions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GlobalInstructionsResponse>, ApiError> {
    let content = state.runtime.load_global_instructions().map_err(|error| {
        ApiError::internal(format!("failed to read global instructions: {error}"))
    })?;
    Ok(Json(global_instructions_response(&state, content)))
}

pub(super) async fn update_global_instructions(
    State(state): State<Arc<AppState>>,
    Json(body): Json<UpdateGlobalInstructionsRequest>,
) -> Result<Json<GlobalInstructionsResponse>, ApiError> {
    state
        .runtime
        .save_global_instructions(&body.content)
        .map_err(|error| {
            ApiError::internal(format!("failed to save global instructions: {error}"))
        })?;
    Ok(Json(global_instructions_response(
        &state,
        Some(body.content),
    )))
}

pub(super) async fn delete_global_instructions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GlobalInstructionsResponse>, ApiError> {
    state
        .runtime
        .delete_global_instructions()
        .map_err(|error| {
            ApiError::internal(format!("failed to delete global instructions: {error}"))
        })?;
    Ok(Json(global_instructions_response(&state, None)))
}
