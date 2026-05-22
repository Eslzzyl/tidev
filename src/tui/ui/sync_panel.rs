use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use uuid::Uuid;

use crate::config::ConfigPaths;
use crate::storage::SessionRecord;
use crate::sync::{RemoteMachine, SyncManager};

use super::App;

// ── View states ─────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum SyncView {
    /// Main list of configured remotes.
    RemoteList,
    /// Actions available for a selected remote.
    RemoteActions { remote_index: usize },
    /// Multi-select sessions for push/pull.
    SessionPicker {
        remote_index: usize,
        action: SyncAction,
        selected_indices: Vec<usize>,
        cursor: usize,
    },
    /// Two-step add-remote form.
    AddRemote {
        host: String,
        name: String,
        step: AddRemoteStep,
    },
    /// Confirmation dialog.
    Confirm {
        message: String,
        on_confirm: Box<SyncView>,
    },
    /// Operation result.
    Result { message: String, success: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncAction {
    Push,
    Pull,
}

impl SyncAction {
    pub fn label(&self) -> &'static str {
        match self {
            SyncAction::Push => "Push",
            SyncAction::Pull => "Pull",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddRemoteStep {
    Host,
    Name,
}

// ── Main panel state ────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct SyncPanelState {
    pub view: SyncView,
    pub sessions: Vec<SessionRecord>,
    pub selected_index: usize,
}

// ── App methods ─────────────────────────────────────────────────

impl App {
    pub(crate) fn open_sync_panel(&mut self) {
        // Close overlapping panels
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.mcp_panel = None;
        self.agents_panel = None;
        self.settings_panel = None;
        self.memory_panel = None;
        self.message_panel = None;
        self.skills_panel = None;
        self.sandbox_panel = None;
        self.command_palette.clear();

        // Reload config to pick up CLI-side changes
        if let Ok(paths) = ConfigPaths::discover()
            && let Ok(config) = crate::config::AppConfig::load_or_create(&paths)
        {
            self.config = config;
        }

        let sessions = self.store.load_all_sessions().unwrap_or_default();

        self.sync_panel = Some(SyncPanelState {
            view: SyncView::RemoteList,
            sessions,
            selected_index: 0,
        });
    }

    fn close_sync_panel(&mut self) {
        self.sync_panel = None;
        self.composer.clear();
    }

    fn sync_remotes(&self) -> Vec<RemoteMachine> {
        self.config.sync.remotes.clone()
    }

    fn sync_manager(&self) -> SyncManager {
        SyncManager::new(self.config.sync.clone(), self.store.clone())
    }

    pub(crate) fn handle_sync_panel_key(
        &mut self,
        key: KeyEvent,
        runtime: &tokio::runtime::Runtime,
    ) -> Result<()> {
        let Some(panel) = self.sync_panel.clone() else {
            return Ok(());
        };

        let view = panel.view.clone();
        match view {
            SyncView::RemoteList => self.handle_list_key(panel, key),
            SyncView::RemoteActions { remote_index } => {
                self.handle_actions_key(panel, key, runtime, remote_index)
            }
            SyncView::SessionPicker {
                remote_index,
                action,
                selected_indices,
                cursor,
            } => self.handle_session_picker_key(
                panel,
                key,
                remote_index,
                action,
                selected_indices,
                cursor,
            ),
            SyncView::AddRemote { .. } => self.handle_add_remote_key(panel, key),
            SyncView::Confirm { on_confirm, .. } => {
                self.handle_confirm_key(panel, key, runtime, *on_confirm)
            }
            SyncView::Result { .. } => self.handle_result_key(panel, key),
        }
    }

    // ── RemoteList ──────────────────────────────────────────────

    fn handle_list_key(&mut self, mut panel: SyncPanelState, key: KeyEvent) -> Result<()> {
        let remotes = self.sync_remotes();
        match key.code {
            KeyCode::Esc => {
                self.close_sync_panel();
                Ok(())
            }
            KeyCode::Up => {
                if !remotes.is_empty() {
                    panel.selected_index = panel.selected_index.saturating_sub(1);
                    self.sync_panel = Some(panel);
                }
                Ok(())
            }
            KeyCode::Down => {
                let max = remotes.len().saturating_sub(1);
                if panel.selected_index < max {
                    panel.selected_index += 1;
                    self.sync_panel = Some(panel);
                }
                Ok(())
            }
            KeyCode::Enter => {
                if !remotes.is_empty() {
                    let idx = panel.selected_index.min(remotes.len() - 1);
                    panel.view = SyncView::RemoteActions { remote_index: idx };
                    self.sync_panel = Some(panel);
                }
                Ok(())
            }
            KeyCode::Char('a') => {
                panel.view = SyncView::AddRemote {
                    host: String::new(),
                    name: String::new(),
                    step: AddRemoteStep::Host,
                };
                self.composer.clear();
                self.composer.set_placeholder("SSH host alias or user@host");
                self.sync_panel = Some(panel);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    // ── RemoteActions ───────────────────────────────────────────

    fn handle_actions_key(
        &mut self,
        mut panel: SyncPanelState,
        key: KeyEvent,
        _runtime: &tokio::runtime::Runtime,
        remote_index: usize,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                panel.view = SyncView::RemoteList;
                self.sync_panel = Some(panel);
            }
            KeyCode::Char('p') => {
                panel.view = SyncView::SessionPicker {
                    remote_index,
                    action: SyncAction::Push,
                    selected_indices: Vec::new(),
                    cursor: 0,
                };
                self.sync_panel = Some(panel);
            }
            KeyCode::Char('u') => {
                panel.view = SyncView::SessionPicker {
                    remote_index,
                    action: SyncAction::Pull,
                    selected_indices: Vec::new(),
                    cursor: 0,
                };
                self.sync_panel = Some(panel);
            }
            KeyCode::Char('t') => {
                let remotes = self.sync_remotes();
                if let Some(remote) = remotes.get(remote_index) {
                    let result = remote.test_connection();
                    match result {
                        Ok(version) => {
                            let version = version.trim();
                            panel.view = SyncView::Result {
                                message: format!(
                                    "Connection to '{}' successful (remote tidev: {})",
                                    remote.name, version
                                ),
                                success: true,
                            };
                        }
                        Err(e) => {
                            panel.view = SyncView::Result {
                                message: format!("Connection to '{}' failed:\n{}", remote.name, e),
                                success: false,
                            };
                        }
                    }
                    self.sync_panel = Some(panel);
                }
            }
            KeyCode::Char('d') => {
                let remotes = self.sync_remotes();
                if let Some(remote) = remotes.get(remote_index) {
                    panel.view = SyncView::Confirm {
                        message: format!("Remove remote '{}'?", remote.name),
                        on_confirm: Box::new(SyncView::RemoteActions { remote_index }),
                    };
                    self.sync_panel = Some(panel);
                }
            }
            _ => {}
        }
        Ok(())
    }

    // ── SessionPicker ───────────────────────────────────────────

    fn handle_session_picker_key(
        &mut self,
        mut panel: SyncPanelState,
        key: KeyEvent,
        remote_index: usize,
        action: SyncAction,
        selected_indices: Vec<usize>,
        cursor: usize,
    ) -> Result<()> {
        let sessions = panel.sessions.clone();
        let max = sessions.len().saturating_sub(1);
        let remote_name = self
            .sync_remotes()
            .get(remote_index)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| "unknown".to_string());

        match key.code {
            KeyCode::Esc => {
                panel.view = SyncView::RemoteActions { remote_index };
                self.sync_panel = Some(panel);
            }
            KeyCode::Up => {
                let new_cursor = cursor.saturating_sub(1);
                panel.view = SyncView::SessionPicker {
                    remote_index,
                    action,
                    selected_indices,
                    cursor: new_cursor,
                };
                self.sync_panel = Some(panel);
            }
            KeyCode::Down if cursor < max => {
                panel.view = SyncView::SessionPicker {
                    remote_index,
                    action,
                    selected_indices,
                    cursor: cursor + 1,
                };
                self.sync_panel = Some(panel);
            }
            KeyCode::Char(' ') => {
                let mut new_selected = selected_indices.clone();
                if let Some(pos) = new_selected.iter().position(|i| *i == cursor) {
                    new_selected.remove(pos);
                } else {
                    new_selected.push(cursor);
                }
                panel.view = SyncView::SessionPicker {
                    remote_index,
                    action,
                    selected_indices: new_selected,
                    cursor,
                };
                self.sync_panel = Some(panel);
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let all: Vec<usize> = (0..sessions.len()).collect();
                panel.view = SyncView::SessionPicker {
                    remote_index,
                    action,
                    selected_indices: all,
                    cursor,
                };
                self.sync_panel = Some(panel);
            }
            KeyCode::Enter => {
                let count = selected_indices.len();
                let msg = if action == SyncAction::Pull && count == 0 {
                    format!("Pull all sessions from '{}'?", remote_name)
                } else {
                    format!(
                        "{} {} session(s) to/from '{}'?",
                        action.label(),
                        count,
                        remote_name
                    )
                };
                panel.view = SyncView::Confirm {
                    message: msg,
                    on_confirm: Box::new(SyncView::SessionPicker {
                        remote_index,
                        action,
                        selected_indices: selected_indices.clone(),
                        cursor,
                    }),
                };
                self.sync_panel = Some(panel);
            }
            _ => {}
        }
        Ok(())
    }

    // ── AddRemote ───────────────────────────────────────────────

    fn handle_add_remote_key(&mut self, mut panel: SyncPanelState, key: KeyEvent) -> Result<()> {
        let SyncView::AddRemote { host, name, step } = panel.view.clone() else {
            return Ok(());
        };

        match key.code {
            KeyCode::Esc => {
                panel.view = SyncView::RemoteList;
                self.composer.clear();
                self.sync_panel = Some(panel);
            }
            KeyCode::Enter => {
                match step {
                    AddRemoteStep::Host => {
                        let text = self.composer.text().trim().to_string();
                        if text.is_empty() {
                            // Show error by staying on same step; error rendered in dialog
                            panel.view = SyncView::AddRemote {
                                host,
                                name,
                                step: AddRemoteStep::Host,
                            };
                            self.sync_panel = Some(panel);
                            self.last_notice = Some("Host cannot be empty".to_string());
                            return Ok(());
                        }
                        let new_name = if !name.is_empty() { name } else { text.clone() };
                        panel.view = SyncView::AddRemote {
                            host: text.clone(),
                            name: new_name,
                            step: AddRemoteStep::Name,
                        };
                        self.composer.clear();
                        self.composer.set_text(text);
                        self.composer
                            .set_placeholder("Friendly name (or empty to use host)");
                        self.sync_panel = Some(panel);
                    }
                    AddRemoteStep::Name => {
                        let text = self.composer.text().trim().to_string();
                        let final_name = if text.is_empty() { host.clone() } else { text };

                        let remote = RemoteMachine {
                            name: final_name,
                            host: host.clone(),
                            tidev_path: None,
                            last_sync_at: None,
                        };

                        let paths = ConfigPaths::discover()?;
                        let mut config = crate::config::AppConfig::load_or_create(&paths)?;
                        config.sync.remotes.push(remote);
                        config.save(&paths)?;
                        self.config = config;

                        self.composer.clear();
                        panel.view = SyncView::RemoteList;
                        self.sync_panel = Some(panel);
                    }
                }
            }
            KeyCode::Backspace if step == AddRemoteStep::Name => {
                panel.view = SyncView::AddRemote {
                    host: host.clone(),
                    name,
                    step: AddRemoteStep::Host,
                };
                self.composer.clear();
                self.composer.set_text(host.clone());
                self.composer.set_placeholder("SSH host alias or user@host");
                self.sync_panel = Some(panel);
            }
            _ => {}
        }
        Ok(())
    }

    // ── Confirm ─────────────────────────────────────────────────

    fn handle_confirm_key(
        &mut self,
        mut panel: SyncPanelState,
        key: KeyEvent,
        _runtime: &tokio::runtime::Runtime,
        on_confirm: SyncView,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                panel.view = on_confirm;
                self.sync_panel = Some(panel);
            }
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                match on_confirm {
                    SyncView::SessionPicker {
                        remote_index,
                        action,
                        selected_indices,
                        ..
                    } => {
                        self.execute_sync_action(panel, remote_index, action, selected_indices)?;
                    }
                    SyncView::RemoteActions { remote_index } => {
                        // Delete remote
                        let paths = ConfigPaths::discover()?;
                        let mut config = crate::config::AppConfig::load_or_create(&paths)?;
                        let remotes = self.sync_remotes();
                        if let Some(remote) = remotes.get(remote_index) {
                            config.sync.remotes.retain(|r| r.name != remote.name);
                            config.save(&paths)?;
                            self.config = config;
                        }
                        panel.view = SyncView::RemoteList;
                        self.sync_panel = Some(panel);
                    }
                    _ => {
                        panel.view = SyncView::RemoteList;
                        self.sync_panel = Some(panel);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    // ── Result ──────────────────────────────────────────────────

    fn handle_result_key(&mut self, _panel: SyncPanelState, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.close_sync_panel();
                Ok(())
            }
            _ => Ok(()),
        }
    }

    // ── Execute sync action ─────────────────────────────────────

    fn execute_sync_action(
        &mut self,
        mut panel: SyncPanelState,
        remote_index: usize,
        action: SyncAction,
        selected_indices: Vec<usize>,
    ) -> Result<()> {
        let remotes = self.sync_remotes();
        let Some(remote) = remotes.get(remote_index) else {
            panel.view = SyncView::Result {
                message: "Remote not found".to_string(),
                success: false,
            };
            self.sync_panel = Some(panel);
            return Ok(());
        };

        let remote_name = remote.name.clone();
        let manager = self.sync_manager();

        let session_ids: Vec<Uuid> = if action == SyncAction::Pull && selected_indices.is_empty() {
            Vec::new()
        } else {
            selected_indices
                .iter()
                .filter_map(|i| panel.sessions.get(*i).map(|s| s.session_id))
                .collect()
        };

        let result = match action {
            SyncAction::Push => {
                if session_ids.is_empty() {
                    Err(anyhow::anyhow!("No sessions selected"))
                } else {
                    manager.push(&session_ids, &remote_name, false)
                }
            }
            SyncAction::Pull => {
                let filter: Vec<String> = session_ids.iter().map(|id| id.to_string()).collect();
                manager.pull(&filter, &remote_name, false)
            }
        };

        match result {
            Ok(summary) => {
                let paths = ConfigPaths::discover()?;
                let mut config = crate::config::AppConfig::load_or_create(&paths)?;
                if let Some(r) = config
                    .sync
                    .remotes
                    .iter_mut()
                    .find(|r| r.name == remote_name)
                {
                    r.last_sync_at = Some(chrono::Utc::now().to_rfc3339());
                }
                config.save(&paths)?;
                self.config = config;

                panel.sessions = self.store.load_all_sessions().unwrap_or_default();

                panel.view = SyncView::Result {
                    message: format!(
                        "{}: {} session(s) ({})",
                        action.label(),
                        summary.sessions_count,
                        format_size(summary.total_bytes),
                    ),
                    success: true,
                };
            }
            Err(e) => {
                panel.view = SyncView::Result {
                    message: format!("{} failed:\n{}", action.label(), e),
                    success: false,
                };
            }
        }
        self.sync_panel = Some(panel);
        Ok(())
    }
}

// ── Helpers ─────────────────────────────────────────────────────

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
