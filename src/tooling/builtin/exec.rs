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
};

use super::utils::truncate_in_place;
use crate::tooling::tools::BashArgs;
use crate::tooling::{ToolDefinition, ToolPermission};

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
) -> Result<String> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;
    let args = serde_json::from_value::<BashArgs>(arguments)
        .with_context(|| format!("failed to decode arguments for tool '{}'", call.name))?;
    run_shell(workspace_root, &args.command, max_output_bytes, rtk_enabled)
}

pub fn execute_tool_call_with_cancel(
    workspace_root: &Path,
    call: &crate::session::ToolCall,
    max_output_bytes: usize,
    rtk_enabled: bool,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
) -> Result<String> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;
    let args = serde_json::from_value::<BashArgs>(arguments)
        .with_context(|| format!("failed to decode arguments for tool '{}'", call.name))?;
    run_shell_with_cancel(workspace_root, &args.command, max_output_bytes, rtk_enabled, cancelled)
}

fn run_shell(workspace_root: &Path, command: &str, max_output_bytes: usize, rtk_enabled: bool) -> Result<String> {
    run_shell_inner(workspace_root, command, max_output_bytes, rtk_enabled, None)
}

fn run_shell_with_cancel(
    workspace_root: &Path,
    command: &str,
    max_output_bytes: usize,
    rtk_enabled: bool,
    cancelled: Arc<AtomicBool>,
) -> Result<String> {
    run_shell_inner(workspace_root, command, max_output_bytes, rtk_enabled, Some(cancelled))
}

fn run_shell_inner(
    workspace_root: &Path,
    command: &str,
    max_output_bytes: usize,
    rtk_enabled: bool,
    cancelled: Option<Arc<AtomicBool>>,
) -> Result<String> {
    // Wrap command with rtk if enabled
    let actual_command = if rtk_enabled {
        format!("rtk run {}", command)
    } else {
        command.to_string()
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

    loop {
        if cancelled
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            let _ = process.kill();
            let _ = process.wait();
            return Err(anyhow::anyhow!("shell command cancelled"));
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
            return Ok(format!("[exit {status}]\n{combined}"));
        }

        thread::sleep(std::time::Duration::from_millis(50));
    }
}
