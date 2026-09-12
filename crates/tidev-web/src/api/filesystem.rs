use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct ListDirParams {
    path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DirectoryEntry {
    name: String,
    path: String,
    is_directory: bool,
    is_symlink: bool,
    size: Option<u64>,
    modified: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ListDirResponse {
    directory: String,
    entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReadFileParams {
    path: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ReadFileResponse {
    content: String,
    path: String,
    language: Option<String>,
    line_count: usize,
    size: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct WriteFileRequest {
    path: String,
    content: String,
}

#[derive(Debug, Serialize)]
pub(super) struct WriteFileResponse {
    path: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateItemRequest {
    path: String,
    #[serde(rename = "type")]
    item_type: String,
}

#[derive(Debug, Serialize)]
pub(super) struct CreateItemResponse {
    path: String,
    #[serde(rename = "type")]
    item_type: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RenameItemRequest {
    path: String,
    new_path: String,
}

#[derive(Debug, Serialize)]
pub(super) struct RenameItemResponse {
    path: String,
    new_path: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RemoveItemRequest {
    path: String,
}

#[derive(Debug, Serialize)]
pub(super) struct RemoveItemResponse {
    path: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReadBase64Params {
    path: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ReadBase64Response {
    path: String,
    data: String,
    mime: String,
}

pub(super) async fn fs_list(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListDirParams>,
) -> Result<Json<ListDirResponse>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let requested = params.path.unwrap_or_default();
    let target = resolve_path(&workspace_root, &requested)?;
    let directory = target.to_string_lossy().to_string();
    let mut entries = Vec::new();
    let mut read_dir = fs::read_dir(&target)
        .await
        .map_err(|e| ApiError::not_found(format!("Directory not found: {e}")))?;
    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let rel_path = entry
            .path()
            .strip_prefix(&workspace_root)
            .unwrap_or(&entry.path())
            .to_string_lossy()
            .to_string();
        let metadata = entry.metadata().await.ok();
        let is_symlink = entry
            .file_type()
            .await
            .map(|ft| ft.is_symlink())
            .unwrap_or(false);
        let is_directory = entry
            .file_type()
            .await
            .map(|ft| ft.is_dir())
            .unwrap_or(false);
        entries.push(DirectoryEntry {
            name,
            path: rel_path,
            is_directory,
            is_symlink,
            size: metadata
                .as_ref()
                .and_then(|m| if m.is_file() { Some(m.len()) } else { None }),
            modified: metadata
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| {
                    let secs = d.as_secs();
                    format!("{}", secs)
                }),
        });
    }
    entries.sort_by(|a, b| match (a.is_directory, b.is_directory) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(Json(ListDirResponse { directory, entries }))
}

pub(super) async fn fs_read(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ReadFileParams>,
) -> Result<Json<ReadFileResponse>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let target = resolve_path(&workspace_root, &params.path)?;
    let metadata = fs::metadata(&target)
        .await
        .map_err(|e| ApiError::not_found(format!("File not found: {e}")))?;
    if metadata.is_dir() {
        return Err(ApiError::bad_request("Path is a directory"));
    }
    if metadata.len() > 10 * 1024 * 1024 {
        return Err(ApiError::bad_request("File too large (max 10MB)"));
    }
    let content = fs::read_to_string(&target)
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to read file: {e}")))?;
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

pub(super) async fn fs_write(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WriteFileRequest>,
) -> Result<Json<WriteFileResponse>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let target = resolve_path(&workspace_root, &body.path)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::bad_request(format!("Failed to create parent dir: {e}")))?;
    }
    fs::write(&target, body.content.as_bytes())
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to write file: {e}")))?;
    let size = body.content.len() as u64;
    Ok(Json(WriteFileResponse {
        path: body.path,
        size,
    }))
}

pub(super) async fn fs_create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateItemRequest>,
) -> Result<Json<CreateItemResponse>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let target = resolve_path_for_create(&workspace_root, &body.path)?;
    if body.item_type == "directory" {
        fs::create_dir_all(&target)
            .await
            .map_err(|e| ApiError::bad_request(format!("Failed to create directory: {e}")))?;
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| ApiError::bad_request(format!("Failed to create parent dir: {e}")))?;
        }
        fs::write(&target, b"")
            .await
            .map_err(|e| ApiError::bad_request(format!("Failed to create file: {e}")))?;
    }
    Ok(Json(CreateItemResponse {
        path: body.path,
        item_type: body.item_type,
    }))
}

pub(super) async fn fs_rename(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RenameItemRequest>,
) -> Result<Json<RenameItemResponse>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let source = resolve_path(&workspace_root, &body.path)?;
    let dest = resolve_path_for_create(&workspace_root, &body.new_path)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::bad_request(format!("Failed to create dest parent: {e}")))?;
    }
    fs::rename(&source, &dest)
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to rename: {e}")))?;
    Ok(Json(RenameItemResponse {
        path: body.path,
        new_path: body.new_path,
    }))
}

