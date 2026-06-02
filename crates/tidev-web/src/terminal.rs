//! Terminal session management for web terminal.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tokio::sync::broadcast;
use uuid::Uuid;

/// Maximum buffer size per terminal session (1 MB).
/// When exceeded, oldest data is trimmed on each new write.
const MAX_BUFFER_SIZE: usize = 1 * 1024 * 1024;

struct TerminalSession {
    /// Writer half of the PTY master — for sending user input.
    writer: Option<Box<dyn Write + Send>>,
    /// Master handle — kept for resize().
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    reader_task: tokio::task::JoinHandle<()>,
    /// Child process handle — used for kill() + wait() in close_session().
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
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

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
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
        shell: Option<String>,
    ) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(initial_size)
            .map_err(|e| format!("Failed to open PTY: {e}"))?;

        let shell = shell
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("SHELL").ok())
            .or_else(|| std::env::var("ComSpec").ok())
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "powershell.exe".to_string()
                } else {
                    "/bin/bash".to_string()
                }
            });
        let mut command_builder = CommandBuilder::new(&shell);
        command_builder.cwd(std::env::current_dir().unwrap_or_default());
        command_builder.env("TERM", "xterm-256color");

        let child = pair
            .slave
            .spawn_command(command_builder)
            .map_err(|e| format!("Failed to spawn shell: {e}"))?;

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
                            session_id: sid,
                            data: Vec::new(),
                            closed: true,
                        });
                        break;
                    }
                    Ok(n) => {
                        let data = &buf[..n];
                        let mut guard = buffer_clone.lock().unwrap();
                        guard.extend_from_slice(data);
                        // Trim buffer to max size
                        if guard.len() > MAX_BUFFER_SIZE {
                            let excess = guard.len() - MAX_BUFFER_SIZE;
                            guard.drain(..excess);
                        }
                        if tx_clone.receiver_count() > 0 {
                            let _ = tx_clone.send(TerminalOutput {
                                session_id: sid,
                                data: data.to_vec(),
                                closed: false,
                            });
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        log::error!("[terminal {}] read error: {e}", sid);
                        let _ = tx_clone.send(TerminalOutput {
                            session_id: sid,
                            data: Vec::new(),
                            closed: true,
                        });
                        break;
                    }
                }
            }
        });

        let session = TerminalSession {
            writer: Some(writer),
            master: Some(master),
            reader_task,
            child: Some(child),
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
        if let Some(ref mut writer) = session.writer {
            writer
                .write_all(data)
                .and_then(|_| writer.flush())
                .map_err(|e| format!("Failed to write to terminal: {e}"))
        } else {
            Err("Session writer already taken (closed)".to_string())
        }
    }

    pub async fn resize(&self, session_id: Uuid, cols: u16, rows: u16) -> Result<(), String> {
        let sessions = self.sessions.lock().await;
        let session = sessions
            .get(&session_id)
            .ok_or_else(|| format!("Terminal session {session_id} not found"))?;
        if let Some(ref master) = session.master {
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| format!("Failed to resize PTY: {e}"))
        } else {
            Err("No master handle available for resize".to_string())
        }
    }

    /// Close a single terminal session (kill child, wait for reader, clean up).
    pub async fn close_session(&self, session_id: Uuid) {
        let mut sessions = self.sessions.lock().await;
        Self::close_session_inner(&mut sessions, session_id).await;
    }

    /// Shut down ALL terminal sessions. Called during server graceful shutdown.
    pub async fn shutdown(&self) {
        let mut sessions = self.sessions.lock().await;
        let ids: Vec<Uuid> = sessions.keys().copied().collect();
        for id in ids {
            log::info!("Shutting down terminal session {id}");
            Self::close_session_inner(&mut sessions, id).await;
        }
        log::info!("All terminal sessions shut down");
    }

    /// Internal helper: close one session. Lock is held by the caller.
    async fn close_session_inner(sessions: &mut HashMap<Uuid, TerminalSession>, session_id: Uuid) {
        if let Some(mut session) = sessions.remove(&session_id) {
            // 1. Drop the writer first. This sends EOT to the slave,
            //    prompting the shell to exit gracefully.
            drop(session.writer.take());

            // 2. Kill the child process. portable_pty::Child::kill()
            //    first sends SIGHUP, polls try_wait() up to ~250ms,
            //    then falls back to SIGKILL.
            if let Some(mut child) = session.child.take() {
                let _ = child.kill();
                // 3. Wait for the child to fully exit (up to 5s).
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) => {
                            if tokio::time::Instant::now() >= deadline {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                        Err(_) => break,
                    }
                }
            }

            // 4. The reader should now get EOF since the slave PTY is
            //    closed. Wait for the blocking task to finish (up to 3s).
            let _ = tokio::time::timeout(Duration::from_secs(3), async {
                let _ = session.reader_task.await;
            })
            .await;
        }
    }

    pub async fn has_session(&self, session_id: Uuid) -> bool {
        self.sessions.lock().await.contains_key(&session_id)
    }

    /// Returns the list of all active session IDs.
    pub async fn list_sessions(&self) -> Vec<Uuid> {
        self.sessions.lock().await.keys().copied().collect()
    }
}
