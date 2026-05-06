use std::path::{Path, PathBuf};

use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::web::{error::WebResult, state::AppState};

#[derive(Debug, Deserialize)]
pub struct ListDirParams {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub is_symlink: bool,
    pub size: Option<u64>,
    pub modified: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListDirResponse {
    pub directory: String,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ReadFileParams {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct ReadFileResponse {
    pub content: String,
    pub path: String,
    pub language: Option<String>,
    pub line_count: usize,
    pub size: u64,
}

/// List directory contents (walk one level deep).
pub async fn list_directory(
    State(state): State<AppState>,
    Query(params): Query<ListDirParams>,
) -> WebResult<Json<ListDirResponse>> {
    let requested = params.path.unwrap_or_default();
    let target = resolve_path(&state.workspace_root, &requested)?;
    let directory = target.to_string_lossy().to_string();

    let mut entries = Vec::new();
    let mut read_dir = fs::read_dir(&target).await.map_err(|e| {
        crate::web::error::AppError::NotFound(format!("Directory not found: {}", e))
    })?;

    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip hidden files
        if name.starts_with('.') {
            continue;
        }
        let rel_path = entry.path().strip_prefix(&state.workspace_root)
            .unwrap_or(&entry.path())
            .to_string_lossy()
            .to_string();

        let metadata = entry.metadata().await.ok();
        entries.push(DirectoryEntry {
            name,
            path: rel_path,
            is_directory: entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false),
            is_symlink: entry.file_type().await.map(|t| t.is_symlink()).unwrap_or(false),
            size: metadata.as_ref().and_then(|m| {
                if m.is_file() { Some(m.len()) } else { None }
            }),
            modified: metadata.and_then(|m| {
                m.modified().ok().map(|t| {
                    let dt: chrono::DateTime<chrono::Local> = t.into();
                    dt.format("%Y-%m-%d %H:%M:%S").to_string()
                })
            }),
        });
    }

    // Sort: directories first, then files, alphabetically
    entries.sort_by(|a, b| {
        if a.is_directory != b.is_directory {
            b.is_directory.cmp(&a.is_directory)
        } else {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        }
    });

    Ok(Json(ListDirResponse { directory, entries }))
}

/// Read a file's content.
pub async fn read_file(
    State(state): State<AppState>,
    Query(params): Query<ReadFileParams>,
) -> WebResult<Json<ReadFileResponse>> {
    let target = resolve_path(&state.workspace_root, &params.path)?;

    // Security: ensure the resolved path is within workspace
    if !target.starts_with(&state.workspace_root) {
        return Err(crate::web::error::AppError::Forbidden(
            "Access denied: file is outside workspace".to_string(),
        ));
    }

    let metadata = fs::metadata(&target).await.map_err(|e| {
        crate::web::error::AppError::NotFound(format!("File not found: {}", e))
    })?;

    if !metadata.is_file() {
        return Err(crate::web::error::AppError::BadRequest(
            "Path is not a file".to_string(),
        ));
    }

    // Limit file size to 1MB for safety
    if metadata.len() > 1024 * 1024 {
        return Err(crate::web::error::AppError::BadRequest(
            "File too large to read (>1MB)".to_string(),
        ));
    }

    let content = fs::read_to_string(&target).await.map_err(|e| {
        crate::web::error::AppError::BadRequest(format!("Cannot read file: {}", e))
    })?;

    let line_count = content.lines().count();
    let language = detect_language(&params.path);

    Ok(Json(ReadFileResponse {
        content,
        path: params.path,
        language,
        line_count,
        size: metadata.len(),
    }))
}

/// Resolve a workspace-relative path, ensuring it stays within workspace.
fn resolve_path(workspace_root: &Path, requested: &str) -> crate::web::error::WebResult<PathBuf> {
    let base = if requested.is_empty() || requested == "/" || requested == "." {
        workspace_root.to_path_buf()
    } else {
        // Strip any leading slash to make relative
        let clean = requested.trim_start_matches('/');
        workspace_root.join(clean)
    };

    // Canonicalize to resolve any ".." components
    let canonical = base.canonicalize().map_err(|e| {
        crate::web::error::AppError::NotFound(format!("Path not found: {}", e))
    })?;

    // Verify it's still under workspace root
    if !canonical.starts_with(workspace_root) {
        return Err(crate::web::error::AppError::Forbidden(
            "Access denied: path is outside workspace".to_string(),
        ));
    }

    Ok(canonical)
}

/// Detect language from file extension.
fn detect_language(path: &str) -> Option<String> {
    let ext = Path::new(path).extension()?.to_str()?;
    let lang = match ext {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "rb" => "ruby",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" => "cpp",
        "cs" => "csharp",
        "css" | "scss" | "sass" | "less" => "css",
        "html" | "htm" => "html",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "sql" => "sql",
        "sh" | "bash" | "zsh" => "bash",
        "dockerfile" | "Dockerfile" => "dockerfile",
        "vue" => "vue",
        "svelte" => "svelte",
        "astro" => "astro",
        "tex" => "latex",
        "xml" | "svg" => "xml",
        _ => return None,
    };
    Some(lang.to_string())
}
