//! McpServerPanel — MCP server management panel.
//!
//! Displays all configured MCP servers with their connection status,
//! tool counts, and allows the user to connect/disconnect, refresh,
//! add, edit, and remove servers.

use std::collections::BTreeMap;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Margin, Rect};
use ratatui::prelude::{Color, Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use tidev_config::mcp::McpServerConfig;
use tidev_core::mcp::{McpConnectionStatus, McpManager, McpServerSummary};

use crate::action::{Action, McpAction, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, UpdateContext};
use crate::utils::{centered_rect, render_scrollbar, single_line_input_cursor};

// ---------------------------------------------------------------------------
// Editor state for adding / editing a server
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct ServerDraft {
    name: String,
    kind: String, // "stdio", "http", "sse"
    command: String,
    args: String,
    cwd: String,
    env: String,
    url: String,
    headers: String,
}

impl ServerDraft {
    fn new() -> Self {
        Self {
            name: String::new(),
            kind: "stdio".to_string(),
            command: String::new(),
            args: String::new(),
            cwd: String::new(),
            env: String::new(),
            url: String::new(),
            headers: String::new(),
        }
    }

    fn to_config(&self) -> Result<McpServerConfig> {
        match self.kind.as_str() {
            "stdio" => {
                if self.command.trim().is_empty() {
                    anyhow::bail!("Command is required for stdio servers");
                }
                let args: Vec<String> = shlex::split(&self.args).unwrap_or_default();
                let env: BTreeMap<String, String> = self
                    .env
                    .lines()
                    .filter_map(|line| {
                        let line = line.trim();
                        let (k, v) = line.split_once('=')?;
                        Some((k.trim().to_string(), v.trim().to_string()))
                    })
                    .collect();
                let cwd = if self.cwd.trim().is_empty() {
                    None
                } else {
                    Some(self.cwd.trim().to_string())
                };
                Ok(McpServerConfig::Stdio {
                    command: self.command.trim().to_string(),
                    args,
                    cwd,
                    env,
                })
            }
            "http" | "sse" => {
                if self.url.trim().is_empty() {
                    anyhow::bail!("URL is required for HTTP/SSE servers");
                }
                let url = self.url.trim().to_string();
                let headers: BTreeMap<String, String> = self
                    .headers
                    .lines()
                    .filter_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        let name = name.trim();
                        if name.is_empty() {
                            return None;
                        }
                        Some((name.to_string(), value.trim().to_string()))
                    })
                    .collect();
                if self.kind == "http" {
                    Ok(McpServerConfig::Http { url, headers })
                } else {
                    Ok(McpServerConfig::Sse { url, headers })
                }
            }
            other => anyhow::bail!("unknown MCP server kind '{other}'"),
        }
    }
}

// ---------------------------------------------------------------------------
// Panel component
// ---------------------------------------------------------------------------

pub(crate) struct McpServerPanel {
    mcp: McpManager,
    summaries: Vec<McpServerSummary>,
    filtered_indices: Vec<usize>,
    selected_index: usize,
    query: String,
    list_scroll: usize,
    query_active: bool,
    editing: bool,
    edit_draft: ServerDraft,
    edit_scroll: usize,
    edit_original_name: Option<String>,
    error_message: Option<String>,
}

impl McpServerPanel {
    pub(crate) fn new(mcp: &McpManager) -> Self {
        let summaries = mcp.summaries();
        let filtered_indices: Vec<usize> = (0..summaries.len()).collect();
        Self {
            mcp: mcp.clone(),
            summaries,
            filtered_indices,
            selected_index: 0,
            query: String::new(),
            list_scroll: 0,
            query_active: false,
            editing: false,
            edit_draft: ServerDraft::new(),
            edit_scroll: 0,
            edit_original_name: None,
            error_message: None,
        }
    }

    fn filtered_count(&self) -> usize {
        self.filtered_indices.len()
    }

