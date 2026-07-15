//! SettingsPanel component — settings panel.
//!
//! Mirrors the old `tidev_tui::ui::settings_panel` module with a self-contained
//! Component implementation. All value types are re-defined here to avoid
//! depending on private types from the old crate.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Margin, Position, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem};

use crate::action::{Action, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::centered_rect;

// ---------------------------------------------------------------------------
// Value types (redefined, matching the old crate)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub(crate) enum SettingType {
    Toggle(bool),
    Number {
        value: f32,
        min: f32,
        max: f32,
    },
    Cycle {
        options: Vec<String>,
        selected: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SettingKey {
    NotificationEnabled,
    LoggingEnabled,
    LogLevel,
    SaveRequestBody,
    SaveResponseBody,
    ScrollSpeed,
    AllowSensitiveFileAccess,
    AllowOutsideWorkspaceAccess,
    SubagentEnabled,
}

#[derive(Clone, Debug)]
pub(crate) struct SettingItem {
    pub name: String,
    pub description: String,
    pub setting_type: SettingType,
    pub key: SettingKey,
    pub disabled: bool,
}

// ---------------------------------------------------------------------------
// SettingsPanel component
// ---------------------------------------------------------------------------

pub(crate) struct SettingsPanel {
    selected_index: usize,
    items: Vec<SettingItem>,
}

impl SettingsPanel {
    /// Build the settings items from the current config.
    pub(crate) fn new(config: &tidev_config::AppConfig) -> Self {
        let log_levels = vec![
            "DEBUG".to_string(),
            "INFO".to_string(),
            "WARN".to_string(),
            "ERROR".to_string(),
        ];
        let log_level_index = log_levels
            .iter()
            .position(|l| l == &config.logging.level.to_uppercase())
            .unwrap_or(1);

        let items = vec![
            SettingItem {
                name: "Notifications".to_string(),
                description: "Enable system notifications".to_string(),
                setting_type: SettingType::Toggle(config.notifications.enabled),
                key: SettingKey::NotificationEnabled,
                disabled: false,
            },
            SettingItem {
                name: "Logging".to_string(),
                description: "Enable debug logging to file".to_string(),
                setting_type: SettingType::Toggle(config.logging.enabled),
                key: SettingKey::LoggingEnabled,
                disabled: false,
            },
            SettingItem {
                name: "Log Level".to_string(),
                description: format!("Log level: {}", log_levels[log_level_index]),
                setting_type: SettingType::Cycle {
                    options: log_levels,
                    selected: log_level_index,
                },
                key: SettingKey::LogLevel,
                disabled: false,
            },
            SettingItem {
                name: "Save Request Body".to_string(),
                description: "Save LLM request bodies to /tmp/tidev-requests/ for debugging"
                    .to_string(),
                setting_type: SettingType::Toggle(config.logging.save_request_body),
                key: SettingKey::SaveRequestBody,
                disabled: false,
            },
            SettingItem {
                name: "Save Response Body".to_string(),
                description:
                    "Save LLM streaming response payloads to /tmp/tidev-responses/ for debugging"
                        .to_string(),
                setting_type: SettingType::Toggle(config.logging.save_response_body),
                key: SettingKey::SaveResponseBody,
                disabled: false,
            },
            SettingItem {
                name: "Scroll Speed".to_string(),
                description: format!("Scroll speed multiplier: {:.1}", config.ui.scroll_speed),
                setting_type: SettingType::Number {
                    value: config.ui.scroll_speed,
                    min: 1.0,
                    max: 10.0,
                },
                key: SettingKey::ScrollSpeed,
                disabled: false,
            },
            SettingItem {
                name: "Allow Sensitive File Access".to_string(),
                description: "Allow reading sensitive files without confirmation".to_string(),
                setting_type: SettingType::Toggle(
                    config.access_control.allow_sensitive_file_access,
                ),
                key: SettingKey::AllowSensitiveFileAccess,
                disabled: false,
            },
            SettingItem {
                name: "Allow Outside Workspace Access".to_string(),
                description: "Allow accessing files outside workspace without confirmation"
                    .to_string(),
                setting_type: SettingType::Toggle(
                    config.access_control.allow_outside_workspace_access,
                ),
                key: SettingKey::AllowOutsideWorkspaceAccess,
                disabled: false,
            },
            SettingItem {
                name: "Subagent".to_string(),
                description: "Enable subagent (task tool)".to_string(),
                setting_type: SettingType::Toggle(config.subagent.enabled),
                key: SettingKey::SubagentEnabled,
                disabled: false,
            },
        ];

        Self {
            selected_index: 0,
            items,
        }
    }

    // ── Navigation helpers ──

    fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    fn move_down(&mut self) {
        if self.selected_index < self.items.len().saturating_sub(1) {
            self.selected_index += 1;
        }
    }

    /// Toggle for Toggle / Cycle type.
    fn toggle_selected(&mut self) {
        let selected = self.selected_index;
        let Some(item) = self.items.get(selected) else {
            return;
        };
        if item.disabled {
            return;
        }
        if let Some(item) = self.items.get_mut(selected) {
            match &mut item.setting_type {
                SettingType::Toggle(val) => {
                    *val = !*val;
                }
                SettingType::Cycle {
                    options,
                    selected: sel,
                } => {
                    *sel = (*sel + 1) % options.len();
                    item.description = format!("Log level: {}", options[*sel]);
                }
                SettingType::Number { .. } => {}
            }
        }
    }

    /// Increase value for Number type only.
    fn increase_selected(&mut self) {
        if let Some(item) = self.items.get_mut(self.selected_index)
            && let SettingType::Number {
                value,
                min: _,
                max,
            } = &mut item.setting_type
            {
                *value = (*value + 1.0).min(*max);
                item.description = format!("Scroll speed multiplier: {:.1}", *value);
            }
    }

    /// Decrease value for Number type only.
    fn decrease_selected(&mut self) {
        if let Some(item) = self.items.get_mut(self.selected_index)
            && let SettingType::Number {
                value,
                min,
                max: _,
            } = &mut item.setting_type
            {
                *value = (*value - 1.0).max(*min);
                item.description = format!("Scroll speed multiplier: {:.1}", *value);
            }
    }

    /// Apply current items to an AppConfig.
    fn apply_to_config(items: &[SettingItem], config: &mut tidev_config::AppConfig) {
        for item in items {
            match item.key {
                SettingKey::NotificationEnabled => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        config.notifications.enabled = val;
                    }
                }
                SettingKey::LoggingEnabled => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        config.logging.enabled = val;
                    }
                }
                SettingKey::LogLevel => {
                    if let SettingType::Cycle {
                        ref options,
                        selected,
                    } = item.setting_type
                        && selected < options.len() {
                            config.logging.level = options[selected].clone();
                        }
                }
                SettingKey::SaveRequestBody => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        config.logging.save_request_body = val;
                    }
                }
                SettingKey::SaveResponseBody => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        config.logging.save_response_body = val;
                    }
                }
                SettingKey::ScrollSpeed => {
                    if let SettingType::Number { value, .. } = item.setting_type {
                        config.ui.scroll_speed = value;
                    }
                }
                SettingKey::AllowSensitiveFileAccess => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        config.access_control.allow_sensitive_file_access = val;
                    }
                }
                SettingKey::AllowOutsideWorkspaceAccess => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        config.access_control.allow_outside_workspace_access = val;
                    }
                }
                SettingKey::SubagentEnabled => {
                    if let SettingType::Toggle(val) = item.setting_type {
                        config.subagent.enabled = val;
                    }
                }
            }
        }
    }
}

