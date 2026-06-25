pub mod ssh;

use std::path::Path;

/// Abstract transport for file transfer and remote command execution.
pub trait Transport: Send + Sync {
    /// Push a local file to the remote machine.
    fn push_file(&self, local_path: &Path, remote_path: &Path) -> anyhow::Result<()>;

    /// Pull a remote file to the local machine.
    fn pull_file(&self, remote_path: &Path, local_path: &Path) -> anyhow::Result<()>;

    /// Execute a command on the remote machine and return stdout.
    fn exec(&self, command: &str) -> anyhow::Result<String>;
}
