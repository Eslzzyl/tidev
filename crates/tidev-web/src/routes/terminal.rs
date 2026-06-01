//! Terminal API routes for web terminal.
//!
//! - `POST /api/terminal/start`     — Start a new terminal session (with optional cols/rows/shell)
//! - `POST /api/terminal/input`     — Send raw input to a terminal session
//! - `POST /api/terminal/resize`    — Resize the PTY (cols × rows)
//! - `GET  /api/terminal/events`    — SSE stream for terminal output
//! - `GET  /api/terminal/shells`    — List available shells on the server
//! - `GET  /api/terminal/ws`        — WebSocket endpoint for terminal I/O
//! - `DELETE /api/terminal/{id}`    — Close a terminal session

use std::convert::Infallible;
use std::collections::HashSet;

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
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
use crate::terminal::TerminalManager;

pub fn terminal_routes() -> Router<AppState> {
    Router::new()
        .route("/terminal/start", post(start_terminal))
        .route("/terminal/input", post(terminal_input))
        .route("/terminal/resize", post(terminal_resize))
        .route("/terminal/events", get(terminal_events))
        .route("/terminal/shells", get(list_shells))
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
}

/// Control frame tag byte for WebSocket binary protocol.
const CONTROL_TAG: u8 = 0x01;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsClientMessage {
    Bind { session_id: String },
    Resize { cols: u16, rows: u16 },
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

    let session_id = state
        .terminal_manager
        .start_session(state.terminal_tx.clone(), size, shell)
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
async fn list_shells(
    State(_state): State<AppState>,
) -> Json<ShellsResponse> {
    let shells = detect_shells();
    let default_shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    Json(ShellsResponse { shells, default_shell })
}

#[cfg(unix)]
fn detect_shells() -> Vec<ShellEntry> {
    let mut shells = Vec::new();
    let mut seen = HashSet::new();

    // 1. Always include $SHELL first
    if let Ok(s) = std::env::var("SHELL") {
        if !s.is_empty() && std::path::Path::new(&s).exists() {
            let name = std::path::Path::new(&s)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| s.clone());
            shells.push(ShellEntry { path: s.clone(), name });
            seen.insert(s);
        }
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
                shells.push(ShellEntry { path: line.to_string(), name });
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
    .chain(
        [
            format!("{}/.cargo/bin", home),
            format!("{}/.local/bin", home),
            format!("{}/.nix-profile/bin", home),
        ]
        .into_iter(),
    )
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
    if let Ok(s) = std::env::var("ComSpec") {
        if !s.is_empty() && std::path::Path::new(&s).exists() && !seen.contains(&s) {
            let name = std::path::Path::new(&s)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "cmd.exe".to_string());
            shells.push(ShellEntry { path: s.clone(), name });
            seen.insert(s);
        }
    }

    // 2. Include $SHELL if set (may be used by MSYS2/Cygwin environments)
    if let Ok(s) = std::env::var("SHELL") {
        if !s.is_empty() && std::path::Path::new(&s).exists() && !seen.contains(&s) {
            let name = std::path::Path::new(&s)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| s.clone());
            shells.push(ShellEntry { path: s.clone(), name });
            seen.insert(s);
        }
    }

    // 3. System-known shells from PATH (where we can resolve them)
    let path_dirs = std::env::var("PATH").unwrap_or_default();
    let shell_exes = ["powershell.exe", "pwsh.exe", "cmd.exe", "wsl.exe", "bash.exe", "nu.exe", "fish.exe", "zsh.exe", "ksh.exe", "tcsh.exe", "elvish.exe", "dash.exe"];
    for dir in path_dirs.split(';') {
        let dir = dir.trim();
        if dir.is_empty() {
            continue;
        }
        for name in &shell_exes {
            let path = format!("{}\\{}", dir, name);
            if std::path::Path::new(&path).exists() && !seen.contains(&path) {
                let display_name = name.trim_end_matches(".exe").to_string();
                shells.push(ShellEntry { path: path.clone(), name: display_name });
                seen.insert(path);
            }
        }
    }

    // 4. Cross-product search: common install directories × shell names
    let userprofile = std::env::var("USERPROFILE").unwrap_or_default();
    let localappdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let programfiles = std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".to_string());
    let programfiles_x86 = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| "C:\\Program Files (x86)".to_string());
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
        "powershell.exe", "pwsh.exe", "cmd.exe", "bash.exe",
        "nu.exe", "fish.exe", "zsh.exe", "ksh.exe", "tcsh.exe",
        "elvish.exe", "dash.exe", "sh.exe", "wsl.exe",
    ];

    for dir in &search_dirs {
        for name in &shell_names {
            let path = format!("{}\\{}", dir, name);
            if std::path::Path::new(&path).exists() && !seen.contains(&path) {
                let display_name = name.trim_end_matches(".exe").to_string();
                shells.push(ShellEntry { path: path.clone(), name: display_name });
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
/// Respects `cancel_token` for graceful shutdown.
async fn terminal_events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, crate::error::AppError>
{
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
/// Protocol:
///   Client → Server:
///     - `\x01{"type":"bind","session_id":"..."}` — bind to session (control frame)
///     - `\x01{"type":"resize","cols":N,"rows":M}` — resize PTY (control frame)
///     - raw text — terminal input to write to PTY
///
///   Server → Client:
///     - raw text — terminal output from PTY
///     - `\x01{"type":"close"}` — session closed (control frame)
async fn terminal_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_terminal_ws(socket, state))
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
                    .send(Message::Text(
                        "\x01{\"type\":\"error\",\"message\":\"bind timeout\"}".into(),
                    ))
                    .await;
                return;
            }
        };

        let sid = match parse_ws_bind(&msg) {
            Ok(sid) => sid,
            Err(e) => {
                let _ = ws
                    .send(Message::Text(
                        format!("\x01{{\"type\":\"error\",\"message\":\"{e}\"}}").into(),
                    ))
                    .await;
                continue;
            }
        };

        if !terminal_manager.has_session(sid).await {
            let _ = ws
                .send(Message::Text(
                    "\x01{\"type\":\"error\",\"message\":\"session not found\"}".into(),
                ))
                .await;
            continue;
        }

        // Send OK
        let _ = ws.send(Message::Text("\x01{\"type\":\"ok\"}".into())).await;
        break sid;
    };

    // Flush any buffered output that was produced before this subscriber
    // connected (e.g. the initial shell prompt), matching the SSE handler.
    let buf = terminal_manager.get_buffer(session_id).await;
    if !buf.is_empty()
        && let Ok(text) = String::from_utf8(buf)
    {
        let _ = ws.send(Message::Text(text.into())).await;
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
                _ = cancel_clone.cancelled() => {
                    break;
                }
                result = rx.recv() => {
                    match result {
                        Ok(output) if output.session_id == sid => {
                            if output.closed {
                                let _ = output_tx_clone
                                    .send(Message::Text("\x01{\"type\":\"close\"}".into()))
                                    .await;
                                break;
                            }
                            if let Ok(text) = String::from_utf8(output.data)
                                && output_tx_clone.send(Message::Text(text.into())).await.is_err() {
                                    break;
                                }
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            log::warn!("terminal WS lagged by {n} for session {sid}");
                            continue;
                        }
                    }
                }
            }
        }
    });

    // Main loop: read from WS (client input) and from output channel (PTY output).
    // Send periodic WebSocket pings to keep the connection alive through proxies.
    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(15));
    // Reset the stream so the first tick waits the full interval.
    ping_interval.reset();
    loop {
        tokio::select! {
            ws_msg = ws.recv() => {
                let ws_msg = match ws_msg {
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => {
                        log::warn!("terminal WS recv error: {e}");
                        break;
                    }
                    None => break,
                };

                if matches!(&ws_msg, Message::Close(_)) {
                    break;
                }

                handle_ws_message(
                    &ws_msg,
                    session_id,
                    &terminal_manager,
                    &output_tx,
                ).await;
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
                if ws.send(Message::Ping(bytes::Bytes::from_static(b"ping"))).await.is_err() {
                    break;
                }
            }
        }
    }

    output_task.abort();
}

