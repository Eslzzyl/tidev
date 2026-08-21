#![allow(clippy::all)]
//! Terminal API routes for web terminal.
//!
//! - `POST /api/terminal/start`     — Start a new terminal session (with optional cols/rows/shell)
//! - `GET  /api/terminal/shells`    — List available shells on the server
//! - `GET  /api/terminal/ws?session_id=...` — WebSocket endpoint for terminal I/O
//! - `DELETE /api/terminal/{id}`    — Close a terminal session
//!
//! ## WebSocket Protocol
//!
//! The protocol follows restty's native WebSocket PTY transport.
//!
//! **Client → Server:**
//! - `{"type":"input","data":"<text>"}` — Send input to the PTY
//! - `{"type":"resize","cols":<cols>,"rows":<rows>}` — Resize the PTY
//!
//! **Server → Client:**
//! - Raw UTF-8 text or binary frames — PTY output
//! - `{"type":"exit"}` — PTY process exited

use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::{ApiError, AppState};

pub fn terminal_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/terminal/start", post(start_terminal))
        .route("/terminal/rename", post(rename_terminal))
        .route("/terminal/shells", get(list_shells))
        .route("/terminal/list", get(list_sessions))
        .route("/terminal/ws", get(terminal_ws_handler))
        .route("/terminal/{session_id}", delete(close_terminal))
}

// ── Request / Response types ──────────────────────────────────────────────

#[derive(Serialize)]
struct StartResponse {
    session_id: String,
}

#[derive(Serialize)]
struct ShellEntry {
    path: String,
    name: String,
}

#[derive(Serialize)]
struct ShellsResponse {
    shells: Vec<ShellEntry>,
    default_shell: String,
}

#[derive(Deserialize)]
#[serde(default)]
#[derive(Default)]
struct StartRequest {
    cols: Option<u16>,
    rows: Option<u16>,
    shell: Option<String>,
    label: Option<String>,
}

#[derive(Deserialize)]
struct WsQuery {
    /// Optional auth token (for WebSocket which can't set custom headers)
    token: Option<String>,
    /// Existing terminal session to attach to.
    session_id: String,
}

#[derive(Serialize)]
struct SessionListResponse {
    sessions: Vec<SessionEntry>,
}

#[derive(Serialize)]
struct SessionEntry {
    session_id: String,
    label: String,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum PtyClientMessage {
    #[serde(rename = "input")]
    Input { data: String },
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
}

/// Parse a restty PTY message from a WebSocket text or UTF-8 binary frame.
fn parse_pty_message(msg: &Message) -> Result<PtyClientMessage, String> {
    let text = match msg {
        Message::Text(t) => t.to_string(),
        Message::Binary(d) => String::from_utf8(d.to_vec())
            .map_err(|_| "invalid UTF-8 in binary message".to_string())?,
        _ => return Err("unexpected message type".to_string()),
    };

    serde_json::from_str(&text).map_err(|e| format!("invalid PTY message: {e}"))
}

// ── Handlers ──────────────────────────────────────────────────────────────

async fn start_terminal(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StartRequest>,
) -> Result<Json<StartResponse>, ApiError> {
    let cols = req.cols.unwrap_or(80);
    let rows = req.rows.unwrap_or(24);
    let size = portable_pty::PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    };

    // Shell priority:
    //   1. Explicit request parameter (from frontend)
    //   2. Config override (server-side persisted)
    //   3. $SHELL environment variable
    //   4. /bin/bash (hardcoded fallback in start_session)
    let shell = req.shell;

    let label = req.label.unwrap_or_else(|| "Terminal".to_string());

    let session_id = state
        .terminal_manager
        .start_session(state.terminal_tx.clone(), size, shell, label)
        .await
        .map_err(ApiError::internal)?;

    Ok(Json(StartResponse {
        session_id: session_id.to_string(),
    }))
}

