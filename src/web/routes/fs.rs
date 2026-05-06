use std::path::{Path, PathBuf};

use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::web::error::{AppError, WebResult};
use crate::web::state::AppState;

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

#[derive(Debug, Deserialize)]
pub struct WriteFileRequest {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct WriteFileResponse {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Deserialize)]
pub struct CreateItemRequest {
    pub path: String,
    #[serde(rename = "type")]
    pub item_type: String, // "file" or "directory"
}

#[derive(Debug, Serialize)]
pub struct CreateItemResponse {
    pub path: String,
    #[serde(rename = "type")]
    pub item_type: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameItemRequest {
    pub path: String,
    pub new_path: String,
}

#[derive(Debug, Serialize)]
pub struct RenameItemResponse {
    pub path: String,
    pub new_path: String,
}

#[derive(Debug, Deserialize)]
pub struct RemoveItemRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct RemoveItemResponse {
    pub path: String,
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
        AppError::NotFound(format!("Directory not found: {}", e))
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
        return Err(AppError::Forbidden(
            "Access denied: file is outside workspace".to_string(),
        ));
    }

    let metadata = fs::metadata(&target).await.map_err(|e| {
        AppError::NotFound(format!("File not found: {}", e))
    })?;

    if !metadata.is_file() {
        return Err(AppError::BadRequest(
            "Path is not a file".to_string(),
        ));
    }

    // Limit file size to 1MB for safety
    if metadata.len() > 1024 * 1024 {
        return Err(AppError::BadRequest(
            "File too large to read (>1MB)".to_string(),
        ));
    }

    let content = fs::read_to_string(&target).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot read file: {}", e))
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

/// Write content to a file (create or overwrite).
pub async fn write_file(
    State(state): State<AppState>,
    Json(params): Json<WriteFileRequest>,
) -> WebResult<Json<WriteFileResponse>> {
    let target = resolve_path_for_create(&state.workspace_root, &params.path)?;

    // Security: ensure the resolved path is within workspace
    if !target.starts_with(&state.workspace_root) {
        return Err(AppError::Forbidden(
            "Access denied: file is outside workspace".to_string(),
        ));
    }

    // Ensure parent directory exists
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            AppError::BadRequest(format!("Failed to create parent directory: {}", e))
        })?;
    }

    // Write the file
    fs::write(&target, &params.content).await.map_err(|e| {
        AppError::BadRequest(format!("Failed to write file: {}", e))
    })?;

    let metadata = fs::metadata(&target).await.map_err(|e| {
        AppError::NotFound(format!("File not found after write: {}", e))
    })?;

    Ok(Json(WriteFileResponse {
        path: params.path,
        size: metadata.len(),
    }))
}

/// Create a file or directory.
pub async fn create_item(
    State(state): State<AppState>,
    Json(params): Json<CreateItemRequest>,
) -> WebResult<Json<CreateItemResponse>> {
    let target = resolve_path_for_create(&state.workspace_root, &params.path)?;

    // Security: ensure the resolved path is within workspace
    if !target.starts_with(&state.workspace_root) {
        return Err(AppError::Forbidden(
            "Access denied: file is outside workspace".to_string(),
        ));
    }

    // Ensure parent directory exists
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            AppError::BadRequest(format!("Failed to create parent directory: {}", e))
        })?;
    }

    match params.item_type.as_str() {
        "file" => {
            if target.exists() {
                return Err(AppError::BadRequest("File already exists".to_string()));
            }
            fs::write(&target, "").await.map_err(|e| {
                AppError::BadRequest(format!("Failed to create file: {}", e))
            })?;
        }
        "directory" | "dir" => {
            if target.exists() {
                return Err(AppError::BadRequest("Directory already exists".to_string()));
            }
            fs::create_dir(&target).await.map_err(|e| {
                AppError::BadRequest(format!("Failed to create directory: {}", e))
            })?;
        }
        _ => {
            return Err(AppError::BadRequest(format!(
                "Invalid type '{}'. Use 'file' or 'directory'.",
                params.item_type
            )));
        }
    }

    Ok(Json(CreateItemResponse {
        path: params.path,
        item_type: params.item_type,
    }))
}

/// Rename or move a file/directory.
pub async fn rename_item(
    State(state): State<AppState>,
    Json(params): Json<RenameItemRequest>,
) -> WebResult<Json<RenameItemResponse>> {
    let source = resolve_path(&state.workspace_root, &params.path)?;
    let target = resolve_path_for_create(&state.workspace_root, &params.new_path)?;

    // Security: ensure both paths are within workspace
    if !source.starts_with(&state.workspace_root) {
        return Err(AppError::Forbidden(
            "Access denied: source is outside workspace".to_string(),
        ));
    }
    if !target.starts_with(&state.workspace_root) {
        return Err(AppError::Forbidden(
            "Access denied: target is outside workspace".to_string(),
        ));
    }

    if !source.exists() {
        return Err(AppError::NotFound("Source path not found".to_string()));
    }
    if target.exists() {
        return Err(AppError::BadRequest("Target already exists".to_string()));
    }

    // Ensure parent of target exists
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            AppError::BadRequest(format!("Failed to create parent directory: {}", e))
        })?;
    }

    fs::rename(&source, &target).await.map_err(|e| {
        AppError::BadRequest(format!("Failed to rename: {}", e))
    })?;

    Ok(Json(RenameItemResponse {
        path: params.path,
        new_path: params.new_path,
    }))
}

