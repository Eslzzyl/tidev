//! Terminal API routes for web terminal.
//!
//! - `POST /api/terminal/start`     — Start a new terminal session (with optional cols/rows/shell)
//! - `POST /api/terminal/input`     — Send raw input to a terminal session
//! - `POST /api/terminal/resize`    — Resize the PTY (cols × rows)
//! - `GET  /api/terminal/events`    — SSE stream for terminal output
//! - `GET  /api/terminal/shells`    — List available shells on the server
//! - `GET  /api/terminal/ws`        — WebSocket endpoint for terminal I/O
//! - `DELETE /api/terminal/{id}`    — Close a terminal session
//!
//! ## WebSocket Protocol
//!
//! Messages are JSON arrays: `[type, ...args]`
//!
//! **Client → Server:**
//! - `["bind", "<session_id>"]`  — Bind to an existing session
//! - `["stdin", "<text>"]`       — Send input to the PTY
//! - `["resize", <rows>, <cols>]` — Resize the PTY
//!
//! **Server → Client:**
//! - `["setup"]`                 — Session ready (sent after successful bind)
//! - `["stdout", "<text>"]`      — PTY output
//! - `["disconnect", "<reason>"]` — Session closed

use std::collections::HashSet;
use std::convert::Infallible;

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    },
    response::{
        IntoResponse,
        sse::{Event, Sse},
    },
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::super::state::AppState;