/// Detect available shells on the server.
///
/// On Unix, reads `/etc/shells` and filters to existing executables.
/// Always includes `$SHELL` first. Also scans common paths for shells
/// that may not appear in `/etc/shells` (e.g. Homebrew-installed fish).
async fn list_shells(State(_state): State<Arc<AppState>>) -> Json<ShellsResponse> {
    let shells = detect_shells();
    let default_shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    Json(ShellsResponse {
        shells,
        default_shell,
    })
}

/// List running terminal sessions.
async fn list_sessions(State(state): State<Arc<AppState>>) -> Json<SessionListResponse> {
    let entries = state.terminal_manager.list_sessions().await;
    Json(SessionListResponse {
        sessions: entries
            .into_iter()
            .map(|(id, label)| SessionEntry {
                session_id: id.to_string(),
                label,
            })
            .collect(),
    })
}

#[derive(Deserialize)]
struct RenameRequest {
    session_id: String,
    label: String,
}

/// Rename a terminal session (persists label in memory).
async fn rename_terminal(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RenameRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session_id: Uuid = req
        .session_id
        .parse()
        .map_err(|_| ApiError::not_found("Invalid session ID".to_string()))?;
    state
        .terminal_manager
        .rename_session(session_id, req.label)
        .await
        .map_err(ApiError::not_found)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[cfg(unix)]
fn detect_shells() -> Vec<ShellEntry> {
    let mut shells = Vec::new();
    let mut seen = HashSet::new();

    // 1. Always include $SHELL first
    if let Ok(s) = std::env::var("SHELL")
        && !s.is_empty()
        && std::path::Path::new(&s).exists()
    {
        let name = std::path::Path::new(&s)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| s.clone());
        shells.push(ShellEntry {
            path: s.clone(),
            name,
        });
        seen.insert(s);
    }

    // 2. Read /etc/shells
    if let Ok(content) = std::fs::read_to_string("/etc/shells") {
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if std::path::Path::new(line).exists() && !seen.contains(line) {
                let name = std::path::Path::new(line)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| line.to_string());
                shells.push(ShellEntry {
                    path: line.to_string(),
                    name,
                });
                seen.insert(line.to_string());
            }
        }
    }

    // 3. Cross-product search: search_dirs × shell_names.
    //    This catches shells from package managers (Homebrew, cargo, pipx, etc.)
    //    that may not be registered in /etc/shells.
    let home = std::env::var("HOME").unwrap_or_default();
    let search_dirs: Vec<String> = [
        "/bin",
        "/usr/bin",
        "/usr/local/bin",
        "/opt/homebrew/bin",
        "/opt/local/bin",
        "/run/current-system/sw/bin",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .chain([
        format!("{}/.cargo/bin", home),
        format!("{}/.local/bin", home),
        format!("{}/.nix-profile/bin", home),
    ])
    .collect();

    let shell_names = [
        "bash", "zsh", "fish", "nu", "sh", "dash", "tcsh", "ksh", "mksh", "oksh", "elvish", "pwsh",
    ];

    for dir in &search_dirs {
        for name in &shell_names {
            let path = format!("{}/{}", dir, name);
            if std::path::Path::new(&path).exists() && !seen.contains(&path) {
                shells.push(ShellEntry {
                    path: path.clone(),
                    name: name.to_string(),
                });
                seen.insert(path);
            }
        }
    }

    shells
}

