use super::*;

// ── Git helpers and handlers (ported from last-full) ──────────────────────────

pub(super) fn git_workspace(state: &AppState) -> PathBuf {
    state.runtime.workspace_root().clone()
}

pub(super) fn run_git(args: &[&str], cwd: &PathBuf) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("Failed to run git: {e}"))?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|e| format!("Invalid UTF-8: {e}"))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr.trim().to_string())
    }
}

#[derive(Serialize)]
pub(super) struct GitStatusResponse {
    branch: String,
    sha: String,
    files: Vec<GitStatusFile>,
    ahead: i32,
    behind: i32,
}

#[derive(Serialize)]
pub(super) struct GitStatusFile {
    path: String,
    status: String,
    staged: bool,
}

#[derive(Serialize)]
pub(super) struct GitBranchesResponse {
    current: String,
    branches: Vec<GitBranchItem>,
}

#[derive(Serialize)]
pub(super) struct GitBranchItem {
    name: String,
    current: bool,
    remote: Option<String>,
}

#[derive(Serialize)]
pub(super) struct GitLogResponse {
    commits: Vec<GitCommitItem>,
    has_more: bool,
}

#[derive(Serialize)]
pub(super) struct GitCommitItem {
    sha: String,
    message: String,
    author: String,
    date: String,
}

#[derive(Serialize)]
pub(super) struct GitGraphResponse {
    commits: Vec<GitGraphCommit>,
}

#[derive(Serialize)]
pub(super) struct GitGraphCommit {
    sha: String,
    parents: Vec<String>,
    message: String,
    author: String,
    date: String,
    refs: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct GitShowResponse {
    sha: String,
    author: String,
    date: String,
    message: String,
    files: Vec<GitCommitFileInfo>,
    total_additions: usize,
    total_deletions: usize,
}

#[derive(Serialize)]
pub(super) struct GitCommitFileInfo {
    path: String,
    status: String,
    additions: usize,
    deletions: usize,
}

#[derive(Serialize)]
pub(super) struct GitFileDiffResponse {
    path: String,
    diff: String,
}

#[derive(Deserialize)]
pub(super) struct GitLogParams {
    skip: Option<usize>,
    count: Option<usize>,
}

#[derive(Deserialize)]
pub(super) struct GitGraphParams {
    count: Option<usize>,
}

#[derive(Deserialize)]
pub(super) struct GitDiffFileParams {
    path: String,
    staged: Option<bool>,
}

#[derive(Deserialize)]
pub(super) struct GitCommitRequest {
    message: String,
}

#[derive(Deserialize)]
pub(super) struct GitBranchCreateRequest {
    name: String,
    checkout: Option<bool>,
}

#[derive(Serialize)]
pub(super) struct GitMessageResponse {
    success: bool,
    message: String,
}

pub(super) async fn git_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GitStatusResponse>, ApiError> {
    let cwd = git_workspace(&state);
    let branch = run_git(&["rev-parse", "--abbrev-ref", "HEAD"], &cwd)
        .unwrap_or_else(|_| "unknown".to_string());
    let sha = run_git(&["rev-parse", "HEAD"], &cwd)
        .unwrap_or_default()
        .trim()
        .to_string();
    let status_output = run_git(&["status", "--porcelain", "-b"], &cwd).unwrap_or_default();
    let mut files = Vec::new();
    for line in status_output.lines().skip(1) {
        if line.len() < 3 {
            continue;
        }
        let status = line[0..2].trim().to_string();
        let path = line[3..].trim().to_string();
        let staged = line
            .chars()
            .next()
            .map(|c| c != ' ' && c != '?')
            .unwrap_or(false);
        files.push(GitStatusFile {
            path,
            status,
            staged,
        });
    }
    // ahead/behind parsing from first line like "## main...origin/main [ahead 1, behind 2]"
    let first_line = status_output.lines().next().unwrap_or("");
    let ahead = if first_line.contains("ahead") {
        first_line
            .split("ahead ")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    } else {
        0
    };
    let behind = if first_line.contains("behind") {
        first_line
            .split("behind ")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    } else {
        0
    };
    Ok(Json(GitStatusResponse {
        branch: branch.trim().to_string(),
        sha,
        files,
        ahead,
        behind,
    }))
}

pub(super) async fn git_branches(
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<GitBranchesResponse>, ApiError> {
    let cwd = git_workspace(&state);
    let _show_submodules = params
        .get("showSubmodules")
        .map(|v| v == "true")
        .unwrap_or(false);
    let current = run_git(&["rev-parse", "--abbrev-ref", "HEAD"], &cwd)
        .unwrap_or_default()
        .trim()
        .to_string();
    let output =
        run_git(&["branch", "--all", "--format=%(refname:short)"], &cwd).unwrap_or_default();
    let branches = output
        .lines()
        .map(|line| {
            let name = line.trim().to_string();
            let is_current = name == current;
            GitBranchItem {
                name: name.clone(),
                current: is_current,
                remote: if name.contains('/') { Some(name) } else { None },
            }
        })
        .collect();
    Ok(Json(GitBranchesResponse { current, branches }))
}

pub(super) async fn git_log(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GitLogParams>,
) -> Result<Json<GitLogResponse>, ApiError> {
    let cwd = git_workspace(&state);
    let skip = params.skip.unwrap_or(0);
    let count = params.count.unwrap_or(50);
    let output = run_git(
        &[
            "log",
            &format!("--skip={skip}"),
            &format!("-n{}", count + 1),
            "--pretty=format:%H|%s|%an|%aI",
        ],
        &cwd,
    )
    .unwrap_or_default();
    let lines: Vec<&str> = output.lines().collect();
    let has_more = lines.len() > count;
    let commits = lines
        .into_iter()
        .take(count)
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() < 4 {
                return None;
            }
            Some(GitCommitItem {
                sha: parts[0].to_string(),
                message: parts[1].to_string(),
                author: parts[2].to_string(),
                date: parts[3].to_string(),
            })
        })
        .collect();
    Ok(Json(GitLogResponse { commits, has_more }))
}

