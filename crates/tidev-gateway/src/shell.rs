//! Shared shell command execution for gateway channels.
//!
//! Provides:
//! - `execute_shell`: run a command via `sh -c` / `powershell -Command`
//! - `format_shell_output`: format raw output + exit code as Markdown (for DB storage)
//! - `format_shell_output_html`: format raw output + exit code as HTML (for Telegram replies)
//! - `persist_shell_messages`: persist shell user message and output message to the store

use anyhow::Result;
use uuid::Uuid;

use tidev_session::session::{Message, MessageRole};
use tidev_storage::SessionStore;

/// Execute a shell command and return (raw_output, exit_code).
///
/// Platform-agnostic: uses `sh -c` on Unix, `powershell -Command` on Windows.
pub fn execute_shell(command: &str) -> (String, Option<i32>) {
    let (shell, arg) = shell_command();
    let result = std::process::Command::new(shell)
        .arg(arg)
        .arg(command)
        .output();

    match result {
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
        Err(error) => (format!("Failed to execute command: {error}"), None),
    }
}

/// Format shell output as Markdown code block for DB storage.
///
/// Matches the format used by the web API and TUI so that Gateway-persisted
/// messages render correctly in the Web frontend's ShellBlock component.
pub fn format_shell_output(content: &str, exit_code: Option<i32>) -> String {
    if !content.is_empty() {
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
    }
}

/// Format shell output as HTML for Telegram replies.
///
/// Content is HTML-escaped and wrapped in `<pre><code>` tags.
pub fn format_shell_output_html(content: &str, exit_code: Option<i32>) -> String {
    let escaped = content
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");

    if !content.is_empty() {
        match exit_code {
            Some(0) => format!("<pre><code>{escaped}</code></pre>"),
            Some(code) => {
                format!("<pre><code>{escaped}</code></pre>\n\nExit code: <b>{code}</b>")
            }
            None => format!("<pre><code>{escaped}</code></pre>"),
        }
    } else {
        match exit_code {
            Some(0) => "Command completed successfully (no output)".to_string(),
            Some(code) => format!("Exit code: <b>{code}</b>"),
            None => "Command completed (no output)".to_string(),
        }
    }
}

/// Persist shell user message (`$ {command}`) and output message to the store.
pub fn persist_shell_messages(
    store: &SessionStore,
    session_id: Uuid,
    command: &str,
    formatted_output: &str,
) -> Result<()> {
    let user_msg = Message::new(MessageRole::Shell, format!("$ {command}"));
    store.append_message(session_id, &user_msg)?;

    let output_msg = Message::new(MessageRole::Shell, formatted_output);
    store.append_message(session_id, &output_msg)?;

    Ok(())
}

/// Determine the shell command to use based on the platform.
fn shell_command() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("powershell", "-Command")
    } else {
        ("sh", "-c")
    }
}