impl Component for SettingsPanel {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_up();
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_down();
                None
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.toggle_selected();
                None
            }
            KeyCode::Left => {
                self.decrease_selected();
                None
            }
            KeyCode::Right => {
                self.increase_selected();
                None
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                Some(Action::Overlay(OverlayAction::Close(OverlayKind::SettingsPanel)))
            }
            _ => None,
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        let position = Position::new(mouse.column, mouse.row);
        if !area.contains(position) {
            return None;
        }

        let overlay = centered_rect(64, 22, area);
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.move_up();
                None
            }
            MouseEventKind::ScrollDown => {
                self.move_down();
                None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let local_y = position.y.saturating_sub(inner.y);
                // Each item renders as 2 lines, so row = local_y / 2
                let row = (local_y / 2) as usize;
                if row < self.items.len() {
                    self.selected_index = row;
                    self.toggle_selected();
                }
                Some(Action::Noop)
            }
            _ => Some(Action::Noop),
        }
    }

    fn update(&mut self, action: &Action, ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::SettingsPanel)) => {
                let items = self.items.clone();
                ctx.runtime.update_config(|cfg| {
                    Self::apply_to_config(&items, cfg);
                });
                let _ = ctx.runtime.save_config();
                vec![]
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        // 8 items × ~2 lines each = 22 rows
        let overlay = centered_rect(64, 24, rect);
        frame.render_widget(Clear, overlay);

        let panel_block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(panel_block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let list_items: Vec<ListItem> = self
            .items
            .iter()
            .map(|item| {
                let fg = if item.disabled {
                    palette.muted
                } else {
                    palette.text
                };
                let status: String = match &item.setting_type {
                    SettingType::Toggle(true) => "[x]".to_string(),
                    SettingType::Toggle(false) => "[ ]".to_string(),
                    SettingType::Number { .. } => "[~]".to_string(),
                    SettingType::Cycle {
                        options,
                        selected,
                    } => {
                        let current = options.get(*selected).map(|s| s.as_str()).unwrap_or("?");
                        format!("[{current}]")
                    }
                };
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(
                            format!(" {} ", status),
                            Style::default()
                                .fg(match &item.setting_type {
                                    SettingType::Toggle(true) => {
                                        if item.disabled {
                                            palette.muted
                                        } else {
                                            palette.accent
                                        }
                                    }
                                    _ => palette.muted,
                                })
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            &item.name,
                            Style::default().fg(fg).add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(vec![
                        Span::raw("    "),
                        Span::styled(&item.description, Style::default().fg(palette.muted)),
                    ]),
                ])
            })
            .collect();

        let list = List::new(list_items)
            .style(
                Style::default()
                    .bg(palette.panel_alt)
                    .fg(palette.text),
            )
            .highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );

        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(self.selected_index));

        frame.render_stateful_widget(list, inner, &mut list_state);
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn z_order(&self) -> u8 {
        10
    }

    fn blocks_input(&self) -> bool {
        true
    }
}