pub(super) async fn git_graph(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GitGraphParams>,
) -> Result<Json<GitGraphResponse>, ApiError> {
    let cwd = git_workspace(&state);
    let count = params.count.unwrap_or(100);
    let output = run_git(
        &[
            "log",
            "--all",
            &format!("-n{count}"),
            "--pretty=format:%H|%P|%s|%an|%aI|%D",
        ],
        &cwd,
    )
    .unwrap_or_default();
    let commits = output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(6, '|').collect();
            if parts.len() < 6 {
                return None;
            }
            Some(GitGraphCommit {
                sha: parts[0].to_string(),
                parents: if parts[1].is_empty() {
                    vec![]
                } else {
                    parts[1].split(' ').map(|s| s.to_string()).collect()
                },
                message: parts[2].to_string(),
                author: parts[3].to_string(),
                date: parts[4].to_string(),
                refs: if parts[5].is_empty() {
                    vec![]
                } else {
                    parts[5].split(", ").map(|s| s.to_string()).collect()
                },
            })
        })
        .collect();
    Ok(Json(GitGraphResponse { commits }))
}

pub(super) async fn git_show_files(
    State(state): State<Arc<AppState>>,
    Path(sha): Path<String>,
) -> Result<Json<GitShowResponse>, ApiError> {
    let cwd = git_workspace(&state);
    let metadata = run_git(
        &["show", "-s", "--format=%H%x1f%an%x1f%aI%x1f%s", &sha],
        &cwd,
    )
    .map_err(ApiError::not_found)?;
    let fields: Vec<&str> = metadata.trim_end().splitn(4, '\x1f').collect();
    if fields.len() != 4 {
        return Err(ApiError::not_found("Invalid git show response"));
    }

    let output = run_git(
        &["show", "--format=", "--numstat", "--find-renames", &sha],
        &cwd,
    )
    .map_err(ApiError::not_found)?;
    let files: Vec<GitCommitFileInfo> = output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() != 3 {
                return None;
            }
            let additions = parts[0].parse().unwrap_or(0);
            let deletions = parts[1].parse().unwrap_or(0);
            let status = match (additions, deletions) {
                (0, deletions) if deletions > 0 => "D",
                (additions, 0) if additions > 0 => "A",
                _ => "M",
            };
            Some(GitCommitFileInfo {
                path: parts[2].to_string(),
                status: status.to_string(),
                additions,
                deletions,
            })
        })
        .collect();
    let total_additions = files.iter().map(|file| file.additions).sum();
    let total_deletions = files.iter().map(|file| file.deletions).sum();

    Ok(Json(GitShowResponse {
        sha: fields[0].to_string(),
        author: fields[1].to_string(),
        date: fields[2].to_string(),
        message: fields[3].trim_end().to_string(),
        files,
        total_additions,
        total_deletions,
    }))
}

