//! Git API routes for web terminal.
//!
//! - `GET    /api/git/status`          — Working tree status
//! - `GET    /api/git/branches`        — List branches (with optional submodule filtering)
//! - `GET    /api/git/history`         — Commit log (with skip/count pagination)
//! - `GET    /api/git/show/{sha}`      — List files changed in a commit
//! - `GET    /api/git/show/{sha}/diff` — Get all diffs for a commit (or per-file with ?path=)
//! - `POST   /api/git/commit`          — Create a commit
//! - `POST   /api/git/branch`          — Create or switch branch
//! - `DELETE /api/git/branch/{name}`   — Delete a branch
//! - `POST   /api/git/push`            — Push to remote
//! - `POST   /api/git/pull`            — Pull from remote
//! - `POST   /api/git/stash`           — Stash changes
//! - `POST   /api/git/stash/pop`       — Pop stash

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::super::state::AppState;

pub fn git_routes() -> Router<AppState> {
    Router::new()
        .route("/git/status", get(git_status))
        .route("/git/branches", get(git_branches))
        .route("/git/history", get(git_log))
        .route("/git/show/{sha}", get(git_show_files))
        .route("/git/show/{sha}/diff", get(git_show_diff))
        .route("/git/diff/file", get(git_diff_file))
        .route("/git/commit", post(git_commit))
        .route("/git/branch", post(git_branch_create))
        .route("/git/branch/{name}", delete(git_branch_delete))
        .route("/git/push", post(git_push))
        .route("/git/pull", post(git_pull))
        .route("/git/stash", post(git_stash))
        .route("/git/stash/pop", post(git_stash_pop))
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn workspace(state: &AppState) -> &PathBuf {
    &state.workspace_root
}

fn run_git(args: &[&str], cwd: &PathBuf) -> Result<String, String> {
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

// ── Types ─────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct StatusResponse {
    branch: String,
    sha: String,
    files: Vec<StatusFile>,
    ahead: i32,
    behind: i32,
}

#[derive(Serialize)]
struct StatusFile {
    path: String,
    status: String, // M, A, D, R, C, U, ?, !
    staged: bool,
}

#[derive(Serialize)]
struct GitBranchResponse {
    current: String,
    branches: Vec<BranchItem>,
}

#[derive(Serialize)]
struct BranchItem {
    name: String,
    current: bool,
    remote: Option<String>,
}

#[derive(Serialize)]
struct GitLogResponse {
    commits: Vec<CommitItem>,
    has_more: bool,
}

#[derive(Serialize)]
struct CommitItem {
    sha: String,
    author: String,
    date: String,
    message: String,
}

/// File info for a commit (returned by git_show_files).
#[derive(Serialize)]
struct CommitFileInfo {
    path: String,
    status: String, // A, M, D
    additions: i32,
    deletions: i32,
}

#[derive(Serialize)]
struct GitShowResponse {
    sha: String,
    author: String,
    date: String,
    message: String,
    files: Vec<CommitFileInfo>,
    total_additions: i32,
    total_deletions: i32,
}

#[derive(Serialize)]
struct GitFileDiffResponse {
    path: String,
    diff: String,
}

#[derive(Deserialize)]
struct GitLogParams {
    count: Option<usize>,
    skip: Option<usize>,
}

#[derive(Deserialize)]
struct GitBranchesParams {
    show_submodules: Option<bool>,
}

#[derive(Deserialize)]
struct GitShowDiffParams {
    path: Option<String>,
}

#[derive(Deserialize)]
struct GitDiffFileParams {
    path: String,
    staged: Option<bool>,
}

#[derive(Deserialize)]
struct CommitRequest {
    message: String,
}

#[derive(Deserialize)]
struct BranchCreateRequest {
    name: String,
    checkout: bool,
}

#[derive(Serialize)]
struct MessageResponse {
    success: bool,
    message: String,
}

#[derive(Deserialize)]
struct StashRequest {
    message: Option<String>,
}

#[derive(Deserialize)]
struct GitPushPullRequest {
    remote: Option<String>,
    branch: Option<String>,
    force: Option<bool>,
}

// ── Handlers ──────────────────────────────────────────────────────────────

/// `GET /api/git/status`
async fn git_status(
    State(state): State<AppState>,
) -> Result<Json<StatusResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();

    // Get branch name
    let branch = run_git(&["rev-parse", "--abbrev-ref", "HEAD"], &cwd)
        .map_err(crate::web::error::AppError::Internal)?;
    let branch = branch.trim().to_string();

    // Get SHA
    let sha = run_git(&["rev-parse", "--short", "HEAD"], &cwd)
        .map_err(crate::web::error::AppError::Internal)?;
    let sha = sha.trim().to_string();

    // Get ahead/behind counts
    let ahead_behind = run_git(
        &["rev-list", "--count", "--left-right", "@{upstream}...HEAD"],
        &cwd,
    );
    let (ahead, behind) = match &ahead_behind {
        Ok(s) => {
            let parts: Vec<&str> = s.trim().split('\t').collect();
            (
                parts.first().and_then(|p| p.parse().ok()).unwrap_or(0),
                parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0),
            )
        }
        Err(_) => (0, 0),
    };

    // Get status
    let status_output = run_git(&["status", "--porcelain", "-u"], &cwd)
        .map_err(crate::web::error::AppError::Internal)?;

    let files: Vec<StatusFile> = status_output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let first = line.as_bytes().first().copied().unwrap_or(b' ');
            // The first column is the staging area status:
            //   ' ' = not staged, '?' = untracked, '!' = ignored
            //   M/A/D/R/C = staged with that status
            let staged = first != b' ' && first != b'?' && first != b'!';
            let status_char = if staged {
                first as char
            } else {
                line.as_bytes().get(1).copied().unwrap_or(b'?') as char
            };
            let path = if line.len() > 3 { &line[3..] } else { line };
            StatusFile {
                path: path.to_string(),
                status: status_char.to_string(),
                staged,
            }
        })
        .collect();

    Ok(Json(StatusResponse {
        branch,
        sha,
        files,
        ahead,
        behind,
    }))
}

