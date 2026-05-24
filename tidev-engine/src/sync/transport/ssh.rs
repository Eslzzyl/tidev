use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use super::Transport;

/// SSH-based transport using system `ssh`, `rsync`, and `scp` binaries.
///
/// `host` is passed directly to system `ssh`/`scp`, so it can be either
/// an SSH Host alias (from `~/.ssh/config`) or a `user@host` string.
pub struct SshTransport {
    pub host: String,
}

impl SshTransport {
    fn rsync_available() -> bool {
        Command::new("sh")
            .args(["-c", "command -v rsync"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn transfer_file(&self, local: &Path, remote: &Path, pull: bool) -> Result<()> {
        if Self::rsync_available() {
            self.rsync_transfer(local, remote, pull)
        } else {
            if pull {
                self.scp_transfer(remote, local)
            } else {
                self.scp_transfer(local, remote)
            }
        }
    }

    fn rsync_transfer(&self, local: &Path, remote: &Path, pull: bool) -> Result<()> {
        let mut cmd = Command::new("rsync");
        cmd.args(["-az", "--partial-dir=.rsync-partial", "-e", "ssh"]);

        if pull {
            cmd.arg(format!("{}:{}", self.host, remote.display()));
            cmd.arg(local);
        } else {
            cmd.arg(local);
            cmd.arg(format!("{}:{}", self.host, remote.display()));
        }

        let status = cmd.status().context("failed to run rsync")?;
        if !status.success() {
            anyhow::bail!("rsync exited with status: {}", status);
        }
        Ok(())
    }

    fn scp_transfer(&self, source: &Path, dest: &Path) -> Result<()> {
        let mut cmd = Command::new("scp");
        cmd.arg(source);
        cmd.arg(dest);

        let status = cmd.status().context("failed to run scp")?;
        if !status.success() {
            anyhow::bail!("scp exited with status: {}", status);
        }
        Ok(())
    }
}

impl Transport for SshTransport {
    fn push_file(&self, local_path: &Path, remote_path: &Path) -> Result<()> {
        self.transfer_file(local_path, remote_path, false)
    }

    fn pull_file(&self, remote_path: &Path, local_path: &Path) -> Result<()> {
        self.transfer_file(local_path, remote_path, true)
    }

    fn exec(&self, command: &str) -> Result<String> {
        let output = Command::new("ssh")
            .arg(&self.host)
            .arg("--")
            .arg(command)
            .output()
            .with_context(|| format!("failed to execute remote command: {command}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("remote command failed: {}\n{}", command, stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}
