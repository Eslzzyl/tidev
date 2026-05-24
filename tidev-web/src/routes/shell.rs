//! Shell command execution route for the web API.
//!
//! - `POST /api/sessions/{id}/shell` — Execute a shell command

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use tidev_engine::{
    session::{Message, MessageRole},
};
use crate::{
    error::{AppError, WebResult},
    event_bus::AppEvent,
    state::AppState,
};// ── Request / Response types ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ShellCommandRequest {
    pub command: String,
}

#[derive(Serialize)]
pub struct ShellCommandResponse {
    pub request_id: u64,
}

// ── Handler ───────────────────────────────────────────────────────────────

/// Execute a shell command and stream the output back via SSE.
pub async fn execute_shell_command(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<Uuid>,
    Json(body): Json<ShellCommandRequest>,
) -> WebResult<(StatusCode, Json<ShellCommandResponse>)> {
    let command = body.command.trim().to_string();
    if command.is_empty() {
        return Err(AppError::BadRequest("Empty command".to_string()));
    }

    // Verify session exists
    {
        let store = state.store.lock().await;
        store
            .load_session_record(session_id)?
            .ok_or_else(|| AppError::NotFound(format!("Session {} not found", session_id)))?;
    }

    // Generate request ID
    let request_id = rand::random::<u64>();

    // Create and persist the user message showing the shell command
    let user_message = Message::new(MessageRole::Shell, format!("$ {command}"));
    {
        let store = state.store.lock().await;
        store.append_message(session_id, &user_message)?;
    }

    let event_bus = state.event_bus.clone();
    let store_arc = state.store.clone();

    // Spawn async task to execute the command
    tokio::spawn(async move {
        let output_message_id = Uuid::new_v4();
        let now = chrono::Utc::now();

        let (shell, arg) = shell_command();
        let result = std::process::Command::new(shell)
            .arg(arg)
            .arg(&command)
            .output();

        let (content, exit_code) = match result {
            Ok(output) => {
                let exit_code = output.status.code();
                let mut content = String::new();

                if output.status.success() {
                    content = String::from_utf8_lossy(&output.stdout)
                        .trim_end()
                        .to_string();
                    if content.is_empty() {
                        content = String::from_utf8_lossy(&output.stderr)
                            .trim_end()
                            .to_string();
                    }
                } else {
                    if !output.stdout.is_empty() {
                        content.push_str(String::from_utf8_lossy(&output.stdout).trim_end());
                    }
                    if !output.stderr.is_empty() {
                        if !content.is_empty() {
                            content.push('\n');
                        }
                        content.push_str(String::from_utf8_lossy(&output.stderr).trim_end());
                    }
                }

                (content, exit_code)
            }
            Err(error) => {
                let content = format!("Failed to execute command: {error}");
                let formatted = content.clone();

                // Send error via SSE
                event_bus.publish(AppEvent::ShellOutput {
                    session_id,
                    content: formatted,
                    finished: true,
                    exit_code: None,
                });

                // Persist the error message
                let mut msg = Message::new(MessageRole::Shell, &content);
                msg.id = output_message_id;
                msg.created_at = now;
                msg.completed_at = Some(chrono::Utc::now());
                {
                    let store = store_arc.lock().await;
                    if let Err(e) = store.append_message(session_id, &msg) {
                        log::warn!("ShellOutput: failed to persist message: {}", e);
                    }
                }
                return;
            }
        };

        // Format as code block
        let formatted = if !content.is_empty() {
            match exit_code {
                Some(0) => format!("```\n{content}\n```"),
                Some(code) => format!("```\n{content}\n```\n\nExit code: {code}"),
                None => format!("```\n{content}\n```"),
            }
        } else {
            match exit_code {
                Some(0) => "Command completed successfully (no output)".to_string(),
                Some(code) => format!("Exit code: {code}"),
                None => "Command completed (no output)".to_string(),
            }
        };

        // Send ShellOutput event via SSE
        event_bus.publish(AppEvent::ShellOutput {
            session_id,
            content: formatted.clone(),
            finished: true,
            exit_code,
        });

        // Persist the output message
        let mut msg = Message::new(MessageRole::Shell, &formatted);
        msg.id = output_message_id;
        msg.created_at = now;
        msg.completed_at = Some(chrono::Utc::now());
        {
            let store = store_arc.lock().await;
            if let Err(e) = store.append_message(session_id, &msg) {
                log::warn!("ShellOutput: failed to persist message: {}", e);
            }
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(ShellCommandResponse { request_id }),
    ))
}

/// Determine the shell command to use based on the platform.
fn shell_command() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("powershell", "-Command")
    } else {
        ("sh", "-c")
    }
}