/// Remove a file or empty directory.
pub async fn remove_item(
    State(state): State<AppState>,
    Json(params): Json<RemoveItemRequest>,
) -> WebResult<Json<RemoveItemResponse>> {
    let target = resolve_path(&state.workspace_root, &params.path)?;

    // Security: ensure the resolved path is within workspace
    if !target.starts_with(&state.workspace_root) {
        return Err(AppError::Forbidden(
            "Access denied: path is outside workspace".to_string(),
        ));
    }

    if !target.exists() {
        return Err(AppError::NotFound("Path not found".to_string()));
    }

    let metadata = fs::metadata(&target).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot access path: {}", e))
    })?;

    if metadata.is_dir() {
        // Remove empty directory
        fs::remove_dir(&target).await.map_err(|e| {
            AppError::BadRequest(format!("Failed to remove directory (may not be empty): {}", e))
        })?;
    } else {
        fs::remove_file(&target).await.map_err(|e| {
            AppError::BadRequest(format!("Failed to remove file: {}", e))
        })?;
    }

    Ok(Json(RemoveItemResponse { path: params.path }))
}

#[derive(Debug, Deserialize)]
pub struct ReadBase64Params {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct ReadBase64Response {
    pub path: String,
    pub data: String,
    pub mime: String,
}

/// Read a file and return its content as base64 (for images, etc.).
pub async fn read_file_base64(
    State(state): State<AppState>,
    Query(params): Query<ReadBase64Params>,
) -> WebResult<Json<ReadBase64Response>> {
    let target = resolve_path(&state.workspace_root, &params.path)?;

    if !target.starts_with(&state.workspace_root) {
        return Err(AppError::Forbidden(
            "Access denied: file is outside workspace".to_string(),
        ));
    }

    let metadata = fs::metadata(&target).await.map_err(|e| {
        AppError::NotFound(format!("File not found: {}", e))
    })?;

    if !metadata.is_file() {
        return Err(AppError::BadRequest("Path is not a file".to_string()));
    }

    // Limit to 10MB for binary files
    if metadata.len() > 10 * 1024 * 1024 {
        return Err(AppError::BadRequest("File too large (>10MB)".to_string()));
    }

    let bytes = fs::read(&target).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot read file: {}", e))
    })?;

    let ext = Path::new(&params.path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    };

    let data = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &bytes,
    );

    Ok(Json(ReadBase64Response {
        path: params.path,
        data,
        mime: mime.to_string(),
    }))
}

/// Resolve a workspace-relative path, ensuring it stays within workspace.
fn resolve_path(workspace_root: &Path, requested: &str) -> WebResult<PathBuf> {
    let base = if requested.is_empty() || requested == "/" || requested == "." {
        workspace_root.to_path_buf()
    } else {
        // Strip any leading slash to make relative
        let clean = requested.trim_start_matches('/');
        workspace_root.join(clean)
    };

    // Canonicalize to resolve any ".." components
    let canonical = base.canonicalize().map_err(|e| {
        AppError::NotFound(format!("Path not found: {}", e))
    })?;

    // Verify it's still under workspace root
    if !canonical.starts_with(workspace_root) {
        return Err(AppError::Forbidden(
            "Access denied: path is outside workspace".to_string(),
        ));
    }

    Ok(canonical)
}

/// Resolve a path for creation (path does not need to exist yet).
/// Canonicalizes the parent directory to verify it's within workspace.
fn resolve_path_for_create(
    workspace_root: &Path,
    requested: &str,
) -> WebResult<PathBuf> {
    let clean = requested.trim_start_matches('/');
    let target = workspace_root.join(clean);

    // Canonicalize the parent directory to resolve ".." components
    let parent = target.parent().ok_or_else(|| {
        AppError::BadRequest("Invalid path".to_string())
    })?;

    let canonical_parent = parent.canonicalize().map_err(|e| {
        AppError::NotFound(format!("Parent directory not found: {}", e))
    })?;

    // Verify parent is within workspace
    if !canonical_parent.starts_with(workspace_root) {
        return Err(AppError::Forbidden(
            "Access denied: path is outside workspace".to_string(),
        ));
    }

    // Reconstruct the target path from canonical parent + filename
    let file_name = target.file_name().ok_or_else(|| {
        AppError::BadRequest("Invalid path".to_string())
    })?;

    Ok(canonical_parent.join(file_name))
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
