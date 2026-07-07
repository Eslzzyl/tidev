//! New-architecture App root component.
//!
//! Owns the Runtime, manages the component tree via OverlayStack,
//! routes Actions, and dispatches async commands.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use tidev_types::agent_type::AgentType;
use tidev_types::message::{BackendEvent, MessageRole};
use tidev_tui::theme::{ThemeName, ThemePalette};

use crate::action::{Action, ChatAction, ConnectAction, OverlayAction, OverlayKind, SearchAction,
    SessionAction, ThemeAction};
use crate::component::Component;
use crate::components::overlay_stack::OverlayStack;
use crate::components::overlays::agents::AgentsPanel;
use crate::components::overlays::connect::ConnectDialog;
use crate::components::overlays::fork::ForkConfirmDialog;
use crate::components::overlays::image::ImageViewer;
use crate::components::overlays::message::{MessagePanel, MessagePanelMessage};
use crate::components::overlays::model::ModelPanel;
use crate::components::overlays::rename::RenameDialog;
use crate::components::overlays::search::SearchPanel;
use crate::components::overlays::session::SessionPanel;
use crate::components::overlays::settings::SettingsPanel;
use crate::components::overlays::skills::{SkillItem, SkillsPanel};
use crate::components::overlays::theme::ThemePanel;
use crate::components::overlays::undo::UndoConfirmDialog;
use crate::context::{DrawContext, UpdateContext};
use crate::utils::strip_system_reminder_tags;

pub struct App {
    pub(crate) runtime: tidev_core::Runtime,
    overlays: OverlayStack,
    current_palette: ThemePalette,
    should_quit: bool,
    /// Pending scroll target set by ChatAction::ScrollTo (consumed by Chat component).
    scroll_target: Option<uuid::Uuid>,
    /// Current active session (set by SessionPanel when switching sessions).
    current_session_id: Option<uuid::Uuid>,
    /// Status notice shown at the bottom of the screen (plain text, no timeout).
    last_notice: Option<(String, Instant)>,
    /// Transient popup notification in top-right corner (auto-expires).
    toast: Option<(String, Instant)>,
    /// Receiver for tool permission requests from the agent loop.
    pub(crate) perm_rx: Option<tokio::sync::mpsc::UnboundedReceiver<tidev_core::PendingToolApproval>>,
    /// Receiver for backend events (streaming deltas, tool results, etc.).
    pub(crate) event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<BackendEvent>>,
}

