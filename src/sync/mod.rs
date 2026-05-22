pub mod transport;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::SessionStore;
use transport::Transport;

/// Sync-specific errors.
#[derive(Debug)]
pub enum SyncError {
    NotConnected,
    RemoteNotFound(String),
    SessionNotFound(Uuid),
    TransferFailed(String),
}

/// Machine reachable via SSH.
///
/// `host` can be either an SSH Host alias (from ~/.ssh/config) or
/// a user@host string.  System `ssh` resolves all connection details
/// from `~/.ssh/config` automatically.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteMachine {
    /// Human-friendly name (e.g. "devbox", "laptop").
    pub name: String,
    /// SSH Host alias or user@host (e.g. "devbox" or "eslzzyl@192.168.1.100").
    pub host: String,
    /// Override the tidev binary path on the remote (e.g. "/usr/local/bin/tidev").
    #[serde(default)]
    pub tidev_path: Option<String>,
    /// Timestamp (RFC 3339) of the last successful sync.
    #[serde(default)]
    pub last_sync_at: Option<String>,
}

/// Sync configuration, serialized inside `AppConfig`.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SyncConfig {
    #[serde(default)]
    pub remotes: Vec<RemoteMachine>,
}

impl RemoteMachine {
    pub fn display_name(&self) -> String {
        format!("{} ({})", self.name, self.host)
    }

    pub fn create_transport(&self) -> Box<dyn Transport> {
        Box::new(transport::ssh::SshTransport {
            host: self.host.clone(),
        })
    }

    /// Test the SSH connection by running `tidev version` on the remote.
    pub fn test_connection(&self) -> Result<String> {
        let transport = self.create_transport();
        transport.exec("tidev version --version")
    }
}

/// Summary of a sync push/pull operation.
#[derive(Debug)]
pub struct SyncSummary {
    pub sessions_count: usize,
    pub remote_name: String,
    pub total_bytes: u64,
}

/// High-level sync manager.
pub struct SyncManager {
    pub config: SyncConfig,
    pub store: SessionStore,
}

impl SyncManager {
    pub fn new(config: SyncConfig, store: SessionStore) -> Self {
        Self { config, store }
    }

    /// List all configured remotes.
    pub fn list_remotes(&self) -> &[RemoteMachine] {
        &self.config.remotes
    }

    /// Find a remote by name.
    pub fn find_remote(&self, name: &str) -> Option<&RemoteMachine> {
        self.config.remotes.iter().find(|r| r.name == name)
    }

    /// Find a mutable remote by name.
    pub fn find_remote_mut(&mut self, name: &str) -> Option<&mut RemoteMachine> {
        self.config.remotes.iter_mut().find(|r| r.name == name)
    }

    /// Add or update a remote.
    pub fn add_remote(&mut self, remote: RemoteMachine) {
        if let Some(existing) = self.find_remote_mut(&remote.name) {
            *existing = remote;
        } else {
            self.config.remotes.push(remote);
        }
    }

    /// Remove a remote by name.
    pub fn remove_remote(&mut self, name: &str) -> bool {
        let len = self.config.remotes.len();
        self.config.remotes.retain(|r| r.name != name);
        self.config.remotes.len() < len
    }

    /// Push session(s) to a remote machine.
    ///
    /// 1. Export sessions locally to a temp SQLite file (compressed)
    /// 2. Transfer the file to the remote via rsync/scp
    /// 3. Run `tidev import` on the remote
    pub fn push(
        &self,
        session_ids: &[Uuid],
        remote_name: &str,
        replace: bool,
    ) -> Result<SyncSummary> {
        let remote = self
            .find_remote(remote_name)
            .with_context(|| format!("remote '{remote_name}' not found"))?;
        let transport = remote.create_transport();

        let temp_dir = sync_temp_dir()?;
        let export_path = temp_dir.join("tidev-sync.sqlite");

        // Step 1: Export locally
        self.store
            .export_to_sqlite(session_ids, &export_path, true)
            .context("failed to export sessions")?;

        let file_size = std::fs::metadata(&export_path)
            .map(|m| m.len())
            .unwrap_or(0);

        // Step 2: Transfer file to remote
        let remote_tmp = Path::new("/tmp/tidev-sync.sqlite");
        transport
            .push_file(&export_path, remote_tmp)
            .context("failed to transfer file to remote")?;

        // Step 3: Import on remote
        let import_cmd = if replace {
            format!("tidev import {} --replace", remote_tmp.display())
        } else {
            format!("tidev import {}", remote_tmp.display())
        };
        let remote_cmd = remote_tidev_command(remote, &import_cmd);
        transport
            .exec(&remote_cmd)
            .context("remote import failed")?;

        Ok(SyncSummary {
            sessions_count: session_ids.len(),
            remote_name: remote_name.to_string(),
            total_bytes: file_size,
        })
    }

    /// Pull session(s) from a remote machine.
    ///
    /// 1. Run `tidev export --compress` on the remote to produce a temp file
    /// 2. Transfer the file back via rsync/scp
    /// 3. Import sessions locally
    pub fn pull(
        &self,
        session_filter: &[String],
        remote_name: &str,
        replace: bool,
    ) -> Result<SyncSummary> {
        let remote = self
            .find_remote(remote_name)
            .with_context(|| format!("remote '{remote_name}' not found"))?;
        let transport = remote.create_transport();

        // Step 1: Export on remote
        let remote_tmp = Path::new("/tmp/tidev-sync.sqlite");

        if session_filter.is_empty() {
            let export_cmd =
                remote_tidev_command(remote, &format!("tidev export --all -c -o {}", remote_tmp.display()));
            transport
                .exec(&export_cmd)
                .context("remote export failed")?;
        } else {
            let ids = session_filter.join(" --session ");
            let export_cmd = remote_tidev_command(
                remote,
                &format!("tidev export --session {} -c -o {}", ids, remote_tmp.display()),
            );
            transport
                .exec(&export_cmd)
                .context("remote export failed")?;
        }

        // Step 2: Transfer file back
        let temp_dir = sync_temp_dir()?;
        let import_path = temp_dir.join("tidev-sync.sqlite");
        transport
            .pull_file(remote_tmp, &import_path)
            .context("failed to transfer file from remote")?;

        let file_size = std::fs::metadata(&import_path)
            .map(|m| m.len())
            .unwrap_or(0);

        // Step 3: Import locally
        let count = self
            .store
            .import_from_sqlite(&import_path, session_filter, replace)
            .context("local import failed")?;

        Ok(SyncSummary {
            sessions_count: count,
            remote_name: remote_name.to_string(),
            total_bytes: file_size,
        })
    }
}

/// Create a temporary directory for sync operations.
fn sync_temp_dir() -> Result<PathBuf> {
    let base = std::env::temp_dir().join("tidev-sync");
    std::fs::create_dir_all(&base).context("failed to create sync temp base")?;
    let dir = base.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir).context("failed to create sync temp dir")?;
    Ok(dir)
}

/// Build the remote command string, using remote.tidev_path if set.
fn remote_tidev_command(remote: &RemoteMachine, command: &str) -> String {
    match &remote.tidev_path {
        Some(path) => {
            // Replace "tidev" prefix with the configured path
            if let Some(rest) = command.strip_prefix("tidev ") {
                format!("{} {}", path, rest)
            } else {
                command.to_string()
            }
        }
        None => command.to_string(),
    }
}
