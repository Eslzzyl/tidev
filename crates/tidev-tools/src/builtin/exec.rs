use anyhow::{Context, Result};
use serde_json::Value;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::sync::LazyLock;
use std::{
    collections::HashSet,
    io::Read,
    path::Path,
    process::Stdio,
    sync::{Mutex, mpsc},
    thread,
    time::Duration,
};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::utils::truncate_in_place;
use crate::builtin::classify::{Classifier, Safety};
use crate::builtin::utils::decode_tool_args;
use crate::types::{ShellArgs, ToolDefinition, ToolPermission};
use tidev_utils::encoding::decode_command_output;
use tidev_utils::encoding::prepare_command_for_shell;

use super::ShellOutput;

/// Registry of active child process PIDs spawned by the shell tool.
/// Used during program exit to prevent orphaned processes.
static ACTIVE_CHILDREN: LazyLock<Mutex<HashSet<u32>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Register a child PID so it can be killed on program exit.
pub fn register_child(pid: u32) {
    ACTIVE_CHILDREN.lock().unwrap().insert(pid);
}

/// Unregister a child PID that has exited normally.
pub fn unregister_child(pid: u32) {
    ACTIVE_CHILDREN.lock().unwrap().remove(&pid);
}

/// Kill all tracked child processes. Two-phase: SIGTERM → brief wait → SIGKILL.
#[cfg(unix)]
pub fn kill_all_children() {
    let pids: Vec<u32> = ACTIVE_CHILDREN.lock().unwrap().iter().copied().collect();
    if pids.is_empty() {
        return;
    }

    // Phase 1: SIGTERM — graceful shutdown
    for &pid in &pids {
        let _ = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    }

    // Give them a moment to exit cleanly
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Phase 2: SIGKILL — force kill survivors
    for &pid in &pids {
        let _ = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    }
}

/// Kill a process group by its leader PID.
///
/// Uses two-phase termination (SIGTERM → brief wait → SIGKILL) to give
/// the process and its descendants a chance to clean up (e.g. restore
/// terminal settings) before being forcefully killed.
///
/// After `setsid()` in pre_exec, the child's PID equals its PGID
/// (process group ID), so `kill(-pid, ...)` sends the signal to the
/// entire process group — including any grandchildren (git, editor, pager).
#[cfg(unix)]
pub fn kill_process_group(pid: u32) {
    unsafe {
        let _ = libc::kill(-(pid as i32), libc::SIGTERM);
    }
    // Give them a moment to exit cleanly (restore terminal, etc.)
    std::thread::sleep(std::time::Duration::from_millis(200));
    unsafe {
        let _ = libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

/// Kill a process group by its leader PID (no-op on non-Unix).
#[cfg(not(unix))]
pub fn kill_process_group(pid: u32) {
    // Fallback: just kill the individual process on non-Unix platforms.
    // This won't kill grandchildren, but it's the best we can do without
    // process group support.
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .output();
}

/// Kill all tracked child processes (no-op on non-Unix).
#[cfg(not(unix))]
pub fn kill_all_children() {
    // Windows support could be added later using TerminateProcess
}

/// Result of shell tool execution.
#[derive(Debug)]
pub struct ShellExecutionResult {
    pub output: String,
    pub exit_code: Option<i32>,
}

pub fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition::new::<ShellArgs>(
        "shell",
        "Run a shell command in the workspace root",
        ToolPermission::Execute,
    )]
}

#[allow(clippy::too_many_arguments)]
pub fn execute_tool_call(
    workspace_root: &Path,
    tool_name: &str,
    arguments: Value,
    max_output_bytes: usize,
    read_only: bool,
    session_id: Uuid,
    request_id: u64,
    event_tx: Option<UnboundedSender<ShellOutput>>,
) -> Result<ShellExecutionResult> {
    let args = decode_tool_args::<ShellArgs>(tool_name, arguments)?;
    let timeout = args.timeout.unwrap_or(120_000) as u64; // default 2 minutes
    run_shell_inner(
        workspace_root,
        &args.command,
        max_output_bytes,
        None,
        timeout,
        event_tx,
        read_only,
        session_id,
        request_id,
        "",
    )
}

