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
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use tokio::sync::mpsc::UnboundedSender;

use super::utils::truncate_in_place;
use crate::sandbox::{
    CommandSpec, SandboxManager, SandboxPolicy, pre_exec_hardening,
    remove_dangerous_env_vars_parent,
};
use crate::tooling::tools::{BashArgs, decode_tool_args};
use crate::tooling::{ToolDefinition, ToolPermission};
use tidev_session::session::{BackendEvent, tool_output_preview};
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

/// Kill all tracked child processes (no-op on non-Unix).
#[cfg(not(unix))]
pub fn kill_all_children() {
    // Windows support could be added later using TerminateProcess
}

/// Result of bash tool execution, including sandbox and RTK metadata.
#[derive(Debug)]
pub struct BashExecutionResult {
    pub output: String,
    pub rtk_rewritten: bool,
    /// Whether the command was executed inside a sandbox.
    pub sandboxed: bool,
    /// The type of sandbox used, if any.
    pub sandbox_type: String,
    /// Whether the command appeared to be denied by the sandbox.
    pub sandbox_denied: bool,
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
    rtk_enabled: bool,
    sandbox_policy: Option<SandboxPolicy>,
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
        sandbox_policy,
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
    rtk_enabled: bool,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    sandbox_policy: Option<SandboxPolicy>,
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
        sandbox_policy,
        event_tx,
        session_id,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_shell_inner(
    workspace_root: &Path,
    command: &str,
    max_output_bytes: usize,
    rtk_enabled: bool,
    cancelled: Option<Arc<AtomicBool>>,
    timeout_ms: u64,
    sandbox_policy: Option<SandboxPolicy>,
    event_tx: Option<UnboundedSender<BackendEvent>>,
    session_id: Uuid,
) -> Result<BashExecutionResult> {
    // Try to get RTK rewritten command if RTK is enabled
    let (mut actual_command, rtk_rewritten) = if rtk_enabled {
        let result = rewrite_command(command);
        (result.command, result.rewritten)
    } else {
        (command.to_string(), false)
    };

    // Prepare sandbox if a policy is provided
    let sandbox_policy = sandbox_policy.unwrap_or(SandboxPolicy::DangerFullAccess);
    let use_sandbox = !cfg!(target_os = "windows")
        && !matches!(sandbox_policy, SandboxPolicy::DangerFullAccess)
        && !matches!(sandbox_policy, SandboxPolicy::ExternalSandbox);

    // ── Layer 1: Privilege escalation handling (sudo/doas/pkexec) ──────
    // When sudo is detected in a non-sandboxed command, we:
    // 1. Wrap the command so `sudo` becomes `sudo -A`, which uses
    //    SUDO_ASKPASS instead of writing to /dev/tty directly.
    // 2. Create a temporary askpass script that fails with a friendly
    //    error message (no password prompt reaches the terminal).
    let mut sudo_guard: Option<super::sudo::AskpassGuard> = None;
    let _sudo_active = if !use_sandbox && super::sudo::has_privilege_escalation(&actual_command) {
        let guard = super::sudo::create_askpass_script()?;
        let wrapped = super::sudo::wrap_command(&actual_command, guard.path());
        log::info!("sudo: privilege escalation detected, wrapping command with SUDO_ASKPASS");
        sudo_guard = Some(guard);
        actual_command = wrapped;
        true
    } else {
        false
    };

    let mut process = if use_sandbox {
        let spec = CommandSpec::shell(
            &actual_command,
            workspace_root.to_path_buf(),
            Duration::from_millis(timeout_ms),
        )
        .with_policy(sandbox_policy.clone());

        let manager = SandboxManager::new();
        let exec_env = manager.prepare(&spec);

        let mut cmd = std::process::Command::new(exec_env.program());
        cmd.args(exec_env.args())
            .current_dir(&exec_env.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Apply environment variables from the sandbox spec on top of
        // inherited environment.  We do NOT call env_clear() here —
        // remove_dangerous_env_vars_parent() handles stripping dangerous
        // vars (LD_PRELOAD, DYLD_*, …) in the parent before spawn.
        if !exec_env.env.is_empty() {
            cmd.envs(&exec_env.env);
        }

        // Remove dangerous environment variables in the parent process
        // (safe, before fork).  The child inherits the cleaned environment.
        // MUST NOT be done in pre_exec — anything that allocates memory
        // can deadlock after fork() in a multi-threaded process.
        remove_dangerous_env_vars_parent();

        // Determine if Landlock should be applied in the child process
        // (Landlock requires in-process syscalls before exec).
        #[cfg(target_os = "linux")]
        let use_landlock = exec_env.sandbox_type == crate::sandbox::SandboxType::LinuxLandlock;

        // Apply process hardening and Landlock (on Linux) in pre_exec
        #[cfg(unix)]
        if exec_env.is_sandboxed() {
            unsafe {
                cmd.pre_exec(move || {
                    // For Landlock, apply filesystem restrictions before exec
                    #[cfg(target_os = "linux")]
                    if use_landlock {
                        let cwd = std::path::Path::new(".");
                        if let Err(e) =
                            crate::sandbox::landlock::apply_landlock_policy(&sandbox_policy, cwd)
                        {
                            // If Landlock fails, abort the child process
                            let _ = std::io::Write::write(
                                &mut std::io::stderr(),
                                format!("Landlock error: {e}\n").as_bytes(),
                            );
                            std::process::abort();
                        }
                    }

                    // Apply general process hardening
                    pre_exec_hardening()
                });
            }
        }

        cmd.spawn()
            .with_context(|| format!("failed to run sandboxed command '{actual_command}'"))?
    } else {
        // No sandbox: direct execution (original behavior)
        if cfg!(target_os = "windows") {
            let shell = crate::shell::get();
            let mut cmd = std::process::Command::new(&shell.program);
            // arg may contain spaces (e.g. "-NoProfile -Command")
            let mut all_args: Vec<&str> = shell.arg.split_whitespace().collect();
            all_args.push(&actual_command);
            cmd.args(&all_args)
                .current_dir(workspace_root)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

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
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            // ── Layer 1: Inject SUDO_ASKPASS environment variable ──
            // This tells sudo -A where to find the askpass helper.
            if let Some(ref guard) = sudo_guard {
                cmd.env("SUDO_ASKPASS", guard.path());
            }

            // ── Layer 2: Disconnect from controlling terminal ──
            // When sudo is active, create a new session (setsid) so the
            // child process has no controlling terminal. This means
            // open("/dev/tty") will fail with ENXIO, providing defense
            // in depth against terminal corruption even if sudo somehow
            // bypasses the ASKPASS mechanism.
            //
            // This is done in pre_exec (after fork, before exec) so it
            // only affects the child process.
            #[cfg(target_os = "macos")]
            if _sudo_active {
                unsafe {
                    cmd.pre_exec(move || {
                        // Create a new session, detaching from controlling terminal
                        libc::setsid();
                        Ok(())
                    });
                }
            }

            cmd.spawn()
                .with_context(|| format!("failed to run command '{actual_command}'"))?
        }
    };

    // Register child PID so it can be killed on program exit if needed.
    let child_pid = process.id();
    register_child(child_pid);

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
            unregister_child(child_pid);
            return Err(anyhow::anyhow!("shell command cancelled"));
        }

        // Check timeout
        if start_time.elapsed() > timeout {
            let _ = process.kill();
            let _ = process.wait();
            unregister_child(child_pid);
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
    unregister_child(child_pid);
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

    // Determine sandbox type for result metadata
    let sandbox_type = if use_sandbox {
        crate::sandbox::get_platform_sandbox()
            .map(|t| t.to_string())
            .unwrap_or_else(|| "none".to_string())
    } else {
        "none".to_string()
    };

    // Detect sandbox denial from exit code and output content
    let sandbox_denied = use_sandbox
        && status_code != 0
        && (combined.contains("Operation not permitted")
            || combined.contains("denied")
            || combined.contains("Sandbox")
            || combined.contains("sandbox")
            || combined.contains("not allowed")
            || combined.contains("permission denied")
            || combined.contains("EPERM")
            // bwrap read-only filesystem denial
            || combined.contains("Read-only file system"));

    Ok(BashExecutionResult {
        output: if sandbox_denied {
            format!(
                "[exit {status_code}] (sandbox blocked this command)\n\n\
                 The command was blocked by the {} sandbox.\n\
                 Open the panel with /sandbox and switch to \"full access\" to retry.\n\n\
                 {}",
                sandbox_type,
                tool_output_preview(Some("bash"), &combined)
            )
        } else {
            format!("[exit {status_code}]\n{}", combined)
        },
        rtk_rewritten,
        sandboxed: use_sandbox,
        sandbox_type,
        sandbox_denied,
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