#[cfg(windows)]
fn detect_shells() -> Vec<ShellEntry> {
    let mut shells = Vec::new();
    let mut seen = HashSet::new();

    // 1. Always include ComSpec (the standard Windows default shell env var) first
    if let Ok(s) = std::env::var("ComSpec")
        && !s.is_empty()
        && std::path::Path::new(&s).exists()
        && !seen.contains(&s)
    {
        let name = std::path::Path::new(&s)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "cmd.exe".to_string());
        shells.push(ShellEntry {
            path: s.clone(),
            name,
        });
        seen.insert(s);
    }

    // 2. Include $SHELL if set (may be used by MSYS2/Cygwin environments)
    if let Ok(s) = std::env::var("SHELL")
        && !s.is_empty()
        && std::path::Path::new(&s).exists()
        && !seen.contains(&s)
    {
        let name = std::path::Path::new(&s)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| s.clone());
        shells.push(ShellEntry {
            path: s.clone(),
            name,
        });
        seen.insert(s);
    }

    // 3. System-known shells from PATH (where we can resolve them)
    let path_dirs = std::env::var("PATH").unwrap_or_default();
    let shell_exes = [
        "powershell.exe",
        "pwsh.exe",
        "cmd.exe",
        "wsl.exe",
        "bash.exe",
        "nu.exe",
        "fish.exe",
        "zsh.exe",
        "ksh.exe",
        "tcsh.exe",
        "elvish.exe",
        "dash.exe",
    ];
    for dir in path_dirs.split(';') {
        let dir = dir.trim();
        if dir.is_empty() {
            continue;
        }
        for name in &shell_exes {
            let path = format!("{}\\{}", dir, name);
            if std::path::Path::new(&path).exists() && !seen.contains(&path) {
                let display_name = name.trim_end_matches(".exe").to_string();
                shells.push(ShellEntry {
                    path: path.clone(),
                    name: display_name,
                });
                seen.insert(path);
            }
        }
    }

    // 4. Cross-product search: common install directories × shell names
    let userprofile = std::env::var("USERPROFILE").unwrap_or_default();
    let localappdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let programfiles =
        std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".to_string());
    let programfiles_x86 = std::env::var("ProgramFiles(x86)")
        .unwrap_or_else(|_| "C:\\Program Files (x86)".to_string());
    let systemroot = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());

    let search_dirs: Vec<String> = [
        format!("{}\\System32", systemroot),
        format!("{}\\SysWOW64", systemroot),
        format!("{}\\Git\\bin", programfiles),
        format!("{}\\Git\\usr\\bin", programfiles),
        format!("{}\\Git\\bin", programfiles_x86),
        format!("{}\\Git\\usr\\bin", programfiles_x86),
        "C:\\msys64\\usr\\bin".to_string(),
        "C:\\tools\\msys64\\usr\\bin".to_string(),
        "C:\\cygwin64\\bin".to_string(),
        "C:\\cygwin\\bin".to_string(),
        format!("{}\\Chocolatey\\bin", programfiles),
        format!("{}\\chocolatey\\bin", programfiles),
        format!("{}\\scoop\\shims", userprofile),
        format!("{}\\scoop\\apps\\pwsh\\current", userprofile),
        format!("{}\\Microsoft\\WindowsApps", localappdata),
        format!("{}\\.cargo\\bin", userprofile),
    ]
    .into_iter()
    .collect();

    let shell_names = [
        "powershell.exe",
        "pwsh.exe",
        "cmd.exe",
        "bash.exe",
        "nu.exe",
        "fish.exe",
        "zsh.exe",
        "ksh.exe",
        "tcsh.exe",
        "elvish.exe",
        "dash.exe",
        "sh.exe",
        "wsl.exe",
    ];

    for dir in &search_dirs {
        for name in &shell_names {
            let path = format!("{}\\{}", dir, name);
            if std::path::Path::new(&path).exists() && !seen.contains(&path) {
                let display_name = name.trim_end_matches(".exe").to_string();
                shells.push(ShellEntry {
                    path: path.clone(),
                    name: display_name,
                });
                seen.insert(path);
            }
        }
    }

    shells
}

/// WebSocket endpoint for terminal I/O.
///
/// Public endpoint (bypasses auth middleware) because the browser WebSocket API
/// cannot set custom headers. Auth is handled inline via query param.
///
/// The session is selected by the `session_id` query parameter and the payload
/// format follows restty's native WebSocket PTY transport.
async fn terminal_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Query(query): Query<WsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    // Validate auth token if configured
    if let Some(configured) = crate::api::configured_auth_token(&state) {
        let provided = query.token.as_deref().unwrap_or("");
        if provided != configured {
            return Err(ApiError::unauthorized("Invalid or missing auth token"));
        }
    }

    let session_id = Uuid::parse_str(&query.session_id)
        .map_err(|e| ApiError::bad_request(format!("Invalid session_id: {e}")))?;
    if !state.terminal_manager.has_session(session_id).await {
        return Err(ApiError::not_found("Terminal session not found"));
    }

    Ok(ws.on_upgrade(move |socket| handle_terminal_ws(socket, state, session_id)))
}