    fn selected_summary(&self) -> Option<&McpServerSummary> {
        self.filtered_indices
            .get(self.selected_index)
            .and_then(|&idx| self.summaries.get(idx))
    }

    fn status_color(status: &McpConnectionStatus) -> Color {
        match status {
            McpConnectionStatus::Connected => Color::Green,
            McpConnectionStatus::Connecting => Color::Yellow,
            McpConnectionStatus::Disconnected => Color::DarkGray,
            McpConnectionStatus::Failed(_) => Color::Red,
        }
    }

    fn refresh_list(&mut self) {
        self.summaries = self.mcp.summaries();
        self.refilter();
    }

    fn refilter(&mut self) {
        let q = self.query.trim().to_ascii_lowercase();
        if q.is_empty() {
            self.filtered_indices = (0..self.summaries.len()).collect();
        } else {
            self.filtered_indices = self
                .summaries
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.name.to_ascii_lowercase().contains(&q)
                        || s.kind.to_ascii_lowercase().contains(&q)
                        || s.status.label().contains(&q)
                })
                .map(|(i, _)| i)
                .collect();
        }
        self.selected_index = self
            .selected_index
            .min(self.filtered_indices.len().saturating_sub(1));
        self.list_scroll = 0;
    }

    fn ensure_scroll_visible(&mut self, item_count: usize) {
        if self.selected_index < self.list_scroll {
            self.list_scroll = self.selected_index;
        } else if self.selected_index >= self.list_scroll + item_count.saturating_sub(1) {
            self.list_scroll = self
                .selected_index
                .saturating_sub(item_count.saturating_sub(2));
        }
    }

    fn move_up(&mut self, _step: usize) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        } else {
            self.selected_index = self.filtered_indices.len().saturating_sub(1);
        }
        self.ensure_scroll_visible(8);
    }

    fn move_down(&mut self, _step: usize) {
        if self.selected_index + 1 < self.filtered_indices.len() {
            self.selected_index += 1;
        } else {
            self.selected_index = 0;
        }
        self.ensure_scroll_visible(8);
    }

    fn toggle_selected(&mut self) -> Option<Action> {
        let name = self.selected_summary()?.name.clone();
        Some(Action::Mcp(McpAction::Toggle(name)))
    }

    fn refresh_selected(&mut self) -> Option<Action> {
        let name = self.selected_summary()?.name.clone();
        Some(Action::Mcp(McpAction::Refresh(name)))
    }

    fn start_add(&mut self) {
        self.editing = true;
        self.edit_draft = ServerDraft::new();
        self.edit_original_name = None;
        self.edit_scroll = 0;
        self.error_message = None;
    }

    fn start_edit(&mut self) {
        let summary_entry = self.selected_summary().map(|s| {
            let config = self.mcp.server_config(&s.name);
            (s.name.clone(), s.kind.clone(), config)
        });

        let Some((summary_name, kind, config)) = summary_entry else {
            return;
        };

        let mut draft = ServerDraft::new();
        draft.name = summary_name.clone();
        draft.kind = kind;
        if let Some(config) = config {
            match config {
                McpServerConfig::Stdio {
                    command,
                    args,
                    cwd,
                    env,
                    ..
                } => {
                    draft.command = command;
                    draft.args = args.join(" ");
                    draft.cwd = cwd.unwrap_or_default();
                    draft.env = env
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                }
                McpServerConfig::Http { url, headers } => {
                    draft.url = url;
                    draft.headers = headers
                        .iter()
                        .map(|(name, value)| format!("{name}: {value}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                }
                McpServerConfig::Sse { url, headers } => {
                    draft.url = url;
                    draft.headers = headers
                        .iter()
                        .map(|(name, value)| format!("{name}: {value}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                }
            }
        }
        self.editing = true;
        self.edit_draft = draft;
        self.edit_original_name = Some(summary_name);
        self.edit_scroll = 0;
        self.error_message = None;
    }

    fn remove_selected(&mut self) -> Option<Action> {
        let name = self.selected_summary()?.name.clone();
        Some(Action::Mcp(McpAction::Remove(name)))
    }

    fn save_draft(&mut self) -> Option<Action> {
        let draft = self.edit_draft.clone();
        let original_name = self.edit_original_name.clone();

        match draft.to_config() {
            Ok(config) => {
                let name = draft.name.trim().to_string();
                if name.is_empty() {
                    self.error_message = Some("Server name cannot be empty".to_string());
                    return None;
                }

                self.editing = false;
                self.error_message = None;
                Some(Action::Mcp(McpAction::Upsert {
                    name,
                    config,
                    original_name,
                }))
            }
            Err(e) => {
                self.error_message = Some(format!("{}", e));
                None
            }
        }
    }
}

impl Component for McpServerPanel {
    fn is_overlay(&self) -> bool {
        true
    }

    fn blocks_input(&self) -> bool {
        true
    }

    fn overlay_uses_main_area(&self) -> bool {
        true
    }

    fn wants_terminal_cursor(&self) -> bool {
        self.query_active || self.editing
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if key.kind != KeyEventKind::Press {
            return None;
        }

        // ── Editor mode ────────────────────────────────────────────────
        if self.editing {
            match key.code {
                KeyCode::Esc => {
                    self.editing = false;
                    self.error_message = None;
                    return Some(Action::Noop);
                }
                KeyCode::Enter => {
                    return self.save_draft();
                }
                KeyCode::Tab | KeyCode::Down => {
                    self.edit_scroll = (self.edit_scroll + 1).min(7);
                    return Some(Action::Noop);
                }
                KeyCode::Up => {
                    self.edit_scroll = self.edit_scroll.saturating_sub(1);
                    return Some(Action::Noop);
                }
                KeyCode::Backspace => {
                    self.edit_draft.edit_field(self.edit_scroll, |v| {
                        let _ = v.pop();
                    });
                    return Some(Action::Noop);
                }
                KeyCode::Char(ch) => {
                    self.edit_draft.edit_field(self.edit_scroll, |v| {
                        v.push(ch);
                    });
                    return Some(Action::Noop);
                }
                _ => {}
            }
            return None;
        }

        // ── List mode ──────────────────────────────────────────────────
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Some(Action::Overlay(OverlayAction::Close(
                OverlayKind::McpServerPanel,
            ))),
            KeyCode::Up => {
                self.move_up(1);
                Some(Action::Noop)
            }
            KeyCode::Down => {
                self.move_down(1);
                Some(Action::Noop)
            }
            KeyCode::PageUp => {
                for _ in 0..8 {
                    self.move_up(1);
                }
                Some(Action::Noop)
            }
            KeyCode::PageDown => {
                for _ in 0..8 {
                    self.move_down(1);
                }
                Some(Action::Noop)
            }
            KeyCode::Enter => self.toggle_selected(),
            KeyCode::Char('r') => self.refresh_selected(),
            KeyCode::Char('n') => {
                self.start_add();
                Some(Action::Noop)
            }
            KeyCode::Char('e') => {
                self.start_edit();
                Some(Action::Noop)
            }
            KeyCode::Char('d') => self.remove_selected(),
            KeyCode::Char('/') | KeyCode::F(2) => {
                self.query_active = !self.query_active;
                if self.query_active {
                    self.query.clear();
                    self.refilter();
                }
                Some(Action::Noop)
            }
            KeyCode::Backspace if self.query_active => {
                if !self.query.is_empty() {
                    self.query.pop();
                    self.refilter();
                }
                Some(Action::Noop)
            }
            KeyCode::Char(ch) if self.query_active => {
                self.query.push(ch);
                self.refilter();
                Some(Action::Noop)
            }
            _ => None,
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return None;
        }
        // Simple click handling: check if click is within the item list
        let inner = area.inner(Margin {
            horizontal: 2,
            vertical: 3,
        });
        if mouse.column < inner.x || mouse.column >= inner.x + inner.width || mouse.row < inner.y {
            return None;
        }
        let item_index = (mouse.row - inner.y) as usize;
        if item_index < self.filtered_count() {
            self.selected_index = item_index;
            self.ensure_scroll_visible(inner.height as usize);
        }
        None
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Open(OverlayKind::McpServerPanel)) | Action::Mcp(_) => {
                self.refresh_list();
            }
            _ => {}
        }
        vec![]
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &DrawContext) {
        // ── Editor mode ────────────────────────────────────────────────
        if self.editing {
            self.draw_editor(frame, area);
            return;
        }

        // ── Main panel ─────────────────────────────────────────────────
        let palette = ctx.palette; // 4-value ThemePalette
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" MCP Servers ")
            .title_bottom(if self.query_active {
                " Filter: "
            } else {
                " [Enter] toggle  [r] refresh  [n] add  [e] edit  [d] delete  [/] filter  [Esc] close "
            });
        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);

        // ── Query bar ──────────────────────────────────────────────────
        if self.query_active {
            let query_area = Rect::new(inner.x, inner.y, inner.width, 1);
            let (visible_query, cursor) = single_line_input_cursor(query_area, 8, &self.query);
            let query_bar =
                Paragraph::new(Line::from(Span::raw(format!("Filter: {visible_query}"))))
                    .style(Style::default().fg(Color::Cyan));
            frame.render_widget(query_bar, query_area);
            frame.set_cursor_position(cursor);
        }

        // ── Item list ──────────────────────────────────────────────────
        let list_area = if self.query_active {
            Rect::new(
                inner.x,
                inner.y + 1,
                inner.width,
                inner.height.saturating_sub(2),
            )
        } else {
            Rect::new(
                inner.x,
                inner.y,
                inner.width,
                inner.height.saturating_sub(1),
            )
        };

        let display_count = list_area.height as usize;
        let total = self.filtered_count();
        let scroll = self.list_scroll;

        for i in 0..display_count.min(total.saturating_sub(scroll)) {
            let idx = scroll + i;
            let Some(&summary_idx) = self.filtered_indices.get(idx) else {
                break;
            };
            let Some(summary) = self.summaries.get(summary_idx) else {
                break;
            };

            let is_selected = idx == self.selected_index;
            let status_color = Self::status_color(&summary.status);
            let status_icon = match &summary.status {
                McpConnectionStatus::Connected => "●",
                McpConnectionStatus::Connecting => "◌",
                McpConnectionStatus::Disconnected => "○",
                McpConnectionStatus::Failed(_) => "✕",
            };

            let line_y = list_area.y + i as u16;

            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(status_color)
            };

            let line = Line::from(vec![
                Span::styled(format!(" {} ", status_icon), style.fg(status_color)),
                Span::styled(
                    format!(" {:<20} ", summary.name),
                    if is_selected {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    },
                ),
                Span::styled(
                    format!(" {:<6}", summary.kind),
                    if is_selected {
                        Style::default().fg(Color::Black).bg(Color::White)
                    } else {
                        Style::default().fg(Color::Cyan)
                    },
                ),
                Span::styled(
                    format!(" {} tools", summary.tool_count),
                    if is_selected {
                        Style::default().fg(Color::Black).bg(Color::White)
                    } else {
                        Style::default().fg(Color::Gray)
                    },
                ),
                Span::raw(" "),
                Span::styled(
                    summary.status_text(),
                    if is_selected {
                        Style::default().fg(Color::Black).bg(Color::White)
                    } else {
                        Style::default().fg(status_color)
                    },
                ),
            ]);

            frame.render_widget(
                Paragraph::new(line),
                Rect::new(list_area.x, line_y, list_area.width, 1),
            );
        }

        // ── Scrollbar ──────────────────────────────────────────────────
        render_scrollbar(frame, list_area, scroll, display_count, palette, false);

        // ── Help text ──────────────────────────────────────────────────
        if !self.query_active {
            let help = Paragraph::new(Line::from(Span::styled(
                " [Enter] toggle  [r] refresh  [n] add  [e] edit  [d] delete  [/] filter  [Esc] close ",
                Style::default().fg(Color::DarkGray),
            )));
            frame.render_widget(
                help,
                Rect::new(
                    inner.x,
                    inner.y + inner.height.saturating_sub(1),
                    inner.width,
                    1,
                ),
            );
        }
    }
}