#[allow(clippy::too_many_arguments)]
pub fn execute_tool_call_with_cancel(
    workspace_root: &Path,
    tool_name: &str,
    arguments: Value,
    max_output_bytes: usize,
    cancel: &CancellationToken,
    read_only: bool,
    session_id: Uuid,
    request_id: u64,
    event_tx: Option<UnboundedSender<ShellOutput>>,
) -> Result<ShellExecutionResult> {
    let args = decode_tool_args::<ShellArgs>(tool_name, arguments)?;
    let timeout = args.timeout.unwrap_or(120_000) as u64;
    run_shell_inner(
        workspace_root,
        &args.command,
        max_output_bytes,
        Some(cancel),
        timeout,
        event_tx,
        read_only,
        session_id,
        request_id,
        "",
    )
}

/// Execute a shell command with streaming output, cancellation, and timeout.
///
/// Uses `tokio::process::Command` for non-blocking process execution.
/// Stdout is streamed chunk-by-chunk via [`ShellOutput`] events.
/// The `cancel` token terminates the process group on cancellation.
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool_call_with_cancel_async(
    workspace_root: &Path,
    tool_name: &str,
    arguments: Value,
    max_output_bytes: usize,
    cancel: &CancellationToken,
    read_only: bool,
    session_id: Uuid,
    request_id: u64,
    event_tx: Option<UnboundedSender<ShellOutput>>,
    tool_call_id: &str,
) -> Result<ShellExecutionResult> {
    let args = decode_tool_args::<ShellArgs>(tool_name, arguments)?;
    let timeout = args.timeout.unwrap_or(120_000) as u64;
    run_shell_streaming(
        workspace_root,
        &args.command,
        max_output_bytes,
        cancel,
        timeout,
        event_tx,
        read_only,
        session_id,
        request_id,
        tool_call_id,
    )
    .await
}

