use super::*;

pub(super) fn workspace_input_path(raw_path: &str) -> Result<PathBuf, ApiError> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return Err(ApiError::bad_request("workspace path is required"));
    }
    let path = expand_tilde(StdPath::new(raw_path))
        .map_err(|error| ApiError::bad_request(format!("invalid workspace path: {error}")))?;
    if !path.is_absolute() {
        return Err(ApiError::bad_request(
            "workspace path must be absolute or start with ~/",
        ));
    }
    Ok(path)
}

pub(super) fn workspace_completion_path(path: &StdPath, use_tilde: bool) -> String {
    if use_tilde {
        display_path_with_tilde(path)
    } else {
        path.to_string_lossy().to_string()
    }
}

pub(super) async fn canonical_workspace_path(raw_path: &str) -> Result<PathBuf, ApiError> {
    let path = workspace_input_path(raw_path)?;
    let canonical = fs::canonicalize(&path)
        .await
        .map_err(|e| ApiError::not_found(format!("workspace directory not found: {e}")))?;
    let metadata = fs::metadata(&canonical)
        .await
        .map_err(|e| ApiError::not_found(format!("workspace directory not found: {e}")))?;
    if !metadata.is_dir() {
        return Err(ApiError::bad_request("workspace path must be a directory"));
    }
    Ok(canonical)
}

pub(super) fn workspace_name(path: &StdPath) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

pub(super) fn workspace_git_branch(path: &PathBuf) -> Option<String> {
    let branch = run_git(&["rev-parse", "--abbrev-ref", "HEAD"], path)
        .ok()?
        .trim()
        .to_string();
    if branch.is_empty() {
        return None;
    }
    if branch != "HEAD" {
        return Some(branch);
    }

    let short_sha = run_git(&["rev-parse", "--short", "HEAD"], path)
        .ok()?
        .trim()
        .to_string();
    (!short_sha.is_empty()).then(|| format!("@{short_sha}"))
}

pub(super) async fn workspace_context(
    Query(query): Query<WorkspacePathQuery>,
) -> Result<Json<WorkspaceContextResponse>, ApiError> {
    let workspace_root = canonical_workspace_path(&query.path).await?;
    let git_path = workspace_root.clone();
    let git_branch = tokio::task::spawn_blocking(move || workspace_git_branch(&git_path))
        .await
        .map_err(|_| ApiError::internal("workspace git inspection failed"))?;
    Ok(Json(WorkspaceContextResponse {
        workspace_display: display_path_with_tilde(&workspace_root),
        workspace_name: workspace_name(&workspace_root),
        workspace_root: workspace_root.to_string_lossy().to_string(),
        git_branch,
    }))
}

pub(super) async fn workspace_complete(
    Query(query): Query<WorkspacePathQuery>,
) -> Result<Json<WorkspaceCompletionResponse>, ApiError> {
    let raw_path = query.path.trim();
    let input = workspace_input_path(raw_path)?;
    let use_tilde = raw_path == "~" || raw_path.starts_with("~/");
    let navigation_parent = input
        .parent()
        .filter(|parent| *parent != input)
        .map(|parent| workspace_completion_path(parent, use_tilde));

    let (parent, prefix) = if raw_path == "~" || raw_path.ends_with('/') {
        (input, String::new())
    } else {
        let parent = input
            .parent()
            .map(StdPath::to_path_buf)
            .ok_or_else(|| ApiError::bad_request("workspace path has no parent directory"))?;
        let prefix = input
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        (parent, prefix)
    };
    let parent = match fs::canonicalize(parent).await {
        Ok(parent) => parent,
        Err(_) => {
            return Ok(Json(WorkspaceCompletionResponse {
                directories: vec![],
                parent: navigation_parent,
            }));
        }
    };
    let mut entries = match fs::read_dir(&parent).await {
        Ok(entries) => entries,
        Err(_) => {
            return Ok(Json(WorkspaceCompletionResponse {
                directories: vec![],
                parent: navigation_parent,
            }));
        }
    };
    let mut directories = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        ApiError::internal(format!("failed to read workspace directory: {error}"))
    })? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(&prefix) {
            continue;
        }
        let is_directory = entry
            .metadata()
            .await
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        if is_directory {
            directories.push(workspace_completion_path(&entry.path(), use_tilde));
        }
    }
    directories.sort_unstable_by_key(|path| path.to_lowercase());
    directories.truncate(50);
    Ok(Json(WorkspaceCompletionResponse {
        directories,
        parent: navigation_parent,
    }))
}

pub(super) async fn get_workspace(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let workspace_root = state.runtime.workspace_root();
    let ws = workspace_root.to_string_lossy().to_string();
    let workspace_display = display_path_with_tilde(workspace_root);
    Json(serde_json::json!({
        "workspace_root": ws,
        "workspace_display": workspace_display
    }))
}

pub(super) async fn get_init() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "prompt": "Analyze the project and create AGENTS.md" }))
}

pub(super) async fn list_tools() -> Json<Vec<serde_json::Value>> {
    Json(vec![])
}

pub(super) async fn list_skills(State(state): State<Arc<AppState>>) -> Json<SkillListResponse> {
    let catalog = state.runtime.skills();
    let skills = catalog
        .all()
        .iter()
        .map(|s| SkillDto {
            name: s.name.clone(),
            description: s.description.clone(),
            directory: s.directory.to_string_lossy().to_string(),
            location: s.location.to_string_lossy().to_string(),
            is_bundled: s.directory.to_string_lossy().starts_with("__builtin__"),
            companion_files: s
                .companion_files
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            content: s.content.clone(),
            document: s.document.clone(),
        })
        .collect();
    Json(SkillListResponse { skills })
}

pub(super) async fn get_skill(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<SkillDto>, ApiError> {
    let catalog = state.runtime.skills();
    let s = catalog
        .get(&name)
        .ok_or_else(|| ApiError::not_found(format!("Skill '{name}' not found")))?;
    Ok(Json(SkillDto {
        name: s.name.clone(),
        description: s.description.clone(),
        directory: s.directory.to_string_lossy().to_string(),
        location: s.location.to_string_lossy().to_string(),
        is_bundled: s.directory.to_string_lossy().starts_with("__builtin__"),
        companion_files: s
            .companion_files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
        content: s.content.clone(),
        document: s.document.clone(),
    }))
}

pub(super) async fn get_skill_file(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(query): Query<SkillFileQuery>,
) -> Result<Json<SkillFileResponse>, ApiError> {
    let catalog = state.runtime.skills();
    let rel_path = query.path.as_deref().unwrap_or("");
    let content = catalog
        .read_skill_file(&name, rel_path, 1_000_000)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(SkillFileResponse { content }))
}