// ── Editor drawing ─────────────────────────────────────────────────────────

impl McpServerPanel {
    fn draw_editor(&mut self, frame: &mut Frame, area: Rect) {
        let editor_area = centered_rect(65, 70, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" MCP Server ");
        let inner = block.inner(editor_area);
        frame.render_widget(Clear, editor_area);
        frame.render_widget(block, editor_area);

        // ── Fields ────────────────────────────────────────────────────
        let fields = [
            ("Name", &self.edit_draft.name, 0),
            ("Kind", &self.edit_draft.kind, 1),
            ("Cmd", &self.edit_draft.command, 2),
            ("Args", &self.edit_draft.args, 3),
            ("Cwd", &self.edit_draft.cwd, 4),
            ("Env", &self.edit_draft.env, 5),
            ("URL", &self.edit_draft.url, 6),
            ("Header", &self.edit_draft.headers, 7),
        ];

        let field_start_y = inner.y;
        for (label, value, idx) in &fields {
            let y = field_start_y + *idx as u16;
            let is_active = self.edit_scroll == *idx;
            let label_style = if is_active {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let colon = Span::styled(": ", label_style);
            let value_style = if is_active {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let field_area = Rect::new(inner.x, y, inner.width, 1);
            let (visible_value, cursor) = single_line_input_cursor(field_area, 10, value);
            let display_value = if is_active {
                visible_value.to_string()
            } else {
                value.to_string()
            };

            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!("  {:<6}", label), label_style),
                    colon,
                    Span::styled(display_value, value_style),
                ])),
                field_area,
            );
            if is_active {
                frame.set_cursor_position(cursor);
            }
        }

        // ── Error message ─────────────────────────────────────────────
        if let Some(ref err) = self.error_message {
            let err_style = Style::default().fg(Color::Red);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!("  Error: {err}"),
                    err_style,
                ))),
                Rect::new(
                    inner.x,
                    field_start_y + fields.len() as u16 + 1,
                    inner.width,
                    1,
                ),
            );
        }

        // ── Help text ─────────────────────────────────────────────────
        let help_y = inner.y + inner.height.saturating_sub(1);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  [Tab/↑↓] nav  [Enter] save  [Esc] cancel",
                Style::default().fg(Color::DarkGray),
            ))),
            Rect::new(inner.x, help_y, inner.width, 1),
        );
    }
}