/// Process a single WebSocket message from the client.
async fn handle_ws_message(
    msg: &Message,
    session_id: Uuid,
    terminal_manager: &TerminalManager,
    output_tx: &tokio::sync::mpsc::Sender<Message>,
) {
    match msg {
        Message::Text(t) => {
            let data = t.as_bytes().to_vec();

            // Check for control frames (starting with 0x01 byte)
            if data.first() == Some(&CONTROL_TAG) {
                if let Ok(rest) = std::str::from_utf8(&data[1..])
                    && let Ok(ctrl) = serde_json::from_str::<WsClientMessage>(rest)
                {
                    match ctrl {
                        WsClientMessage::Resize { cols, rows } => {
                            let _ = terminal_manager.resize(session_id, cols, rows).await;
                        }
                        WsClientMessage::Bind { .. } => {
                            let _ = output_tx
                                .send(Message::Text("\x01{\"type\":\"ok\"}".into()))
                                .await;
                        }
                    }
                }
                return;
            }

            // Raw text = terminal input
            let _ = terminal_manager.write_input(session_id, &data).await;
        }
        Message::Binary(d) => {
            let data = d.to_vec();

            // Check for control frames (starting with 0x01 byte)
            if data.first() == Some(&CONTROL_TAG) {
                if let Ok(rest) = std::str::from_utf8(&data[1..])
                    && let Ok(ctrl) = serde_json::from_str::<WsClientMessage>(rest)
                {
                    match ctrl {
                        WsClientMessage::Resize { cols, rows } => {
                            let _ = terminal_manager.resize(session_id, cols, rows).await;
                        }
                        WsClientMessage::Bind { .. } => {
                            let _ = output_tx
                                .send(Message::Text("\x01{\"type\":\"ok\"}".into()))
                                .await;
                        }
                    }
                }
                return;
            }

            // Raw binary = terminal input
            let _ = terminal_manager.write_input(session_id, &data).await;
        }
        Message::Close(_) => {}
        Message::Ping(data) => {
            let _ = output_tx.send(Message::Pong(data.clone())).await;
        }
        Message::Pong(_) => {}
    }
}

fn parse_ws_bind(msg: &Message) -> Result<Uuid, String> {
    let data = match msg {
        Message::Text(t) => t.as_bytes().to_vec(),
        Message::Binary(d) => d.to_vec(),
        _ => return Err("expected text or binary message".to_string()),
    };

    if data.first() != Some(&CONTROL_TAG) {
        return Err("expected control frame (0x01 prefix)".to_string());
    }

    let rest = std::str::from_utf8(&data[1..])
        .map_err(|_| "invalid UTF-8 in control frame".to_string())?;

    let ctrl: WsClientMessage =
        serde_json::from_str(rest).map_err(|e| format!("invalid control frame: {e}"))?;

    match ctrl {
        WsClientMessage::Bind { session_id } => {
            Uuid::parse_str(&session_id).map_err(|e| format!("invalid session_id: {e}"))
        }
        _ => Err("expected bind message".to_string()),
    }
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
