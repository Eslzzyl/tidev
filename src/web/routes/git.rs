//! Git API routes for web terminal.
//!
//! - `GET    /api/git/status`          — Working tree status
//! - `GET    /api/git/branches`        — List branches
//! - `GET    /api/git/history`         — Commit log
//! - `POST   /api/git/commit`          — Create a commit
//! - `POST   /api/git/branch`          — Create or switch branch
//! - `DELETE /api/git/branch/{name}`   — Delete a branch
//! - `POST   /api/git/push`            — Push to remote
//! - `POST   /api/git/pull`            — Pull from remote
//! - `POST   /api/git/stash`           — Stash changes
//! - `POST   /api/git/stash/pop`       — Pop stash

use axum::{
    Router,
    extract::{Path, Query, State},
    routing::{delete, get, post},
    Json,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::super::state::AppState;

pub fn git_routes() -> Router<AppState> {
    Router::new()
        .route("/git/status", get(git_status))
        .route("/git/branches", get(git_branches))
        .route("/git/history", get(git_log))
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
        String::from_utf8(output.stdout)
            .map_err(|e| format!("Invalid UTF-8: {e}"))
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
}

#[derive(Serialize)]
struct CommitItem {
    sha: String,
    author: String,
    date: String,
    message: String,
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
    let status_output = run_git(
        &["status", "--porcelain", "-u"],
        &cwd,
    )
    .map_err(crate::web::error::AppError::Internal)?;

    let files: Vec<StatusFile> = status_output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let staged = line.as_bytes().first().copied().unwrap_or(b' ') != b' ';
            let status_char = if staged {
                line.as_bytes().first().copied().unwrap_or(b'?') as char
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

/// `GET /api/git/branches?local=true`
async fn git_branches(
    State(state): State<AppState>,
) -> Result<Json<GitBranchResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();

    let current = run_git(&["rev-parse", "--abbrev-ref", "HEAD"], &cwd)
        .map_err(crate::web::error::AppError::Internal)?;
    let current = current.trim().to_string();

    let output = run_git(&["branch", "--all", "--format=%(refname:short)|%(upstream:short)"], &cwd)
        .map_err(crate::web::error::AppError::Internal)?;

    let branches: Vec<BranchItem> = output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split('|').collect();
            let name = parts[0].to_string();
            let remote = parts.get(1).filter(|s| !s.is_empty()).map(|s| s.to_string());
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

/// `GET /api/git/history?count=20`
async fn git_log(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<GitLogResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();
    let count = params.get("count").and_then(|c| c.parse().ok()).unwrap_or(20);

    let output = run_git(
        &[
            "log",
            &format!("-{}", count),
            "--format=%H|%an|%ai|%s",
        ],
        &cwd,
    )
    .map_err(crate::web::error::AppError::Internal)?;

    let commits: Vec<CommitItem> = output
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

    Ok(Json(GitLogResponse { commits }))
}

/// `POST /api/git/commit`
async fn git_commit(
    State(state): State<AppState>,
    Json(req): Json<CommitRequest>,
) -> Result<Json<MessageResponse>, crate::web::error::AppError> {
    let cwd = workspace(&state).clone();

    run_git(&["add", "-A"], &cwd)
        .map_err(crate::web::error::AppError::Internal)?;

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

    run_git(&["branch", &req.name], &cwd)
        .map_err(crate::web::error::AppError::Internal)?;

    if req.checkout {
        run_git(&["checkout", &req.name], &cwd)
            .map_err(crate::web::error::AppError::Internal)?;
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

    run_git(&["branch", "-D", &name], &cwd)
        .map_err(crate::web::error::AppError::Internal)?;

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

    run_git(&args, &cwd)
        .map_err(crate::web::error::AppError::Internal)?;

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

    run_git(&["pull", remote, branch], &cwd)
        .map_err(crate::web::error::AppError::Internal)?;

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
        run_git(&["stash", "push"], &cwd)
            .map_err(crate::web::error::AppError::Internal)?;
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

    run_git(&["stash", "pop"], &cwd)
        .map_err(crate::web::error::AppError::Internal)?;

    Ok(Json(MessageResponse {
        success: true,
        message: "Stash popped".to_string(),
    }))
}
