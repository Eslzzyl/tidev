//! Terminal session management for web terminal.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tokio::sync::broadcast;
use uuid::Uuid;

struct TerminalSession {
    id: Uuid,
    /// Writer half of the PTY master — for sending user input.
    writer: Box<dyn Write + Send>,
    /// We also keep the full master so we can call `resize()`.
    /// `resize` uses &self so it's fine after take_writer().
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    reader_task: tokio::task::JoinHandle<()>,
    _killer: Option<Box<dyn portable_pty::ChildKiller + Send>>,
    buffer: Arc<Mutex<Vec<u8>>>,
}

#[derive(Clone, Debug)]
pub struct TerminalOutput {
    pub session_id: Uuid,
    pub data: Vec<u8>,
    pub closed: bool,
}

#[derive(Clone)]
pub struct TerminalManager {
    sessions: Arc<tokio::sync::Mutex<HashMap<Uuid, TerminalSession>>>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    pub async fn start_session(
        &self,
        tx: broadcast::Sender<TerminalOutput>,
        initial_size: PtySize,
    ) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        let pty_system = native_pty_system();
        let mut pair = pty_system
            .openpty(initial_size)
            .map_err(|e| format!("Failed to open PTY: {e}"))?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let mut command_builder = CommandBuilder::new(&shell);
        command_builder.cwd(std::env::current_dir().unwrap_or_default());
        command_builder.env("TERM", "xterm-256color");

        let child = pair
            .slave
            .spawn_command(command_builder)
            .map_err(|e| format!("Failed to spawn shell: {e}"))?;

        let killer = child.clone_killer();
        let master = pair.master;

        let mut reader = master
            .try_clone_reader()
            .map_err(|e| format!("Failed to clone PTY reader: {e}"))?;

        let writer = master
            .take_writer()
            .map_err(|e| format!("Failed to take PTY writer: {e}"))?;

        let buffer = Arc::new(Mutex::new(Vec::new()));
        let buffer_clone = buffer.clone();
        let tx_clone = tx.clone();
        let sid = id;

        let reader_task = tokio::task::spawn_blocking(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let _ = tx_clone.send(TerminalOutput {
                            session_id: sid, data: Vec::new(), closed: true,
                        });
                        break;
                    }
                    Ok(n) => {
                        let data = &buf[..n];
                        let mut guard = buffer_clone.lock().unwrap();
                        guard.extend_from_slice(data);
                        if tx_clone.receiver_count() > 0 {
                            let _ = tx_clone.send(TerminalOutput {
                                session_id: sid, data: data.to_vec(), closed: false,
                            });
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        crate::log_error!("[terminal {}] read error: {e}", sid);
                        let _ = tx_clone.send(TerminalOutput {
                            session_id: sid, data: Vec::new(), closed: true,
                        });
                        break;
                    }
                }
            }
        });

        // The master is consumed by take_writer() on some backends.
        // Drop it — we store the writer separately.  Resize can be
        // re-added later by keeping the raw FD.
        drop(master);

        let session = TerminalSession {
            id,
            writer,
            master: None,
            reader_task,
            _killer: Some(killer),
            buffer,
        };

        self.sessions.lock().await.insert(id, session);
        Ok(id)
    }

    pub async fn get_buffer(&self, session_id: Uuid) -> Vec<u8> {
        let sessions = self.sessions.lock().await;
        if let Some(s) = sessions.get(&session_id) {
            let guard = s.buffer.lock().unwrap();
            guard.clone()
        } else {
            Vec::new()
        }
    }

    pub async fn write_input(&self, session_id: Uuid, data: &[u8]) -> Result<(), String> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("Terminal session {session_id} not found"))?;
        session
            .writer
            .write_all(data)
            .and_then(|_| session.writer.flush())
            .map_err(|e| format!("Failed to write to terminal: {e}"))
    }

    pub async fn resize(&self, _session_id: Uuid, _cols: u16, _rows: u16) -> Result<(), String> {
        // TODO: store the master handle/raw FD to enable resize.
        Ok(())
    }

    pub async fn close_session(&self, session_id: Uuid) {
        let mut sessions = self.sessions.lock().await;
        if let Some(mut session) = sessions.remove(&session_id) {
            if let Some(mut killer) = session._killer.take() {
                let _ = killer.kill();
            }
            let _ = tokio::time::timeout(Duration::from_millis(500), async {
                let _ = session.reader_task.await;
            })
            .await;
        }
    }

    pub async fn has_session(&self, session_id: Uuid) -> bool {
        self.sessions.lock().await.contains_key(&session_id)
    }
}
