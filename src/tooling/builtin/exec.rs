use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    io::Read,
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use tokio::sync::mpsc::UnboundedSender;

use super::utils::truncate_in_place;
use crate::session::BackendEvent;
use crate::tooling::tools::{BashArgs, decode_tool_args};
use crate::tooling::{ToolDefinition, ToolPermission};
use uuid::Uuid;

/// Result of bash tool execution, including whether RTK rewrote the command.
#[derive(Debug)]
pub struct BashExecutionResult {
    pub output: String,
    pub rtk_rewritten: bool,
}

pub fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition::new::<BashArgs>(
        "bash",
        "Run a shell command in the workspace root",
        ToolPermission::Execute,
    )]
}

pub fn execute_tool_call(
    workspace_root: &Path,
    tool_name: &str,
    arguments: Value,
    max_output_bytes: usize,
    rtk_enabled: bool,
    session_id: Uuid,
    event_tx: Option<UnboundedSender<BackendEvent>>,
) -> Result<BashExecutionResult> {
    let args = decode_tool_args::<BashArgs>(tool_name, arguments)?;
    let timeout = args.timeout.unwrap_or(120_000) as u64; // default 2 minutes
    run_shell_inner(
        workspace_root,
        &args.command,
        max_output_bytes,
        rtk_enabled,
        None,
        timeout,
        event_tx,
        session_id,
    )
}

pub fn execute_tool_call_with_cancel(
    workspace_root: &Path,
    tool_name: &str,
    arguments: Value,
    max_output_bytes: usize,
    rtk_enabled: bool,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    session_id: Uuid,
    event_tx: Option<UnboundedSender<BackendEvent>>,
) -> Result<BashExecutionResult> {
    let args = decode_tool_args::<BashArgs>(tool_name, arguments)?;
    let timeout = args.timeout.unwrap_or(120_000) as u64; // default 2 minutes
    run_shell_inner(
        workspace_root,
        &args.command,
        max_output_bytes,
        rtk_enabled,
        Some(cancelled),
        timeout,
        event_tx,
        session_id,
    )
}