pub(super) async fn git_show_diff(
    State(state): State<Arc<AppState>>,
    Path(sha): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Vec<GitFileDiffResponse>>, ApiError> {
    let cwd = git_workspace(&state);
    if let Some(path) = params.get("path") {
        let output = run_git(&["show", "--pretty=format:", &sha, "--", path], &cwd)
            .map_err(ApiError::not_found)?;
        return Ok(Json(vec![GitFileDiffResponse {
            path: path.clone(),
            diff: output,
        }]));
    }

    let commit = git_show_files(State(state.clone()), Path(sha.clone()))
        .await
        .map_err(|error| ApiError::not_found(error.message))?
        .0;
    let mut diffs = Vec::with_capacity(commit.files.len());
    for file in commit.files {
        let output = run_git(&["show", "--pretty=format:", &sha, "--", &file.path], &cwd)
            .map_err(ApiError::not_found)?;
        diffs.push(GitFileDiffResponse {
            path: file.path,
            diff: output,
        });
    }
    Ok(Json(diffs))
}

pub(super) async fn git_diff_file(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GitDiffFileParams>,
) -> Result<Json<GitFileDiffResponse>, ApiError> {
    let cwd = git_workspace(&state);
    let staged = params.staged.unwrap_or(false);
    let output = if staged {
        run_git(&["diff", "--cached", "--", &params.path], &cwd)
    } else {
        run_git(&["diff", "--", &params.path], &cwd)
    }
    .map_err(|e| ApiError::not_found(e))?;
    Ok(Json(GitFileDiffResponse {
        path: params.path.clone(),
        diff: output,
    }))
}

pub(super) async fn git_commit(
    State(state): State<Arc<AppState>>,
    Json(body): Json<GitCommitRequest>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    if body.message.trim().is_empty() {
        return Err(ApiError::bad_request("Commit message cannot be empty"));
    }
    run_git(&["commit", "-m", &body.message], &cwd).map_err(|e| ApiError::bad_request(e))?;
    Ok(Json(GitMessageResponse {
        success: true,
        message: "Committed".to_string(),
    }))
}

pub(super) async fn git_branch_create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<GitBranchCreateRequest>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    if body.checkout.unwrap_or(true) {
        run_git(&["checkout", "-b", &body.name], &cwd).map_err(|e| ApiError::bad_request(e))?;
    } else {
        run_git(&["branch", &body.name], &cwd).map_err(|e| ApiError::bad_request(e))?;
    }
    Ok(Json(GitMessageResponse {
        success: true,
        message: format!("Branch {} created", body.name),
    }))
}

pub(super) async fn git_branch_delete(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    run_git(&["branch", "-d", &name], &cwd).map_err(|e| ApiError::bad_request(e))?;
    Ok(Json(GitMessageResponse {
        success: true,
        message: format!("Branch {} deleted", name),
    }))
}

pub(super) async fn git_push(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    run_git(&["push"], &cwd).map_err(|e| ApiError::bad_request(e))?;
    Ok(Json(GitMessageResponse {
        success: true,
        message: "Pushed".to_string(),
    }))
}

pub(super) async fn git_pull(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    run_git(&["pull"], &cwd).map_err(|e| ApiError::bad_request(e))?;
    Ok(Json(GitMessageResponse {
        success: true,
        message: "Pulled".to_string(),
    }))
}

#[derive(Deserialize)]
pub(super) struct StashRequest {
    message: Option<String>,
}

pub(super) async fn git_stash(
    State(state): State<Arc<AppState>>,
    body: Option<Json<StashRequest>>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    if let Some(Json(req)) = body {
        if let Some(msg) = req.message {
            run_git(&["stash", "push", "-m", &msg], &cwd).map_err(|e| ApiError::bad_request(e))?;
        } else {
            run_git(&["stash", "push"], &cwd).map_err(|e| ApiError::bad_request(e))?;
        }
    } else {
        run_git(&["stash", "push"], &cwd).map_err(|e| ApiError::bad_request(e))?;
    }
    Ok(Json(GitMessageResponse {
        success: true,
        message: "Stashed".to_string(),
    }))
}

pub(super) async fn git_stash_pop(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GitMessageResponse>, ApiError> {
    let cwd = git_workspace(&state);
    run_git(&["stash", "pop"], &cwd).map_err(|e| ApiError::bad_request(e))?;
    Ok(Json(GitMessageResponse {
        success: true,
        message: "Stash popped".to_string(),
    }))
}