/// `GET /api/git/branches?show_submodules=false`
async fn git_branches(
    State(state): State<AppState>,
    Query(params): Query<GitBranchesParams>,
) -> Result<Json<GitBranchResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();

    let current = run_git(&["rev-parse", "--abbrev-ref", "HEAD"], &cwd)
        .map_err(crate::web::error::AppError::Internal)?;
    let current = current.trim().to_string();

    let output = run_git(
        &[
            "branch",
            "--all",
            "--format=%(refname:short)|%(upstream:short)",
        ],
        &cwd,
    )
    .map_err(crate::web::error::AppError::Internal)?;

    // Collect submodule paths for filtering
    let submodule_paths = if !params.show_submodules.unwrap_or(false) {
        get_submodule_paths(&cwd)
    } else {
        Vec::new()
    };

    let branches: Vec<BranchItem> = output
        .lines()
        .filter(|l| !l.is_empty())
        .filter(|line| {
            if submodule_paths.is_empty() {
                return true;
            }
            let name = line.split('|').next().unwrap_or(line);
            let name = name.trim_start_matches("* ").trim();
            // Filter out branches that belong to submodules
            // Submodule branches look like: "origin/submodule-path/branch-name"
            // or "submodule-path/branch-name" for local
            !submodule_paths.iter().any(|sub_path| {
                name.contains(&format!("/{}/", sub_path))
                    || name == sub_path.as_str()
                    || name.starts_with(&format!("{}/", sub_path))
            })
        })
        .map(|line| {
            let parts: Vec<&str> = line.split('|').collect();
            let name = parts[0].to_string();
            let remote = parts
                .get(1)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let is_current = name == current || name.strip_prefix("* ").unwrap_or(&name) == current;
            BranchItem {
                name: name.trim_start_matches("* ").to_string(),
                current: is_current || name.starts_with('*'),
                remote,
            }
        })
        .collect();

    Ok(Json(GitBranchResponse { current, branches }))
}

/// Read `.gitmodules` and return a list of submodule paths.
fn get_submodule_paths(cwd: &PathBuf) -> Vec<String> {
    let gitmodules_path = cwd.join(".gitmodules");
    if !gitmodules_path.exists() {
        return Vec::new();
    }

    // Use git config to reliably parse .gitmodules
    let output = std::process::Command::new("git")
        .args([
            "config",
            "--file",
            ".gitmodules",
            "--get-regexp",
            r"submodule\..*\.path",
        ])
        .current_dir(cwd)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout
                .lines()
                .filter_map(|line| {
                    // Format: "submodule.name.path path/to/submodule"
                    line.split_once(' ').map(|(_, p)| p.trim().to_string())
                })
                .collect()
        }
        _ => Vec::new(),
    }
}
async fn git_log(
    State(state): State<AppState>,
    Query(params): Query<GitLogParams>,
) -> Result<Json<GitLogResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();
    let count = params.count.unwrap_or(20);
    let skip = params.skip.unwrap_or(0);

    // Request count+1 so we can detect if there are more commits
    let fetch = count + 1;
    let output = run_git(
        &[
            "log",
            &format!("--skip={}", skip),
            &format!("-{}", fetch),
            "--format=%H|%an|%ai|%s",
        ],
        &cwd,
    )
    .map_err(crate::web::error::AppError::Internal)?;

    let mut commits: Vec<CommitItem> = output
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() >= 4 {
                Some(CommitItem {
                    sha: parts[0].to_string(),
                    author: parts[1].to_string(),
                    date: parts[2].to_string(),
                    message: parts[3].to_string(),
                })
            } else {
                None
            }
        })
        .collect();

    let has_more = commits.len() > count;
    if has_more {
        commits.truncate(count);
    }

    Ok(Json(GitLogResponse { commits, has_more }))
}