async fn handle_terminal_ws(mut ws: WebSocket, state: Arc<AppState>, session_id: Uuid) {
    let cancel_token = state.cancel.clone();
    let terminal_manager = state.terminal_manager.clone();
    let terminal_tx = state.terminal_tx.clone();

    // Flush any buffered output that was produced before this subscriber
    // connected (e.g. the initial shell prompt).
    let buf = terminal_manager.get_buffer(session_id).await;
    if !buf.is_empty() {
        let _ = ws.send(Message::Binary(buf.into())).await;
    }

    // Subscribe to terminal output
    let mut rx = terminal_tx.subscribe();

    // Channel for sending messages to the WebSocket from the spawned task.
    let (output_tx, mut output_rx) = tokio::sync::mpsc::channel::<Message>(256);

    // Spawn a task to forward terminal output to the mpsc channel.
    let cancel_clone = cancel_token.clone();
    let output_tx_clone = output_tx.clone();
    let sid = session_id;
    let output_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel_clone.cancelled() => break,
                result = rx.recv() => {
                    let output = match result {
                        Ok(o) => o,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            log::warn!("terminal WS output lagged by {n} for session {sid}");
                            continue;
                        }
                    };

                    if output.session_id != sid {
                        continue;
                    }

                    if output.closed {
                        let _ = output_tx_clone
                            .send(Message::Text(
                                serde_json::json!({"type": "exit"}).to_string().into(),
                            ))
                            .await;
                        break;
                    }

                    let _ = output_tx_clone
                        .send(Message::Binary(output.data.into()))
                        .await;
                }
            }
        }
    });

    // Main I/O loop
    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(30));

    loop {
        tokio::select! {
            ws_msg = ws.recv() => {
                match ws_msg {
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(msg)) => {
                        match parse_pty_message(&msg) {
                            Ok(PtyClientMessage::Input { data }) => {
                                let _ = terminal_manager
                                    .write_input(session_id, data.as_bytes())
                                    .await;
                            }
                            Ok(PtyClientMessage::Resize { cols, rows }) => {
                                let _ = terminal_manager.resize(session_id, cols, rows).await;
                            }
                            Err(_) => {
                                // Ignore malformed or unsupported PTY messages.
                            }
                        }
                    }
                    Some(Err(_)) | None => break,
                }
            }
            out_msg = output_rx.recv() => {
                match out_msg {
                    Some(msg) => {
                        if ws.send(msg).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ = ping_interval.tick() => {
                if ws.send(Message::Ping(bytes::Bytes::new())).await.is_err() {
                    break;
                }
            }
        }
    }

    output_task.abort();
}

async fn close_terminal(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Ok(id) = Uuid::parse_str(&session_id) {
        state.terminal_manager.close_session(id).await;
    }
    axum::http::StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_input_message() {
        let message = Message::Text(r#"{"type":"input","data":"pwd\r"}"#.into());

        assert!(matches!(
            parse_pty_message(&message),
            Ok(PtyClientMessage::Input { data }) if data == "pwd\r"
        ));
    }

    #[test]
    fn parses_resize_message() {
        let message = Message::Text(r#"{"type":"resize","cols":120,"rows":40}"#.into());

        assert!(matches!(
            parse_pty_message(&message),
            Ok(PtyClientMessage::Resize {
                cols: 120,
                rows: 40
            })
        ));
    }

    #[test]
    fn rejects_legacy_array_message() {
        let message = Message::Text(r#"["stdin","pwd"]"#.into());

        assert!(parse_pty_message(&message).is_err());
    }
}