/// Async streaming shell execution — non-blocking, supports cancellation and timeout.
///
/// Internally uses `tokio::process::Command` so the calling async task is never
/// blocked by a running shell command. Output is read chunk-by-chunk and
/// forwarded as [`ShellOutput`] events.
#[allow(clippy::too_many_arguments)]
async fn run_shell_streaming(
    workspace_root: &Path,
    command: &str,
    max_output_bytes: usize,
    cancel: &CancellationToken,
    timeout_ms: u64,
    event_tx: Option<UnboundedSender<ShellOutput>>,
    read_only: bool,
    session_id: Uuid,
    request_id: u64,
    tool_call_id: &str,
) -> Result<ShellExecutionResult> {
    let mut actual_command = command.to_string();

    // ── Layer 0: Plan mode command classification ────────────────────
    if read_only && Classifier::global().classify(command) >= Safety::WriteOperation {
        log::info!(
            "plan mode blocked write command: {}",
            command.lines().next().unwrap_or(command)
        );

        // Emit shell output so the TUI creates a streaming message
        // that ToolCompleted can finalize; otherwise the result is lost
        // because shell results rely on ShellOutput for display.
        emit_shell_output(
            event_tx.as_ref(),
            session_id,
            request_id,
            tool_call_id,
            "Error: Command blocked in Plan mode — this command appears \
                          to modify files."
                .to_string(),
            true,
            Some(1),
        );

        return Ok(ShellExecutionResult {
            output: "[exit 1]\nError: Command blocked in Plan mode — this command appears \
                     to modify files."
                .to_string(),
            exit_code: Some(1),
        });
    }

    // ── Layer 1: Privilege escalation handling (sudo/doas/pkexec) ──────
    let mut sudo_guard: Option<super::sudo::AskpassGuard> = None;
    let _sudo_active = if super::sudo::has_privilege_escalation(&actual_command) {
        let guard = super::sudo::create_askpass_script()?;
        let wrapped = super::sudo::wrap_command(&actual_command, guard.path());
        log::info!("sudo: privilege escalation detected, wrapping command with SUDO_ASKPASS");
        sudo_guard = Some(guard);
        actual_command = wrapped;
        true
    } else {
        false
    };

    // ── Build & spawn the shell process ───────────────────────────────
    let mut child = if cfg!(target_os = "windows") {
        let shell = crate::shell::get();
        let shell_command = prepare_command_for_shell(&actual_command, &shell.program, &shell.arg);

        let mut cmd = tokio::process::Command::new(&shell.program);
        let mut all_args: Vec<&str> = shell.arg.split_whitespace().collect();
        all_args.push(&shell_command);
        cmd.args(&all_args)
            .current_dir(workspace_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        cmd.env("LANG", "C.UTF-8");
        cmd.env("LC_ALL", "C.UTF-8");
        cmd.env("MSYS2_ENCODING", "UTF-8");
        cmd.env("PYTHONIOENCODING", "utf-8:surrogateescape");
        cmd.env("NO_COLOR", "1");

        if let Some(ref guard) = sudo_guard {
            cmd.env("SUDO_ASKPASS", guard.path());
        }

        cmd.spawn()
            .with_context(|| format!("failed to run command '{actual_command}'"))?
    } else {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-lc")
            .arg(&actual_command)
            .current_dir(workspace_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(ref guard) = sudo_guard {
            cmd.env("SUDO_ASKPASS", guard.path());
        }

        // Disconnect from controlling terminal (setsid) so the child has no
        // controlling terminal, preventing it from stealing the TUI's terminal.
        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(move || {
                libc::setsid();
                Ok(())
            });
        }

        cmd.spawn()
            .with_context(|| format!("failed to run command '{actual_command}'"))?
    };

    let child_pid = child.id().expect("child process should have a PID");
    register_child(child_pid);

    let mut stdout = child.stdout.take().expect("piped stdout");
    let stderr_handle = child.stderr.take();

    let timeout_dur = Duration::from_millis(timeout_ms);
    let deadline = tokio::time::Instant::now() + timeout_dur;

    let mut raw_bytes: Vec<u8> = Vec::new();
    let mut output_buf = String::new();
    let mut output_truncated = false;
    let mut read_buf = [0u8; 8192];

    // ── Async read loop with cancellation and timeout ─────────────────
    'read_loop: loop {
        tokio::select! {
            biased; // check cancel/timeout first

            _ = cancel.cancelled() => {
                kill_process_group(child_pid);
                let _ = child.wait().await;
                unregister_child(child_pid);

                emit_shell_output(
                    event_tx.as_ref(),
                    session_id,
                    request_id,
                    tool_call_id,
                    output_buf.clone(),
                    true,
                    None,
                );

                truncate_in_place(&mut output_buf, max_output_bytes);
                return Ok(ShellExecutionResult {
                    output: format!("[exit -1] (cancelled)\n{}", output_buf),
                    exit_code: Some(-1),
                });
            }

            _ = tokio::time::sleep_until(deadline) => {
                kill_process_group(child_pid);
                let _ = child.wait().await;
                unregister_child(child_pid);

                emit_shell_output(
                    event_tx.as_ref(),
                    session_id,
                    request_id,
                    tool_call_id,
                    output_buf.clone(),
                    true,
                    None,
                );

                truncate_in_place(&mut output_buf, max_output_bytes);
                return Ok(ShellExecutionResult {
                    output: format!(
                        "[exit -1] (timed out after {}s)\n{}",
                        timeout_ms / 1000,
                        output_buf
                    ),
                    exit_code: Some(-1),
                });
            }

            result = stdout.read(&mut read_buf) => {
                match result {
                    Ok(0) => break 'read_loop, // EOF
                    Ok(n) => {
                        if !output_truncated {
                            raw_bytes.extend_from_slice(&read_buf[..n]);
                            output_buf = decode_command_output(&raw_bytes);
                            if output_buf.len() > max_output_bytes {
                                truncate_in_place(&mut output_buf, max_output_bytes);
                                output_truncated = true;
                                raw_bytes.clear();
                            }
                        }

                        // Send streaming event
                        emit_shell_output(
                            event_tx.as_ref(),
                            session_id,
                            request_id,
                            tool_call_id,
                            output_buf.clone(),
                            false,
                            None,
                        );
                    }
                    Err(e) => {
                        log::error!("Failed to read from shell stdout: {e}");
                        break 'read_loop;
                    }
                }
            }
        }
    }

    // ─── Wait for process exit ────────────────────────────────────────
    let exit_code = match child.wait().await {
        Ok(status) => {
            unregister_child(child_pid);
            status.code()
        }
        Err(e) => {
            unregister_child(child_pid);
            return Err(anyhow::anyhow!("failed to wait for shell command: {e}"));
        }
    };

    // ─── Merge stderr ─────────────────────────────────────────────────
    let mut combined = output_buf;
    if let Some(mut stderr_handle) = stderr_handle {
        let mut stderr_bytes = Vec::new();
        let _ = stderr_handle.read_to_end(&mut stderr_bytes).await;
        let error_output = decode_command_output(&stderr_bytes);
        if !error_output.is_empty() {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&error_output);
        }
    }

    truncate_in_place(&mut combined, max_output_bytes);

    // Send final event with exit code
    emit_shell_output(
        event_tx.as_ref(),
        session_id,
        request_id,
        tool_call_id,
        combined.clone(),
        true,
        exit_code,
    );

    let status_code = exit_code.unwrap_or_default();

    Ok(ShellExecutionResult {
        output: format!("[exit {status_code}]\n{}", combined),
        exit_code,
    })
}
#[allow(clippy::too_many_arguments)]
fn run_shell_inner(
    workspace_root: &Path,
    command: &str,
    max_output_bytes: usize,
    cancel: Option<&CancellationToken>,
    timeout_ms: u64,
    event_tx: Option<UnboundedSender<ShellOutput>>,
    read_only: bool,
    session_id: Uuid,
    request_id: u64,
    tool_call_id: &str,
) -> Result<ShellExecutionResult> {
    let mut actual_command = command.to_string();

    // ── Layer 0: Plan mode command classification ────────────────────
    if read_only && Classifier::global().classify(command) >= Safety::WriteOperation {
        log::info!(
            "plan mode blocked write command: {}",
            command.lines().next().unwrap_or(command)
        );

        // Emit shell output so the TUI creates a streaming message
        // that ToolCompleted can finalize; otherwise the result is lost
        // because shell results rely on ShellOutput for display.
        emit_shell_output(
            event_tx.as_ref(),
            session_id,
            request_id,
            tool_call_id,
            "Error: Command blocked in Plan mode — this command appears \
                          to modify files."
                .to_string(),
            true,
            Some(1),
        );

        return Ok(ShellExecutionResult {
            output: "[exit 1]\nError: Command blocked in Plan mode — this command appears \
                     to modify files."
                .to_string(),
            exit_code: Some(1),
        });
    }

    // ── Layer 1: Privilege escalation handling (sudo/doas/pkexec) ──────
    let mut sudo_guard: Option<super::sudo::AskpassGuard> = None;
    let _sudo_active = if super::sudo::has_privilege_escalation(&actual_command) {
        let guard = super::sudo::create_askpass_script()?;
        let wrapped = super::sudo::wrap_command(&actual_command, guard.path());
        log::info!("sudo: privilege escalation detected, wrapping command with SUDO_ASKPASS");
        sudo_guard = Some(guard);
        actual_command = wrapped;
        true
    } else {
        false
    };

    let mut process = if cfg!(target_os = "windows") {
        let shell = crate::shell::get();

        // Prepend shell-specific encoding setup so that Windows programs
        // output UTF-8 instead of the system ANSI code page.
        let shell_command = prepare_command_for_shell(&actual_command, &shell.program, &shell.arg);

        let mut cmd = std::process::Command::new(&shell.program);
        // arg may contain spaces (e.g. "-NoProfile -Command")
        let mut all_args: Vec<&str> = shell.arg.split_whitespace().collect();
        all_args.push(&shell_command);
        cmd.args(&all_args)
            .current_dir(workspace_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Set environment variables to encourage UTF-8 output from child
        // processes.  This helps Git Bash / MSYS2 / Python / etc.
        cmd.env("LANG", "C.UTF-8");
        cmd.env("LC_ALL", "C.UTF-8");
        cmd.env("MSYS2_ENCODING", "UTF-8");
        cmd.env("PYTHONIOENCODING", "utf-8:surrogateescape");
        cmd.env("NO_COLOR", "1");

        // Inject SUDO_ASKPASS for privilege escalation handling
        if let Some(ref guard) = sudo_guard {
            cmd.env("SUDO_ASKPASS", guard.path());
        }

        cmd.spawn()
            .with_context(|| format!("failed to run command '{actual_command}'"))?
    } else {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-lc")
            .arg(&actual_command)
            .current_dir(workspace_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // ── Layer 1: Inject SUDO_ASKPASS environment variable ──
        // This tells sudo -A where to find the askpass helper.
        if let Some(ref guard) = sudo_guard {
            cmd.env("SUDO_ASKPASS", guard.path());
        }

        // ── Layer 2: Disconnect from controlling terminal ──
        // Create a new session (setsid) so the child process has no
        // controlling terminal. This means open("/dev/tty") will fail
        // with ENXIO, preventing the child from stealing the TUI's
        // terminal or corrupting its settings.
        // This is done in pre_exec (after fork, before exec) so it
        // only affects the child process.
        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(move || {
                libc::setsid();
                Ok(())
            });
        }

        cmd.spawn()
            .with_context(|| format!("failed to run command '{actual_command}'"))?
    };

    // Register child PID so it can be killed on program exit if needed.
    let child_pid = process.id();
    register_child(child_pid);

    let mut stderr = process.stderr.take();
    let start_time = std::time::Instant::now();
    let timeout = Duration::from_millis(timeout_ms);

    // ─── Stream stdout chunk-by-chunk via a reader thread ──────────────
    let (chunk_tx, chunk_rx) = mpsc::channel::<Vec<u8>>();
    let mut raw_bytes: Vec<u8> = Vec::new();
    let mut output_buf = String::new();
    let mut output_truncated = false;

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
        if cancel.is_some_and(|c| c.is_cancelled()) {
            // Kill the entire process group (PID == PGID after setsid())
            kill_process_group(child_pid);
            let _ = process.wait();
            unregister_child(child_pid);

            // Send final shell output event so UI consumers see the last state.
            emit_shell_output(
                event_tx.as_ref(),
                session_id,
                request_id,
                tool_call_id,
                output_buf.clone(),
                true,
                None,
            );

            // Only show the output we got so far (truncated at max)
            truncate_in_place(&mut output_buf, max_output_bytes);
            return Ok(ShellExecutionResult {
                output: format!("[exit -1] (cancelled)\n{}", output_buf),
                exit_code: Some(-1),
            });
        }

        if start_time.elapsed() > timeout && timeout_ms > 0 {
            kill_process_group(child_pid);
            let _ = process.wait();
            unregister_child(child_pid);

            emit_shell_output(
                event_tx.as_ref(),
                session_id,
                request_id,
                tool_call_id,
                output_buf.clone(),
                true,
                None,
            );

            truncate_in_place(&mut output_buf, max_output_bytes);
            return Ok(ShellExecutionResult {
                output: format!(
                    "[exit -1] (timed out after {}s)\n{}",
                    timeout_ms / 1000,
                    output_buf
                ),
                exit_code: Some(-1),
            });
        }

        match chunk_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                if !output_truncated {
                    raw_bytes.extend_from_slice(&chunk);
                    // Decode output from raw bytes, tolerating non-UTF-8
                    // encoding when UTF-8 decoding fails.
                    output_buf = decode_command_output(&raw_bytes);
                    if output_buf.len() > max_output_bytes {
                        truncate_in_place(&mut output_buf, max_output_bytes);
                        output_truncated = true;
                        raw_bytes.clear();
                    }
                }

                // Send streaming event.
                emit_shell_output(
                    event_tx.as_ref(),
                    session_id,
                    request_id,
                    tool_call_id,
                    output_buf.clone(),
                    false,
                    None,
                );
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break, // reader thread finished
        }
    }

    // ─── Process finished ──────────────────────────────────────────────
    let status = process.wait().context("failed to wait for shell command")?;
    unregister_child(child_pid);
    let exit_code = status.code();

    // Merge stderr output
    let mut combined = output_buf;
    if let Some(mut handle) = stderr.take() {
        let mut stderr_bytes = Vec::new();
        let _ = handle.read_to_end(&mut stderr_bytes);
        let error_output = decode_command_output(&stderr_bytes);
        if !error_output.is_empty() {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&error_output);
        }
    }

    truncate_in_place(&mut combined, max_output_bytes);

    // Send final event with exit code
    emit_shell_output(
        event_tx.as_ref(),
        session_id,
        request_id,
        tool_call_id,
        combined.clone(),
        true,
        exit_code,
    );

    let status_code = exit_code.unwrap_or_default();

    Ok(ShellExecutionResult {
        output: format!("[exit {status_code}]\n{}", combined),
        exit_code,
    })
}

fn emit_shell_output(
    event_tx: Option<&UnboundedSender<ShellOutput>>,
    session_id: Uuid,
    request_id: u64,
    tool_call_id: &str,
    content: String,
    finished: bool,
    exit_code: Option<i32>,
) {
    if let Some(tx) = event_tx {
        let _ = tx.send(ShellOutput {
            session_id,
            request_id,
            tool_call_id: tool_call_id.to_string(),
            content,
            finished,
            exit_code,
        });
    }
}
