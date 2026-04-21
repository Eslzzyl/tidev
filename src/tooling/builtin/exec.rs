use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    io::Read,
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use super::utils::truncate_in_place;
use crate::tooling::tools::BashArgs;
use crate::tooling::{ToolDefinition, ToolPermission};

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
    call: &crate::session::ToolCall,
    max_output_bytes: usize,
    rtk_enabled: bool,
) -> Result<BashExecutionResult> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;
    let args = serde_json::from_value::<BashArgs>(arguments)
        .with_context(|| format!("failed to decode arguments for tool '{}'", call.name))?;
    let timeout = args.timeout.unwrap_or(120_000) as u64; // default 2 minutes
    run_shell_inner(
        workspace_root,
        &args.command,
        max_output_bytes,
        rtk_enabled,
        None,
        timeout,
    )
}

pub fn execute_tool_call_with_cancel(
    workspace_root: &Path,
    call: &crate::session::ToolCall,
    max_output_bytes: usize,
    rtk_enabled: bool,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
) -> Result<BashExecutionResult> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;
    let args = serde_json::from_value::<BashArgs>(arguments)
        .with_context(|| format!("failed to decode arguments for tool '{}'", call.name))?;
    let timeout = args.timeout.unwrap_or(120_000) as u64; // default 2 minutes
    run_shell_inner(
        workspace_root,
        &args.command,
        max_output_bytes,
        rtk_enabled,
        Some(cancelled),
        timeout,
    )
}

fn run_shell_inner(
    workspace_root: &Path,
    command: &str,
    max_output_bytes: usize,
    rtk_enabled: bool,
    cancelled: Option<Arc<AtomicBool>>,
    timeout_ms: u64,
) -> Result<BashExecutionResult> {
    // Try to get RTK rewritten command if RTK is enabled
    let (actual_command, rtk_rewritten) = if rtk_enabled {
        let rewritten = rewrite_command(command);
        let was_rewritten = rewritten != command;
        (rewritten, was_rewritten)
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

    let mut stdout = process.stdout.take();
    let mut stderr = process.stderr.take();

    let start_time = std::time::Instant::now();
    let timeout = Duration::from_millis(timeout_ms);

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

        if let Some(status) = process
            .try_wait()
            .with_context(|| format!("failed while waiting for command '{command}' to finish"))?
        {
            let mut combined = String::new();

            if let Some(mut handle) = stdout.take() {
                let _ = handle.read_to_string(&mut combined);
            }

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

            let status = status.code().unwrap_or_default();
            return Ok(BashExecutionResult {
                output: format!("[exit {status}]\n{combined}"),
                rtk_rewritten,
            });
        }

        thread::sleep(Duration::from_millis(50));
    }
}

/// Try to rewrite a command using RTK's rewrite feature.
/// Returns the RTK rewritten command if available, otherwise the original command.
///
/// RTK rewrite works by:
/// - Running `rtk rewrite <command>` to check if RTK has a filter for this command
/// - If RTK returns a rewritten command (exit 0), use it
/// - If RTK returns exit 1 (no equivalent), use the original command
fn rewrite_command(command: &str) -> String {
    let output = std::process::Command::new("rtk")
        .arg("rewrite")
        .arg(command)
        .output()
        .ok();

    match output {
        Some(output) if output.status.success() => {
            let rewritten = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !rewritten.is_empty() && rewritten != command {
                return rewritten;
            }
        }
        _ => {}
    }

    command.to_string()
}