/// `GET /api/git/show/{sha}` — List files changed in a commit (no diff content).
async fn git_show_files(
    State(state): State<AppState>,
    Path(sha): Path<String>,
) -> Result<Json<GitShowResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();

    // Get commit metadata
    let info = run_git(
        &["log", "-1", "--format=%H|%an|%ai|%s", &sha],
        &cwd,
    )
    .map_err(crate::web::error::AppError::Internal)?;

    let (author, date, message) = info
        .lines()
        .next()
        .and_then(|line| {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() >= 4 {
                Some((
                    parts[1].to_string(),
                    parts[2].to_string(),
                    parts[3].to_string(),
                ))
            } else {
                None
            }
        })
        .unwrap_or_default();

    // Get file list with name-status
    let name_status = run_git(
        &["diff-tree", "--no-commit-id", "-r", "--name-status", "--root", &sha],
        &cwd,
    )
    .map_err(crate::web::error::AppError::Internal)?;

    // Get file list with numstat (additions/deletions)
    let numstat = run_git(
        &["diff-tree", "--no-commit-id", "-r", "--numstat", "--root", &sha],
        &cwd,
    )
    .map_err(crate::web::error::AppError::Internal)?;

    // Build status map from --name-status output
    let mut status_map: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for line in name_status.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() == 2 {
            status_map.insert(parts[1], parts[0]);
        }
    }

    // Build files list from --numstat output, merging status
    let mut files: Vec<CommitFileInfo> = Vec::new();
    let mut total_additions = 0i32;
    let mut total_deletions = 0i32;

    for line in numstat.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let path = parts[2];
        let additions: i32 = parts[0].parse().unwrap_or(0);
        let deletions: i32 = parts[1].parse().unwrap_or(0);
        let status = status_map.get(path).copied().unwrap_or("M").to_string();

        total_additions += additions;
        total_deletions += deletions;

        files.push(CommitFileInfo {
            path: path.to_string(),
            status,
            additions,
            deletions,
        });
    }

    Ok(Json(GitShowResponse {
        sha: sha.to_string(),
        author,
        date,
        message,
        files,
        total_additions,
        total_deletions,
    }))
}

/// `GET /api/git/show/{sha}/diff?path=file/path` — Get diffs for a commit.
/// If `path` is provided, returns diff for that specific file only.
/// Otherwise, returns all diffs as a per-file array.
async fn git_show_diff(
    State(state): State<AppState>,
    Path(sha): Path<String>,
    Query(params): Query<GitShowDiffParams>,
) -> Result<Json<Vec<GitFileDiffResponse>>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();

    if let Some(file_path) = &params.path {
        // Single file diff
        let output = run_git(
            &[
                "diff-tree",
                "--no-commit-id",
                "-r",
                "-p",
                "--root",
                &sha,
                "--",
                file_path,
            ],
            &cwd,
        )
        .map_err(crate::web::error::AppError::Internal)?;

        Ok(Json(vec![GitFileDiffResponse {
            path: file_path.clone(),
            diff: output,
        }]))
    } else {
        // All files diff
        let output = run_git(
            &["diff-tree", "--no-commit-id", "-r", "-p", "--root", &sha],
            &cwd,
        )
        .map_err(crate::web::error::AppError::Internal)?;

        // Parse diff into per-file chunks
        let per_file = split_diff_by_file(&output);
        Ok(Json(per_file))
    }
}

/// Split a unified diff string into per-file chunks.
fn split_diff_by_file(diff: &str) -> Vec<GitFileDiffResponse> {
    let mut result: Vec<GitFileDiffResponse> = Vec::new();
    let mut current_path = String::new();
    let mut current_lines: Vec<&str> = Vec::new();

    for line in diff.lines() {
        if line.starts_with("diff --git a/") {
            // Save previous file
            if !current_path.is_empty() && !current_lines.is_empty() {
                result.push(GitFileDiffResponse {
                    path: std::mem::take(&mut current_path),
                    diff: current_lines.join("\n"),
                });
                current_lines.clear();
            }
            // Extract new path from "diff --git a/path b/path"
            if let Some(b_part) = line.split(" b/").nth(1) {
                current_path = b_part.to_string();
            }
            current_lines.push(line);
        } else if line.starts_with("diff --cc ") {
            // Merge commit format
            if !current_path.is_empty() && !current_lines.is_empty() {
                result.push(GitFileDiffResponse {
                    path: std::mem::take(&mut current_path),
                    diff: current_lines.join("\n"),
                });
                current_lines.clear();
            }
            if let Some(b_part) = line.split(" b/").nth(1) {
                current_path = b_part.to_string();
            }
            current_lines.push(line);
        } else {
            current_lines.push(line);
        }
    }

    // Last file
    if !current_path.is_empty() && !current_lines.is_empty() {
        result.push(GitFileDiffResponse {
            path: current_path,
            diff: current_lines.join("\n"),
        });
    }

    result
}

