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
    sync::{mpsc, Mutex},
    thread,
    time::Duration,
};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use super::utils::truncate_in_place;
use crate::builtin::utils::decode_tool_args;
use tidev_types::tools::{BashArgs, ToolDefinition, ToolPermission};
use tidev_utils::encoding::decode_command_output;
use tidev_utils::encoding::prepare_command_for_shell;
use tidev_types::message::BackendEvent;
use uuid::Uuid;

/// Registry of active child process PIDs spawned by the bash tool.
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

/// Result of bash tool execution.
#[derive(Debug)]
pub struct BashExecutionResult {
    pub output: String,
}

pub fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition::new::<BashArgs>(
        "bash",
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
    session_id: Uuid,
    event_tx: Option<UnboundedSender<BackendEvent>>,
) -> Result<BashExecutionResult> {
    let args = decode_tool_args::<BashArgs>(tool_name, arguments)?;
    let timeout = args.timeout.unwrap_or(120_000) as u64; // default 2 minutes
    run_shell_inner(
        workspace_root,
        &args.command,
        max_output_bytes,
        None,
        timeout,
        event_tx,
        session_id,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn execute_tool_call_with_cancel(
    workspace_root: &Path,
    tool_name: &str,
    arguments: Value,
    max_output_bytes: usize,
    cancel: &CancellationToken,
    session_id: Uuid,
    event_tx: Option<UnboundedSender<BackendEvent>>,
) -> Result<BashExecutionResult> {
    let args = decode_tool_args::<BashArgs>(tool_name, arguments)?;
    let timeout = args.timeout.unwrap_or(120_000) as u64; // default 2 minutes
    run_shell_inner(
        workspace_root,
        &args.command,
        max_output_bytes,
        Some(cancel),
        timeout,
        event_tx,
        session_id,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_shell_inner(
    workspace_root: &Path,
    command: &str,
    max_output_bytes: usize,
    cancel: Option<&CancellationToken>,
    timeout_ms: u64,
    event_tx: Option<UnboundedSender<BackendEvent>>,
    session_id: Uuid,
) -> Result<BashExecutionResult> {
    let mut actual_command = command.to_string();

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
        let shell_command = prepare_command_for_shell(
            &actual_command,
            &shell.program,
            &shell.arg,
        );

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

            // Send final ShellOutput event so UI consumers see the last state
            if let Some(ref tx) = event_tx {
                let _ = tx.send(BackendEvent::ShellOutput {
                    session_id,
                    content: output_buf.clone(),
                    finished: true,
                    exit_code: None,
                });
            }

            // Only show the output we got so far (truncated at max)
            truncate_in_place(&mut output_buf, max_output_bytes);
            return Ok(BashExecutionResult {
                output: format!("[exit -1] (cancelled)\n{}", output_buf),
            });
        }

        if start_time.elapsed() > timeout && timeout_ms > 0 {
            kill_process_group(child_pid);
            let _ = process.wait();
            unregister_child(child_pid);

            if let Some(ref tx) = event_tx {
                let _ = tx.send(BackendEvent::ShellOutput {
                    session_id,
                    content: output_buf.clone(),
                    finished: true,
                    exit_code: None,
                });
            }

            truncate_in_place(&mut output_buf, max_output_bytes);
            return Ok(BashExecutionResult {
                output: format!(
                    "[exit -1] (timed out after {}s)\n{}",
                    timeout_ms / 1000,
                    output_buf
                ),
            });
        }

        match chunk_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                raw_bytes.extend_from_slice(&chunk);
                // Decode output from raw bytes, tolerating non-UTF-8
                // encoding when UTF-8 decoding fails.
                output_buf = decode_command_output(&raw_bytes);

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
                        // Re-decode after truncation
                        output_buf = decode_command_output(&raw_bytes);
                    }
                }

                // Send streaming event
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
        output: format!("[exit {status_code}]\n{}", combined),
    })
}