pub(super) async fn fs_remove(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RemoveItemRequest>,
) -> Result<Json<RemoveItemResponse>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let target = resolve_path(&workspace_root, &params.path)?;
    let metadata = fs::metadata(&target)
        .await
        .map_err(|e| ApiError::not_found(format!("Path not found: {e}")))?;
    if metadata.is_dir() {
        fs::remove_dir_all(&target)
            .await
            .map_err(|e| ApiError::bad_request(format!("Failed to remove dir: {e}")))?;
    } else {
        fs::remove_file(&target)
            .await
            .map_err(|e| ApiError::bad_request(format!("Failed to remove file: {e}")))?;
    }
    Ok(Json(RemoveItemResponse { path: params.path }))
}

pub(super) async fn fs_read_base64(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ReadBase64Params>,
) -> Result<Json<ReadBase64Response>, ApiError> {
    let workspace_root = state.runtime.workspace_root().clone();
    let target = resolve_path(&workspace_root, &params.path)?;
    let bytes = fs::read(&target)
        .await
        .map_err(|e| ApiError::not_found(format!("File not found: {e}")))?;
    if bytes.len() > 10 * 1024 * 1024 {
        return Err(ApiError::bad_request("File too large"));
    }
    let mime = mime_guess::from_path(&target)
        .first_or_octet_stream()
        .to_string();
    let data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    Ok(Json(ReadBase64Response {
        path: params.path,
        data,
        mime,
    }))
}

pub(super) fn resolve_path(workspace_root: &StdPath, requested: &str) -> Result<PathBuf, ApiError> {
    let base = if requested.is_empty() || requested == "/" || requested == "." {
        workspace_root.to_path_buf()
    } else {
        let clean = requested.trim_start_matches('/');
        workspace_root.join(clean)
    };
    let canonical = base
        .canonicalize()
        .map_err(|e| ApiError::not_found(format!("Path not found: {e}")))?;
    if !canonical.starts_with(workspace_root) {
        return Err(ApiError::forbidden(
            "Access denied: path is outside workspace",
        ));
    }
    Ok(canonical)
}

pub(super) fn resolve_path_for_create(
    workspace_root: &StdPath,
    requested: &str,
) -> Result<PathBuf, ApiError> {
    let clean = requested.trim_start_matches('/');
    let target = workspace_root.join(clean);
    let parent = target
        .parent()
        .ok_or_else(|| ApiError::bad_request("Invalid path"))?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|e| ApiError::not_found(format!("Parent directory not found: {e}")))?;
    if !canonical_parent.starts_with(workspace_root) {
        return Err(ApiError::forbidden(
            "Access denied: path is outside workspace",
        ));
    }
    let file_name = target
        .file_name()
        .ok_or_else(|| ApiError::bad_request("Invalid path"))?;
    Ok(canonical_parent.join(file_name))
}

pub(super) fn detect_language(path: &str) -> Option<String> {
    let ext = StdPath::new(path).extension()?.to_str()?;
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
