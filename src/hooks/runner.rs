use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

/// The outcome of running a single hook command.
#[derive(Clone, Debug)]
pub struct HookCommandOutput {
    /// Whether the hook command exited successfully.
    pub success: bool,
    /// Combined stdout (on success) or error message (on failure).
    pub output: String,
}

/// Run a shell command with a timeout.
///
/// The command is executed via `sh -c <command>` so that pipes,
/// redirects, and compound statements work as expected.
///
/// Returns `HookCommandOutput` — this function never panics.
pub async fn run_hook_command(
    command: &str,
    cwd: &Path,
    timeout_sec: u64,
) -> HookCommandOutput {
    let child = match Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return HookCommandOutput {
                success: false,
                output: format!("Failed to spawn hook command '{command}': {e}"),
            };
        }
    };

    let result = tokio::time::timeout(Duration::from_secs(timeout_sec), child.wait_with_output())
        .await;

    match result {
        Ok(Ok(output)) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                HookCommandOutput {
                    success: true,
                    output: stdout,
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                HookCommandOutput {
                    success: false,
                    output: format!(
                        "Hook '{command}' exited with {}:\n{}",
                        output.status.code().unwrap_or(-1),
                        stderr.trim()
                    ),
                }
            }
        }
        Ok(Err(e)) => HookCommandOutput {
            success: false,
            output: format!("Hook '{command}' error: {e}"),
        },
        Err(_) => HookCommandOutput {
            success: false,
            output: format!("Hook '{command}' timed out after {timeout_sec}s"),
        },
    }
}