/// `GET /api/git/diff/file?path=xxx&staged=false` — Get working tree diff for a file.
async fn git_diff_file(
    State(state): State<AppState>,
    Query(params): Query<GitDiffFileParams>,
) -> Result<Json<GitFileDiffResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();
    let staged = params.staged.unwrap_or(false);

    let mut args = vec!["diff", "--no-color"];
    if staged {
        args.push("--cached");
    }
    args.push("--");
    args.push(&params.path);

    let output = run_git(&args, &cwd).map_err(crate::web::error::AppError::Internal)?;

    Ok(Json(GitFileDiffResponse {
        path: params.path,
        diff: output,
    }))
}

/// `POST /api/git/commit`
async fn git_commit(
    State(state): State<AppState>,
    Json(req): Json<CommitRequest>,
) -> Result<Json<MessageResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();

    run_git(&["add", "-A"], &cwd).map_err(crate::web::error::AppError::Internal)?;

    run_git(&["commit", "-m", &req.message], &cwd)
        .map_err(crate::web::error::AppError::Internal)?;

    Ok(Json(MessageResponse {
        success: true,
        message: "Committed successfully".to_string(),
    }))
}

/// `POST /api/git/branch`
async fn git_branch_create(
    State(state): State<AppState>,
    Json(req): Json<BranchCreateRequest>,
) -> Result<Json<MessageResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();

    run_git(&["branch", &req.name], &cwd).map_err(crate::web::error::AppError::Internal)?;

    if req.checkout {
        run_git(&["checkout", &req.name], &cwd).map_err(crate::web::error::AppError::Internal)?;
    }

    Ok(Json(MessageResponse {
        success: true,
        message: format!("Branch '{}' created", req.name),
    }))
}

/// `DELETE /api/git/branch/{name}`
async fn git_branch_delete(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<MessageResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();

    run_git(&["branch", "-D", &name], &cwd).map_err(crate::web::error::AppError::Internal)?;

    Ok(Json(MessageResponse {
        success: true,
        message: format!("Branch '{}' deleted", name),
    }))
}

/// `POST /api/git/push`
async fn git_push(
    State(state): State<AppState>,
    Json(req): Json<GitPushPullRequest>,
) -> Result<Json<MessageResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();
    let remote = req.remote.as_deref().unwrap_or("origin");
    let branch = req.branch.as_deref().unwrap_or("HEAD");

    let mut args = vec!["push"];
    if req.force.unwrap_or(false) {
        args.push("--force");
    }
    args.push(remote);
    args.push(branch);

    run_git(&args, &cwd).map_err(crate::web::error::AppError::Internal)?;

    Ok(Json(MessageResponse {
        success: true,
        message: format!("Pushed to {}/{}", remote, branch),
    }))
}

/// `POST /api/git/pull`
async fn git_pull(
    State(state): State<AppState>,
    Json(req): Json<GitPushPullRequest>,
) -> Result<Json<MessageResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();
    let remote = req.remote.as_deref().unwrap_or("origin");
    let branch = req.branch.as_deref().unwrap_or("HEAD");

    run_git(&["pull", remote, branch], &cwd).map_err(crate::web::error::AppError::Internal)?;

    Ok(Json(MessageResponse {
        success: true,
        message: format!("Pulled from {}/{}", remote, branch),
    }))
}

/// `POST /api/git/stash`
async fn git_stash(
    State(state): State<AppState>,
    Json(req): Json<StashRequest>,
) -> Result<Json<MessageResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();

    if let Some(msg) = &req.message {
        run_git(&["stash", "push", "-m", msg], &cwd)
            .map_err(crate::web::error::AppError::Internal)?;
    } else {
        run_git(&["stash", "push"], &cwd).map_err(crate::web::error::AppError::Internal)?;
    }

    Ok(Json(MessageResponse {
        success: true,
        message: "Changes stashed".to_string(),
    }))
}

/// `POST /api/git/stash/pop`
async fn git_stash_pop(
    State(state): State<AppState>,
) -> Result<Json<MessageResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();

    run_git(&["stash", "pop"], &cwd).map_err(crate::web::error::AppError::Internal)?;

    Ok(Json(MessageResponse {
        success: true,
        message: "Stash popped".to_string(),
    }))
}
