use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};

use crate::{error::WebResult, state::AppState};

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FileSuggestion {
    pub path: String,
    pub display: String,
    pub kind: String,
    pub matched_indices: Vec<usize>,
}

#[derive(Debug, Serialize)]
pub struct FileSearchResponse {
    pub suggestions: Vec<FileSuggestion>,
}

/// Search files in the workspace for @-mention completion
pub async fn search_files(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> WebResult<Json<FileSearchResponse>> {
    let query = params.q.unwrap_or_default();

    log::debug!("File search request: query='{}'", query);

    // Ensure background indexing is started
    state
        .file_search_index
        .ensure_background_indexing(&state.workspace_root);

    // Perform search
    let file_suggestions = state.file_search_index.search(&query);

    // Convert to API response format
    let suggestions: Vec<FileSuggestion> = file_suggestions
        .into_iter()
        .map(|s| FileSuggestion {
            path: s.path,
            display: s.display,
            kind: match s.kind {
                tidev_engine::shared::file_search::FileEntryKind::File => "file".to_string(),
                tidev_engine::shared::file_search::FileEntryKind::Directory => "directory".to_string(),
                tidev_engine::shared::file_search::FileEntryKind::Image => "image".to_string(),
            },
            matched_indices: s.matched_indices,
        })
        .collect();

    log::debug!("File search returned {} suggestions", suggestions.len());
    Ok(Json(FileSearchResponse { suggestions }))
}