// ── Editor field access helper ─────────────────────────────────────────────

trait EditField {
    fn edit_field<F>(&mut self, field: usize, f: F)
    where
        F: FnOnce(&mut String);
}

impl EditField for ServerDraft {
    fn edit_field<F>(&mut self, field: usize, f: F)
    where
        F: FnOnce(&mut String),
    {
        match field {
            0 => f(&mut self.name),
            1 => {
                // Cycle through kinds
                self.kind = match self.kind.as_str() {
                    "stdio" => "http".to_string(),
                    "http" => "sse".to_string(),
                    "sse" => "stdio".to_string(),
                    _ => "stdio".to_string(),
                };
            }
            2 => f(&mut self.command),
            3 => f(&mut self.args),
            4 => f(&mut self.cwd),
            5 => f(&mut self.env),
            6 => f(&mut self.url),
            7 => f(&mut self.headers),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_draft_to_config_stdio() {
        let mut draft = ServerDraft::new();
        draft.kind = "stdio".to_string();
        draft.name = "my-server".to_string();
        draft.command = "python".to_string();
        draft.args = "-m http.server 8080".to_string();
        draft.cwd = "/project".to_string();
        draft.env = "KEY=val\nFOO=bar".to_string();

        let config = draft.to_config().unwrap();
        match config {
            McpServerConfig::Stdio {
                command,
                args,
                cwd,
                env,
                ..
            } => {
                assert_eq!(command, "python");
                assert_eq!(args, vec!["-m", "http.server", "8080"]);
                assert_eq!(cwd, Some("/project".into()));
                assert_eq!(env.get("KEY"), Some(&"val".into()));
                assert_eq!(env.get("FOO"), Some(&"bar".into()));
            }
            other => panic!("expected Stdio, got {other:?}"),
        }
    }

    #[test]
    fn test_server_draft_to_config_http() {
        let mut draft = ServerDraft::new();
        draft.kind = "http".to_string();
        draft.name = "http-server".to_string();
        draft.url = "https://mcp.example.com".to_string();
        draft.headers = "Authorization: Bearer token\nX-Trace: trace-1".to_string();

        let config = draft.to_config().unwrap();
        match config {
            McpServerConfig::Http { url, headers } => {
                assert_eq!(url, "https://mcp.example.com");
                assert_eq!(headers.get("Authorization").unwrap(), "Bearer token");
                assert_eq!(headers.get("X-Trace").unwrap(), "trace-1");
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn test_server_draft_to_config_sse() {
        let mut draft = ServerDraft::new();
        draft.kind = "sse".to_string();
        draft.name = "sse-server".to_string();
        draft.url = "http://localhost:8080/sse".to_string();

        let config = draft.to_config().unwrap();
        match config {
            McpServerConfig::Sse { url, .. } => {
                assert_eq!(url, "http://localhost:8080/sse");
            }
            other => panic!("expected Sse, got {other:?}"),
        }
    }

    #[test]
    fn test_server_draft_to_config_empty_command_fails() {
        let mut draft = ServerDraft::new();
        draft.kind = "stdio".to_string();
        draft.command = "".to_string();
        assert!(draft.to_config().is_err());
    }

    #[test]
    fn test_server_draft_to_config_empty_url_fails() {
        let mut draft = ServerDraft::new();
        draft.kind = "http".to_string();
        draft.url = "".to_string();
        assert!(draft.to_config().is_err());
    }

    #[test]
    fn test_server_draft_to_config_unknown_kind() {
        let mut draft = ServerDraft::new();
        draft.kind = "unknown".to_string();
        assert!(draft.to_config().is_err());
    }

    #[test]
    fn test_server_draft_new_defaults() {
        let draft = ServerDraft::new();
        assert_eq!(draft.kind, "stdio");
        assert!(draft.name.is_empty());
        assert!(draft.command.is_empty());
    }

    #[test]
    fn test_server_draft_edit_field_cycles_kind() {
        let mut draft = ServerDraft::new();
        assert_eq!(draft.kind, "stdio");
        draft.edit_field(1, |_| {});
        assert_eq!(draft.kind, "http");
        draft.edit_field(1, |_| {});
        assert_eq!(draft.kind, "sse");
        draft.edit_field(1, |_| {});
        assert_eq!(draft.kind, "stdio");
    }

    #[test]
    fn test_server_draft_edit_field_modifies_string() {
        let mut draft = ServerDraft::new();
        draft.edit_field(0, |v| v.push_str("my-name"));
        assert_eq!(draft.name, "my-name");
        draft.edit_field(2, |v| v.push_str("cmd"));
        assert_eq!(draft.command, "cmd");
    }

    #[test]
    fn test_server_draft_stdio_no_args() {
        let mut draft = ServerDraft::new();
        draft.command = "node".to_string();
        let config = draft.to_config().unwrap();
        match config {
            McpServerConfig::Stdio { args, .. } => {
                assert!(args.is_empty());
            }
            other => panic!("expected Stdio, got {other:?}"),
        }
    }
}