impl App {
    pub fn new(
        runtime: tidev_core::Runtime,
        perm_rx: Option<tokio::sync::mpsc::UnboundedReceiver<tidev_core::PendingToolApproval>>,
        event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<BackendEvent>>,
    ) -> Self {
        let theme_str = runtime.config().theme;
        let current_palette = ThemePalette::from_name(&theme_str);
        Self {
            runtime,
            overlays: OverlayStack::new(),
            current_palette,
            should_quit: false,
            scroll_target: None,
            current_session_id: None,
            last_notice: None,
            toast: None,
            perm_rx,
            event_rx,
        }
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    // ── Notifications ──

    /// Set a persistent status notice shown at the bottom of the screen.
    pub(crate) fn set_notice(&mut self, msg: impl Into<String>) {
        self.last_notice = Some((msg.into(), Instant::now()));
    }

    /// Set a transient toast notification (auto-expires after `duration`).
    pub(crate) fn set_toast(&mut self, msg: impl Into<String>, duration: std::time::Duration) {
        self.toast = Some((msg.into(), Instant::now() + duration));
    }

    // ── Event handling ──

    /// Handle a backend event from the agent loop (streaming, tool results, etc.).
    pub(crate) fn handle_backend_event(&mut self, event: BackendEvent) {
        match event {
            BackendEvent::Delta { .. } => {
                // TODO: Phase 6 — forward to message rendering
            }
            BackendEvent::ReasoningDelta { .. } => {
                // TODO: Phase 6 — forward to message rendering
            }
            BackendEvent::ToolCallUpdated { .. } => {
                // TODO: Phase 6 — update tool call display
            }
            BackendEvent::Finished { .. } => {
                // TODO: Phase 6 — message completed
            }
            BackendEvent::ToolCompleted { .. } => {
                // TODO: Phase 6 — tool result received
            }
            BackendEvent::ShellOutput { .. } => {
                // TODO: Phase 6 — bash output streaming
            }
            BackendEvent::SubagentStatus { .. }
            | BackendEvent::SubagentCompleted { .. } => {
                // TODO: Phase 6 — subagent progress
            }
            BackendEvent::UsageStats { .. } => {
                // TODO: show usage in status bar
            }
            BackendEvent::TurnStarting { .. } => {
                // TODO: Phase 6 — new turn starting
            }
            BackendEvent::StreamEnd { .. } => {
                // TODO: Phase 6 — streaming finished
            }
            _ => {
                log::debug!("Unhandled backend event: {event:?}");
            }
        }
    }

    /// Handle a pending tool approval request from the agent loop.
    pub(crate) fn handle_pending_approval(
        &mut self,
        approval: tidev_core::PendingToolApproval,
    ) {
        // TODO: Phase 5c/5d — show permission/security dialogs, collect decisions
        log::debug!(
            "PendingToolApproval: {} tool call(s), mode={:?}",
            approval.tool_calls.len(),
            approval.mode,
        );
    }

    pub fn handle_key_event(&mut self, key: KeyEvent) {
        // 1. Global shortcuts (unaffected by overlays)
        if let Some(action) = self.handle_global_key(key) {
            self.process_action(action);
            return;
        }

        // 2. OverlayStack top-first
        if let Some(action) = self.overlays.handle_key_event(key) {
            self.process_action(action);
        }
    }

    pub fn handle_mouse_event(&mut self, _mouse: MouseEvent) {
        // TODO: route to overlays
    }

    pub fn handle_resize(&mut self, _w: u16, _h: u16) {
        // TODO: mark layout dirty
    }

    /// Global shortcuts that work regardless of overlay state.
    fn handle_global_key(&self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => Some(Action::Quit),
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => Some(Action::Quit),
            KeyCode::F(1) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::ThemePanel))),
            KeyCode::F(2) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::AgentsPanel))),
            KeyCode::F(3) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::SkillsPanel))),
            KeyCode::F(4) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::SettingsPanel))),
            KeyCode::F(5) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::SearchPanel))),
            KeyCode::F(6) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::MessagePanel))),
            KeyCode::F(7) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::ModelPanel))),
            KeyCode::F(8) => Some(Action::Overlay(OverlayAction::Open(OverlayKind::SessionPanel))),
            KeyCode::Esc if !self.overlays.is_empty() => {
                Some(Action::Overlay(OverlayAction::CloseTop))
            }
            _ => None,
        }
    }

    // ── Action processing ──

    fn process_action(&mut self, action: Action) {
        let mut queue = vec![action];
        while let Some(action) = queue.pop() {
            match action {
                Action::Quit => {
                    self.should_quit = true;
                }
                Action::Overlay(OverlayAction::Open(kind)) => {
                    self.open_overlay(kind);
                }
                Action::Overlay(OverlayAction::Close(kind)) => {
                    self.close_overlay(kind, &mut queue);
                }
                Action::Overlay(OverlayAction::CloseTop) => {
                    if let Some(mut overlay) = self.overlays.pop() {
                        let palette = &self.current_palette;
                        let mut ctx = UpdateContext {
                            runtime: &mut self.runtime,
                            palette,
                        };
                        let follow = overlay.update(
                            &Action::Overlay(OverlayAction::Close(OverlayKind::ThemePanel)),
                            &mut ctx,
                        );
                        queue.extend(follow);
                    }
                }
                Action::Overlay(OverlayAction::CloseAll) => {
                    while self.overlays.pop().is_some() {}
                }
                Action::Theme(ThemeAction::Preview(name)) => {
                    self.current_palette = ThemePalette::from_name(name.as_str());
                }
                Action::Theme(ThemeAction::Set(name)) => {
                    self.current_palette = ThemePalette::from_name(name.as_str());
                    self.runtime
                        .update_config(|cfg| cfg.set_theme(name.as_str()));
                    let _ = self.runtime.save_config();
                }
                Action::Theme(ThemeAction::Toggle) => {
                    let current = ThemeName::parse(&self.current_palette.name.as_str())
                        .unwrap_or(ThemeName::Dark);
                    let next = current.toggle();
                    self.process_action(Action::Theme(ThemeAction::Preview(next)));
                    self.process_action(Action::Theme(ThemeAction::Set(next)));
                }
                Action::Search(SearchAction::SwitchProvider(provider)) => {
                    self.runtime
                        .update_config(|cfg| cfg.websearch.default_provider = provider);
                    let _ = self.runtime.save_config();
                }
                Action::Search(SearchAction::SaveApiKey {
                    provider,
                    key,
                    is_cx,
                }) => {
                    self.runtime.update_auth(|auth| {
                        if is_cx {
                            auth.web.google_cx = Some(key);
                        } else {
                            auth.web.search_api_keys.insert(provider, key);
                        }
                    });
                    let _ = self.runtime.save_auth();
                }
                Action::Connect(ConnectAction::SaveApiKey {
                    provider_id,
                    key,
                }) => {
                    if key.trim().is_empty() {
                        self.set_notice("API key was empty");
                        return;
                    }

                    self.runtime
                        .update_auth(|auth| auth.set_api_key(&provider_id, &key));
                    let _ = self.runtime.save_auth();

                    // Resolve the provider's default model and switch to it
                    match self
                        .runtime
                        .config()
                        .resolve_provider_default_model(&self.runtime.auth(), &provider_id)
                    {
                        Ok(model) => {
                            self.runtime.set_active_model(model.clone());

                            // Persist model to current session if one is active
                            if let Some(session_id) = self.current_session_id {
                                if self
                                    .runtime
                                    .session_manager()
                                    .store()
                                    .load_session_record(session_id)
                                    .ok()
                                    .flatten()
                                    .is_some()
                                {
                                    let _ = self
                                        .runtime
                                        .session_manager()
                                        .update_session_model(
                                            session_id,
                                            &model.provider_id,
                                            &model.provider_display_name,
                                            &model.model_id,
                                            &model.display_name,
                                        );
                                }
                            }

                            self.set_notice(format!(
                                "Connected to {}",
                                model.provider_display_name
                            ));
                        }
                        Err(e) => {
                            self.set_notice(format!(
                                "Connected, but failed to resolve model: {e}"
                            ));
                        }
                    }
                }
                Action::Connect(ConnectAction::PruneOrphans) => {
                    let known_ids = self.runtime.config().provider_ids();
                    let mut pruned = 0usize;
                    self.runtime.update_auth(|auth| {
                        pruned = auth.prune_orphan_providers(&known_ids);
                    });
                    if pruned > 0 {
                        let _ = self.runtime.save_auth();
                        self.set_notice(format!(
                            "Pruned {pruned} orphan provider(s) from auth file"
                        ));
                    } else {
                        self.set_notice("No orphan auth entries to prune");
                    }
                }
                Action::Session(SessionAction::Select(session_id)) => {
                    // Switch to the selected session
                    self.current_session_id = Some(session_id);
                    self.scroll_target = None;

                    // Load session record (for logging)
                    match self.runtime.session_manager().load_session(session_id) {
                        Ok(Some(record)) => {
                            log::info!(
                                "Switching to session: {} ({})",
                                record.title,
                                session_id
                            );
                        }
                        Ok(None) => {
                            log::warn!("Session not found: {session_id}");
                        }
                        Err(e) => {
                            log::error!("Failed to load session: {e}");
                        }
                    }

                    // Continue the agent loop if the session has pending work
                    let rt = self.runtime.clone();
                    tokio::spawn(async move {
                        if let Err(e) = rt.continue_session(session_id).await {
                            log::error!("continue_session failed: {e}");
                        }
                    });
                }
                Action::Session(SessionAction::Reload) => {
                    // Broadcast to overlays so SessionPanel reloads its list.
                    let palette = &self.current_palette;
                    let mut ctx = UpdateContext {
                        runtime: &mut self.runtime,
                        palette,
                    };
                    queue.extend(self.overlays.update_all(&action, &mut ctx));
                }
                Action::Session(SessionAction::Fork(message_id)) => {
                    let session_id = match self.current_session_id {
                        Some(id) => id,
                        None => return,
                    };

                    // Load messages from DB
                    let messages = match self.runtime.session_manager().load_messages(session_id) {
                        Ok(msgs) => msgs,
                        Err(e) => {
                            log::error!("Failed to load messages for fork: {e}");
                            return;
                        }
                    };

                    // Find the message index by UUID
                    let message_index = match messages.iter().position(|m| m.id == message_id) {
                        Some(idx) => idx,
                        None => {
                            log::warn!("Fork target message not found: {}", message_id);
                            return;
                        }
                    };

                    // Load session title from DB
                    let session_title = self
                        .runtime
                        .session_manager()
                        .load_session(session_id)
                        .ok()
                        .flatten()
                        .map(|r| r.title)
                        .unwrap_or_default();

                    let workspace_root =
                        self.runtime.workspace_root().to_string_lossy().to_string();
                    let config = self.runtime.config();
                    let auth = self.runtime.auth();
                    let active_model = match config.resolve_active_model(&auth) {
                        Ok(m) => m,
                        Err(e) => {
                            log::error!("Failed to resolve active model for fork: {e}");
                            return;
                        }
                    };

                    // Create new session
                    let new_session_id = uuid::Uuid::new_v4();
                    if let Err(e) = self.runtime.session_manager().create_session(
                        new_session_id,
                        &workspace_root,
                        &active_model.provider_id,
                        &active_model.provider_display_name,
                        &active_model.model_id,
                        &active_model.display_name,
                        &format!("Fork of {}", session_title),
                    ) {
                        log::error!("Failed to create fork session: {e}");
                        return;
                    }

                    // Copy parent's system prompt
                    if !active_model.system_prompt.is_empty() {
                        let _ = self.runtime.session_manager().store().update_session(
                            new_session_id, None, None, None, None,
                            Some(&active_model.system_prompt), None, None, None, None,
                        );
                    }

                    // Copy messages up to the selected message, assigning new IDs
                    let mut id_mapping: std::collections::HashMap<uuid::Uuid, uuid::Uuid> =
                        std::collections::HashMap::new();

                    for original in messages.iter().take(message_index + 1) {
                        let mut new_message = original.clone();
                        let new_id = uuid::Uuid::new_v4();
                        id_mapping.insert(original.id, new_id);
                        new_message.id = new_id;

                        // Update tool_call_id references to new IDs
                        if let Some(ref tool_call_id) = new_message.tool_call_id {
                            if let Ok(old_id) = uuid::Uuid::parse_str(tool_call_id) {
                                if let Some(&new_tool_call_id) = id_mapping.get(&old_id) {
                                    new_message.tool_call_id =
                                        Some(new_tool_call_id.to_string());
                                }
                            }
                        }

                        if let Err(e) = self
                            .runtime
                            .session_manager()
                            .append_message(new_session_id, &new_message)
                        {
                            log::error!("Failed to copy message to fork: {e}");
                            return;
                        }
                    }

                    // Switch to the new session
                    self.current_session_id = Some(new_session_id);
                    self.scroll_target = None;

                    self.set_notice(format!(
                        "Forked session with {} messages",
                        message_index + 1,
                    ));

                    log::info!(
                        "Forked session {} -> {} with {} messages",
                        session_id,
                        new_session_id,
                        message_index + 1,
                    );
                }
                Action::Session(SessionAction::Undo) => {
                    let session_id = match self.current_session_id {
                        Some(id) => id,
                        None => return,
                    };
                    self.set_notice("Undo in progress...");
                    let rt = self.runtime.clone();
                    tokio::spawn(async move {
                        if let Err(e) = rt.undo(session_id).await {
                            log::error!("Undo failed: {e}");
                        }
                    });
                }
                Action::Session(SessionAction::Rename(session_id, title)) => {
                    let final_title = if title.trim().is_empty() {
                        "Untitled session".to_string()
                    } else {
                        title.trim().to_string()
                    };
                    match self
                        .runtime
                        .session_manager()
                        .update_session(session_id, Some(&final_title), None)
                    {
                        Ok(_) => {
                            self.set_notice("Session title updated");
                            log::info!("Renamed session {} to {}", session_id, final_title);
                        }
                        Err(e) => log::error!("Failed to rename session: {e}"),
                    }
                }
                Action::Chat(ChatAction::SetInput(text)) => {
                    // TODO: route to Composer once migrated
                    log::info!("SetInput: {}", text);
                }
                Action::Chat(ChatAction::ScrollTo(message_id)) => {
                    self.scroll_target = Some(message_id);
                    log::info!("ScrollTo: {}", message_id);
                }
                Action::Noop => {}
                _ => {
                    // Broadcast to all overlays
                    let palette = &self.current_palette;
                    let mut ctx = UpdateContext {
                        runtime: &mut self.runtime,
                        palette,
                    };
                    queue.extend(self.overlays.update_all(&action, &mut ctx));
                }
            }
        }
    }

    fn open_overlay(&mut self, kind: OverlayKind) {
        let component: Option<Box<dyn Component>> = match kind {
            OverlayKind::ThemePanel => {
                let current = ThemeName::parse(&self.current_palette.name.as_str())
                    .unwrap_or(ThemeName::Dark);
                Some(Box::new(ThemePanel::new(current)))
            }
            OverlayKind::AgentsPanel => Some(Box::new(AgentsPanel::new())),
            OverlayKind::SkillsPanel => {
                let catalog = &self.runtime.skills;
                let skills: Vec<SkillItem> = catalog
                    .all()
                    .iter()
                    .map(|s| SkillItem {
                        name: s.name.clone(),
                        description: s.description.clone(),
                        location: s.location.clone(),
                    })
                    .collect();
                Some(Box::new(SkillsPanel::new(skills)))
            }
            OverlayKind::SettingsPanel => {
                let config = self.runtime.config();
                Some(Box::new(SettingsPanel::new(&config)))
            }
            OverlayKind::SearchPanel => {
                let config = self.runtime.config();
                let auth = self.runtime.auth();
                Some(Box::new(SearchPanel::new(
                    &config.websearch.default_provider,
                    &auth,
                )))
            }
            OverlayKind::MessagePanel => {
                // TODO: populate from ChatContext once Chat component is migrated (Phase 6)
                Some(Box::new(MessagePanel::new(Vec::new())))
            }
            OverlayKind::ModelPanel => {
                use crate::components::overlays::model::ModelPanelTab;
                let config = self.runtime.config();
                let auth = self.runtime.auth();
                let active_model = match config.resolve_active_model(&auth) {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("Failed to resolve active model: {e}");
                        return;
                    }
                };

                let mut tabs = vec![ModelPanelTab::new(
                    "general",
                    "General",
                    &active_model.label(),
                )];
                for agent_type in AgentType::all() {
                    if *agent_type == AgentType::General {
                        continue;
                    }
                    let ty = agent_type.display_name();
                    let label = config.agent_model_display(ty);
                    tabs.push(ModelPanelTab::new(
                        ty,
                        agent_type.display_name(),
                        &label,
                    ));
                }

                let connected_models = config.connected_models(&auth);
                Some(Box::new(ModelPanel::new(
                    tabs,
                    connected_models,
                    active_model,
                )))
            }
            OverlayKind::SessionPanel => {
                use crate::components::overlays::session::SessionViewMode;
                let store = self.runtime.session_manager().store();
                let sessions = store.list_sessions(1000, 0).unwrap_or_default();
                // TODO: get real current_session_id from ChatContext (Phase 6)
                let current_session_id = uuid::Uuid::nil();
                Some(Box::new(SessionPanel::new(
                    sessions,
                    SessionViewMode::CurrentWorkspace,
                    current_session_id,
                )))
            }
            OverlayKind::ForkConfirmDialog {
                message_id,
                message_count,
            } => Some(Box::new(ForkConfirmDialog::new(message_id, message_count))),
            OverlayKind::UndoConfirmDialog {
                message_id,
                content,
            } => Some(Box::new(UndoConfirmDialog::new(message_id, content))),
            OverlayKind::RenameDialog => {
                let session_id = self.current_session_id.unwrap_or(uuid::Uuid::nil());
                let title = self
                    .runtime
                    .session_manager()
                    .load_session(session_id)
                    .ok()
                    .flatten()
                    .map(|r| r.title)
                    .unwrap_or_default();
                Some(Box::new(RenameDialog::new(session_id, title)))
            }
            OverlayKind::ImageViewer => {
                // ImageViewer requires data from a chat message (data_url + filename).
                // This is triggered by ChatAction::ToggleImage which will be routed
                // once Chat/MessageList is migrated (Phase 6). For now return None
                // so opening ImageViewer is a no-op until the Chat component provides data.
                None
            }
            OverlayKind::ConnectDialog => {
                Some(Box::new(ConnectDialog::new()))
            }
            _ => None,
        };
        if let Some(component) = component {
            self.overlays.push(component);
        }
    }

    fn close_overlay(&mut self, kind: OverlayKind, queue: &mut Vec<Action>) {
        if let Some(mut overlay) = self.overlays.pop() {
            let palette = &self.current_palette;
            let mut ctx = UpdateContext {
                runtime: &mut self.runtime,
                palette,
            };
            queue.extend(
                overlay.update(
                    &Action::Overlay(OverlayAction::Close(kind)),
                    &mut ctx,
                ),
            );
        }
    }

    // ── Drawing ──

    pub fn draw(&mut self, frame: &mut Frame) {
        let palette = self.current_palette;
        let area = frame.area();

        // Background
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.background)),
            area,
        );

        // Welcome / status text when no overlay is open
        if self.overlays.is_empty() {
            let welcome = Paragraph::new(Line::from(vec![
                Span::styled(
                    "tidev",
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  —  "),
                Span::styled("F1", Style::default().fg(palette.accent)),
                Span::raw(" Theme  ·  "),
                Span::styled("F2", Style::default().fg(palette.accent)),
                Span::raw(" Agents  ·  "),
                Span::styled("F3", Style::default().fg(palette.accent)),
                Span::raw(" Skills  ·  "),
                Span::styled("F4", Style::default().fg(palette.accent)),
                Span::raw(" Settings  ·  "),
                Span::styled("F5", Style::default().fg(palette.accent)),
                Span::raw(" Search  ·  "),
                Span::styled("F6", Style::default().fg(palette.accent)),
                Span::raw(" Messages  ·  "),
                Span::styled("F7", Style::default().fg(palette.accent)),
                Span::raw(" Models  ·  "),
                Span::styled("F8", Style::default().fg(palette.accent)),
                Span::raw(" Sessions  ·  "),
                Span::styled("Ctrl+C", Style::default().fg(palette.accent)),
                Span::raw(" quit"),
            ]))
            .style(Style::default().fg(palette.text).bg(palette.background));
            frame.render_widget(welcome, area);
        }

        // Build DrawContext
        let draw_ctx = DrawContext {
            palette,
            focused: true,
            chat_context: None,
        };

        // Draw overlays
        self.overlays.draw(frame, area, &draw_ctx);

        // ── Status notice (last_notice) ──
        // Rendered at the very bottom line, visible regardless of overlays.
        if let Some((msg, _)) = &self.last_notice {
            let notice_y = area.bottom().saturating_sub(1);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    msg.as_str(),
                    Style::default().fg(palette.muted),
                )))
                .style(Style::default().bg(palette.background)),
                Rect::new(area.x + 1, notice_y, area.width.saturating_sub(2), 1),
            );
        }

        // ── Toast notification ──
        // Small popup at the top-right, auto-expires.
        if let Some((msg, expires_at)) = &self.toast.clone() {
            if Instant::now() < *expires_at {
                let toast_width = (msg.len() as u16).min(32).saturating_add(2);
                let toast_rect = Rect::new(
                    area.right().saturating_sub(toast_width + 1),
                    area.y + 1,
                    toast_width,
                    3,
                );
                frame.render_widget(Clear, toast_rect);
                let block = Block::default()
                    .style(Style::default().bg(palette.panel).fg(palette.text));
                let centered = format!("\n{}", msg);
                frame.render_widget(
                    Paragraph::new(centered)
                        .style(Style::default().bg(palette.panel).fg(palette.text))
                        .alignment(Alignment::Center)
                        .block(block),
                    toast_rect,
                );
            } else {
                self.toast = None;
            }
        }
    }
}