fn run_shell_inner(
    workspace_root: &Path,
    command: &str,
    max_output_bytes: usize,
    rtk_enabled: bool,
    cancelled: Option<Arc<AtomicBool>>,
    timeout_ms: u64,
    event_tx: Option<UnboundedSender<BackendEvent>>,
    session_id: Uuid,
) -> Result<BashExecutionResult> {
    // Try to get RTK rewritten command if RTK is enabled
    let (actual_command, rtk_rewritten) = if rtk_enabled {
        let result = rewrite_command(command);
        (result.command, result.rewritten)
    } else {
        (command.to_string(), false)
    };

    let mut process = if cfg!(target_os = "windows") {
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &actual_command])
            .current_dir(workspace_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to run command '{actual_command}'"))?
    } else {
        std::process::Command::new("sh")
            .arg("-lc")
            .arg(&actual_command)
            .current_dir(workspace_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to run command '{actual_command}'"))?
    };

    let mut stderr = process.stderr.take();
    let start_time = std::time::Instant::now();
    let timeout = Duration::from_millis(timeout_ms);

    // ─── Stream stdout chunk-by-chunk via a reader thread ──────────────
    // We spawn a reader thread so the main loop can still check for
    // cancellation / timeout while output trickles in slowly.
    // Raw bytes are accumulated and converted to string at the end to
    // avoid corrupting multi-byte UTF-8 sequences across chunks.
    let (chunk_tx, chunk_rx) = mpsc::channel::<Vec<u8>>();
    let mut raw_bytes: Vec<u8> = Vec::new();
    let mut output_buf = String::new();

    if let Some(stdout_handle) = process.stdout.take() {
        thread::spawn(move || {
            let mut reader = stdout_handle;
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        if chunk_tx.send(buf[..n].to_vec()).is_err() {
                            break; // receiver dropped
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // Main loop: check cancel/timeout, read chunks from the reader thread
    loop {
        if cancelled
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            let _ = process.kill();
            let _ = process.wait();
            return Err(anyhow::anyhow!("shell command cancelled"));
        }

        // Check timeout
        if start_time.elapsed() > timeout {
            let _ = process.kill();
            let _ = process.wait();
            return Err(anyhow::anyhow!(
                "bash tool terminated command after exceeding timeout {} ms. \
                 If this command is expected to take longer and is not waiting for interactive input, \
                 retry with a larger timeout value in milliseconds.",
                timeout_ms
            ));
        }

        match chunk_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                raw_bytes.extend_from_slice(&chunk);

                // Convert accumulated bytes to string for display.
                // Using from_utf8_lossy on the whole buffer is safe across chunk
                // boundaries — any leftover partial UTF-8 from a previous chunk
                // is now complete in this chunk.
                let output_str = String::from_utf8_lossy(&raw_bytes);
                output_buf = output_str.into_owned();

                // If the string was truncated in a previous iteration, align
                // raw_bytes to match so we don't accumulate forever.
                if raw_bytes.len() > max_output_bytes {
                    // Find the byte boundary corresponding to max_output_bytes chars
                    let truncated_str: String = output_buf.chars().take(max_output_bytes).collect();
                    let byte_len = truncated_str.len();
                    raw_bytes.truncate(byte_len);
                    output_buf = truncated_str;
                    if raw_bytes.len() > max_output_bytes {
                        // Safety: ensure raw_bytes doesn't exceed max
                        raw_bytes.truncate(max_output_bytes);
                        let safe = String::from_utf8_lossy(&raw_bytes);
                        output_buf = safe.into_owned();
                    }
                } else {
                    truncate_in_place(&mut output_buf, max_output_bytes);
                    // Sync raw_bytes to truncated string length
                    raw_bytes.truncate(output_buf.len());
                }

                // Send progress event
                if let Some(ref tx) = event_tx {
                    let _ = tx.send(BackendEvent::ShellOutput {
                        session_id,
                        content: output_buf.clone(),
                        finished: false,
                        exit_code: None,
                    });
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break, // reader thread finished
        }
    }

    // ─── Process finished ──────────────────────────────────────────────
    let status = process.wait().context("failed to wait for shell command")?;
    let exit_code = status.code();

    // Merge stderr output
    let mut combined = output_buf;
    if let Some(mut handle) = stderr.take() {
        let mut error_output = String::new();
        let _ = handle.read_to_string(&mut error_output);
        if !error_output.is_empty() {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&error_output);
        }
    }

    truncate_in_place(&mut combined, max_output_bytes);

    // Send final event with exit code
    if let Some(ref tx) = event_tx {
        let _ = tx.send(BackendEvent::ShellOutput {
            session_id,
            content: combined.clone(),
            finished: true,
            exit_code,
        });
    }

    let status_code = exit_code.unwrap_or_default();
    Ok(BashExecutionResult {
        output: format!("[exit {status_code}]\n{combined}"),
        rtk_rewritten,
    })
}

/// Result of RTK rewrite operation.
struct RewriteResult {
    command: String,
    rewritten: bool,
}

/// Try to rewrite a command using RTK's rewrite feature.
/// Returns the RTK rewritten command if available, otherwise the original command.
///
/// RTK rewrite exit codes:
/// - Exit 0: Command rewritten and allowed
/// - Exit 1: No RTK equivalent, use original command
/// - Exit 2: Deny rule matched
/// - Exit 3: Command rewritten but needs user confirmation (ask)
///
/// For exit 0 and 3, we use the rewritten command.
fn rewrite_command(command: &str) -> RewriteResult {
    let output = std::process::Command::new("rtk")
        .arg("rewrite")
        .arg(command)
        .output()
        .ok();

    match output {
        Some(output) => {
            let exit_code = output.status.code().unwrap_or(1);
            // Exit 0 (allow) or Exit 3 (ask) means command was rewritten
            if exit_code == 0 || exit_code == 3 {
                let rewritten = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !rewritten.is_empty() && rewritten != command {
                    return RewriteResult {
                        command: rewritten,
                        rewritten: true,
                    };
                }
            }
            RewriteResult {
                command: command.to_string(),
                rewritten: false,
            }
        }
        None => RewriteResult {
            command: command.to_string(),
            rewritten: false,
        },
    }
}