pub fn terminal_routes() -> Router<AppState> {
    Router::new()
        .route("/terminal/start", post(start_terminal))
        .route("/terminal/input", post(terminal_input))
        .route("/terminal/resize", post(terminal_resize))
        .route("/terminal/rename", post(rename_terminal))
        .route("/terminal/events", get(terminal_events))
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
struct InputRequest {
    session_id: String,
    data: String,
}

#[derive(Deserialize)]
struct ResizeRequest {
    session_id: String,
    cols: u16,
    rows: u16,
}

#[derive(Deserialize)]
struct EventsQuery {
    session_id: String,
    /// Optional auth token (for SSE which can't set custom headers)
    token: Option<String>,
}

#[derive(Deserialize)]
struct WsQuery {
    /// Optional auth token (for WebSocket which can't set custom headers)
    token: Option<String>,
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

/// Parse a JSON array message from the WebSocket.
/// Returns the message type as a string and the remaining arguments.
fn parse_ws_msg(msg: &Message) -> Result<(String, Vec<serde_json::Value>), String> {
    let text = match msg {
        Message::Text(t) => t.to_string(),
        Message::Binary(d) => String::from_utf8(d.to_vec())
            .map_err(|_| "invalid UTF-8 in binary message".to_string())?,
        _ => return Err("unexpected message type".to_string()),
    };

    let arr: Vec<serde_json::Value> =
        serde_json::from_str(&text).map_err(|e| format!("invalid JSON array: {e}"))?;

    if arr.is_empty() {
        return Err("empty message array".to_string());
    }

    let msg_type = arr[0]
        .as_str()
        .ok_or_else(|| "first element must be a string type".to_string())?;

    Ok((msg_type.to_string(), arr[1..].to_vec()))
}

/// Build a JSON array WebSocket message.
fn json_msg(args: impl IntoIterator<Item = impl Into<serde_json::Value>>) -> Utf8Bytes {
    let arr: Vec<serde_json::Value> = args.into_iter().map(Into::into).collect();
    serde_json::to_string(&arr).unwrap_or_default().into()
}

// ── Handlers ──────────────────────────────────────────────────────────────

async fn start_terminal(
    State(state): State<AppState>,
    Json(req): Json<StartRequest>,
) -> Result<Json<StartResponse>, crate::error::AppError> {
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
    let shell = req.shell.or_else(|| {
        let config = state.config.try_read().ok()?;
        config.shell.terminal_shell.clone()
    });

    let label = req.label.unwrap_or_else(|| "Terminal".to_string());

    let session_id = state
        .terminal_manager
        .start_session(state.terminal_tx.clone(), size, shell, label)
        .await
        .map_err(crate::error::AppError::Internal)?;

    Ok(Json(StartResponse {
        session_id: session_id.to_string(),
    }))
}

/// Detect available shells on the server.
///
/// On Unix, reads `/etc/shells` and filters to existing executables.
/// Always includes `$SHELL` first. Also scans common paths for shells
/// that may not appear in `/etc/shells` (e.g. Homebrew-installed fish).
async fn list_shells(State(_state): State<AppState>) -> Json<ShellsResponse> {
    let shells = detect_shells();
    let default_shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    Json(ShellsResponse {
        shells,
        default_shell,
    })
}

/// List running terminal sessions.
async fn list_sessions(State(state): State<AppState>) -> Json<SessionListResponse> {
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
    State(state): State<AppState>,
    Json(req): Json<RenameRequest>,
) -> Result<Json<serde_json::Value>, crate::error::AppError> {
    let session_id: Uuid = req
        .session_id
        .parse()
        .map_err(|_| crate::error::AppError::NotFound("Invalid session ID".to_string()))?;
    state
        .terminal_manager
        .rename_session(session_id, req.label)
        .await
        .map_err(crate::error::AppError::NotFound)?;
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

async fn terminal_input(
    State(state): State<AppState>,
    Json(req): Json<InputRequest>,
) -> Result<(), crate::error::AppError> {
    let session_id = Uuid::parse_str(&req.session_id)
        .map_err(|e| crate::error::AppError::BadRequest(format!("Invalid session_id: {e}")))?;

    state
        .terminal_manager
        .write_input(session_id, req.data.as_bytes())
        .await
        .map_err(crate::error::AppError::Internal)?;

    Ok(())
}

async fn terminal_resize(
    State(state): State<AppState>,
    Json(req): Json<ResizeRequest>,
) -> Result<(), crate::error::AppError> {
    let session_id = Uuid::parse_str(&req.session_id)
        .map_err(|e| crate::error::AppError::BadRequest(format!("Invalid session_id: {e}")))?;

    state
        .terminal_manager
        .resize(session_id, req.cols, req.rows)
        .await
        .map_err(crate::error::AppError::Internal)?;

    Ok(())
}

/// SSE endpoint for terminal output.
/// Public endpoint (bypasses auth middleware) because EventSource
/// cannot set custom headers. Auth is handled inline via query param.
/// Respects `cancel_token` for graceful shutdown.
async fn terminal_events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, crate::error::AppError>
{
    // Validate auth token if configured
    let auth = state.auth.read().await;
    if let Some(configured) = auth.web_token() {
        let provided = query.token.as_deref().unwrap_or("");
        if provided != configured {
            return Err(crate::error::AppError::Unauthorized(
                "Invalid or missing auth token".into(),
            ));
        }
    }
    drop(auth);

    let session_id = Uuid::parse_str(&query.session_id)
        .map_err(|e| crate::error::AppError::BadRequest(format!("Invalid session_id: {e}")))?;

    let cancel_token = state.cancel_token.clone();
    let mut rx = state.terminal_tx.subscribe();

    // Flush any buffered output that was produced before this subscriber
    // connected (e.g. the initial shell prompt).
    let buf = state.terminal_manager.get_buffer(session_id).await;
    let initial: Vec<Result<Event, Infallible>> = if buf.is_empty() {
        Vec::new()
    } else {
        let text = String::from_utf8_lossy(&buf).to_string();
        vec![Ok(Event::default().event("terminal.output").data(text))]
    };

    let stream = async_stream::stream! {
        for evt in initial {
            yield evt;
        }

        loop {
            tokio::select! {
                // Check for shutdown signal
                _ = cancel_token.cancelled() => {
                    log::debug!("terminal SSE closing due to shutdown for session {}", session_id);
                    break;
                }
                result = rx.recv() => {
                    let output = match result {
                        Ok(o) => o,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            log::warn!("terminal SSE lagged by {} for session {}", n, session_id);
                            continue;
                        }
                    };

                    if output.session_id != session_id {
                        continue;
                    }

                    if output.closed {
                        yield Ok(Event::default().event("terminal.close").data(""));
                        break;
                    } else {
                        let text = String::from_utf8_lossy(&output.data).to_string();
                        yield Ok(Event::default().event("terminal.output").data(text));
                    }
                }
            }
        }

        log::debug!("terminal SSE stream ended for session {}", session_id);
    };

    Ok(Sse::new(stream))
}

/// WebSocket endpoint for terminal I/O.
///
/// Public endpoint (bypasses auth middleware) because the browser WebSocket API
/// cannot set custom headers. Auth is handled inline via query param.
///
/// Protocol: JSON arrays over text frames.
///   Client → Server:
///     ["bind", "<session_id>"]
///     ["stdin", "<text>"]
///     ["resize", <rows>, <cols>]
///   Server → Client:
///     ["setup"]
///     ["stdout", "<text>"]
///     ["disconnect", "<reason>"]
async fn terminal_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
) -> Result<impl IntoResponse, crate::error::AppError> {
    // Validate auth token if configured
    let auth = state.auth.read().await;
    if let Some(configured) = auth.web_token() {
        let provided = query.token.as_deref().unwrap_or("");
        if provided != configured {
            return Err(crate::error::AppError::Unauthorized(
                "Invalid or missing auth token".into(),
            ));
        }
    }
    drop(auth);

    Ok(ws.on_upgrade(move |socket| handle_terminal_ws(socket, state)))
}

async fn handle_terminal_ws(mut ws: WebSocket, state: AppState) {
    let cancel_token = state.cancel_token.clone();
    let terminal_manager = state.terminal_manager;
    let terminal_tx = state.terminal_tx;

    // Wait for the bind message to get the session_id.
    let session_id = loop {
        let msg = match tokio::time::timeout(std::time::Duration::from_secs(10), ws.recv()).await {
            Ok(Some(Ok(msg))) => msg,
            Ok(Some(Err(e))) => {
                log::warn!("terminal WS recv error before bind: {e}");
                return;
            }
            Ok(None) | Err(_) => {
                let _ = ws
                    .send(Message::Text(json_msg(["disconnect", "bind timeout"])))
                    .await;
                return;
            }
        };

        let (msg_type, args) = match parse_ws_msg(&msg) {
            Ok(v) => v,
            Err(e) => {
                let _ = ws.send(Message::Text(json_msg(["disconnect", &e]))).await;
                continue;
            }
        };

        if msg_type != "bind" {
            let _ = ws
                .send(Message::Text(json_msg(["disconnect", "expected bind"])))
                .await;
            continue;
        }

        let session_id_str = match args.first().and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                let _ = ws
                    .send(Message::Text(json_msg([
                        "disconnect",
                        "bind missing session_id",
                    ])))
                    .await;
                continue;
            }
        };

        let sid = match Uuid::parse_str(session_id_str) {
            Ok(id) => id,
            Err(e) => {
                let _ = ws
                    .send(Message::Text(json_msg([
                        "disconnect",
                        &format!("invalid session_id: {e}"),
                    ])))
                    .await;
                continue;
            }
        };

        if !terminal_manager.has_session(sid).await {
            let _ = ws
                .send(Message::Text(json_msg(["disconnect", "session not found"])))
                .await;
            continue;
        }

        // Send setup signal — session is ready
        let _ = ws.send(Message::Text(json_msg(["setup"]))).await;
        break sid;
    };

    // Flush any buffered output that was produced before this subscriber
    // connected (e.g. the initial shell prompt), matching the SSE handler.
    let buf = terminal_manager.get_buffer(session_id).await;
    if !buf.is_empty()
        && let Ok(text) = String::from_utf8(buf)
    {
        let _ = ws.send(Message::Text(json_msg(["stdout", &text]))).await;
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
                            .send(Message::Text(json_msg(["disconnect", "session closed"])))
                            .await;
                        break;
                    }

                    if let Ok(text) = String::from_utf8(output.data) {
                        let _ = output_tx_clone
                            .send(Message::Text(json_msg(["stdout", &text])))
                            .await;
                    }
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
                    Some(Ok(Message::Text(data))) => {
                        // Parse JSON array
                        let (msg_type, args) = match parse_ws_msg_raw(&data) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };

                        match msg_type.as_str() {
                            "stdin" => {
                                if let Some(text) = args.first().and_then(|v| v.as_str()) {
                                    let _ = terminal_manager
                                        .write_input(session_id, text.as_bytes())
                                        .await;
                                }
                            }
                            "resize"
                                if args.len() >= 2 => {
                                    let rows = args[0].as_u64().unwrap_or(24) as u16;
                                    let cols = args[1].as_u64().unwrap_or(80) as u16;
                                    let _ = terminal_manager
                                        .resize(session_id, cols, rows)
                                        .await;
                                }
                            _ => {
                                // Unknown message type — ignore
                            }
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        // Treat binary as stdin
                        let _ = terminal_manager
                            .write_input(session_id, &data)
                            .await;
                    }
                    Some(Ok(Message::Close(_))) => break,
                    Some(Err(_)) => break,
                    None => break,
                    _ => {}
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

/// Parse a JSON array from a text WebSocket message (string slice).
/// Returns (message_type, args).
fn parse_ws_msg_raw(text: &str) -> Result<(String, Vec<serde_json::Value>), String> {
    let arr: Vec<serde_json::Value> =
        serde_json::from_str(text).map_err(|e| format!("invalid JSON array: {e}"))?;

    if arr.is_empty() {
        return Err("empty message array".to_string());
    }

    let msg_type = arr[0]
        .as_str()
        .ok_or_else(|| "first element must be a string type".to_string())?;

    Ok((msg_type.to_string(), arr[1..].to_vec()))
}

async fn close_terminal(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Ok(id) = Uuid::parse_str(&session_id) {
        state.terminal_manager.close_session(id).await;
    }
    axum::http::StatusCode::OK
}
