//! McpServerPanel — MCP server management panel.
//!
//! Displays all configured MCP servers with their connection status,
//! tool counts, and allows the user to connect/disconnect, refresh,
//! add, edit, and remove servers.

use std::collections::BTreeMap;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::prelude::{Color, Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

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
    preview_scroll: usize,
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
            preview_scroll: 0,
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
        self.preview_scroll = 0;
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
        self.preview_scroll = 0;
        self.ensure_scroll_visible(8);
    }

    fn move_down(&mut self, _step: usize) {
        if self.selected_index + 1 < self.filtered_indices.len() {
            self.selected_index += 1;
        } else {
            self.selected_index = 0;
        }
        self.preview_scroll = 0;
        self.ensure_scroll_visible(8);
    }

    fn scroll_preview_up(&mut self, step: usize) {
        self.preview_scroll = self.preview_scroll.saturating_sub(step);
    }

    fn scroll_preview_down(&mut self, step: usize) {
        self.preview_scroll = self.preview_scroll.saturating_add(step);
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

    fn generate_details_lines(
        &self,
        summary: &McpServerSummary,
        config: Option<&McpServerConfig>,
        tools: &[tidev_tools::types::ToolDefinition],
        palette: &crate::theme::ThemePalette,
        details_width: usize,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        // ── Status & Transport ──
        let (status_icon, status_label, status_color) = match &summary.status {
            McpConnectionStatus::Connected => ("●", "Connected", Color::Green),
            McpConnectionStatus::Connecting => ("◌", "Connecting", Color::Yellow),
            McpConnectionStatus::Disconnected => ("○", "Disconnected", palette.muted),
            McpConnectionStatus::Failed(_) => ("✕", "Failed", Color::Red),
        };

        lines.push(Line::from(vec![
            Span::styled("  Status:    ", Style::default().fg(palette.muted)),
            Span::styled(
                format!("{status_icon} {status_label}"),
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        lines.push(Line::from(vec![
            Span::styled("  Transport: ", Style::default().fg(palette.muted)),
            Span::styled(
                summary.kind.clone(),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        // ── Configuration details ──
        if let Some(config) = config {
            match config {
                McpServerConfig::Stdio {
                    command,
                    args,
                    cwd,
                    env,
                } => {
                    lines.push(Line::from(vec![
                        Span::styled("  Command:   ", Style::default().fg(palette.muted)),
                        Span::styled(command.clone(), Style::default().fg(palette.text)),
                    ]));
                    if !args.is_empty() {
                        let args_str = args.join(" ");
                        let wrapped =
                            textwrap::wrap(&args_str, details_width.saturating_sub(16).max(20));
                        if wrapped.len() <= 1 {
                            lines.push(Line::from(vec![
                                Span::styled("  Args:      ", Style::default().fg(palette.muted)),
                                Span::styled(args_str, Style::default().fg(palette.text)),
                            ]));
                        } else {
                            lines.push(Line::from(vec![
                                Span::styled("  Args:      ", Style::default().fg(palette.muted)),
                                Span::styled(
                                    wrapped[0].to_string(),
                                    Style::default().fg(palette.text),
                                ),
                            ]));
                            for w in &wrapped[1..] {
                                lines.push(Line::from(vec![
                                    Span::styled(
                                        "             ",
                                        Style::default().fg(palette.muted),
                                    ),
                                    Span::styled(w.to_string(), Style::default().fg(palette.text)),
                                ]));
                            }
                        }
                    }
                    if let Some(cwd) = cwd {
                        lines.push(Line::from(vec![
                            Span::styled("  Cwd:       ", Style::default().fg(palette.muted)),
                            Span::styled(cwd.clone(), Style::default().fg(palette.text)),
                        ]));
                    }
                    if !env.is_empty() {
                        let env_str = env
                            .iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        lines.push(Line::from(vec![
                            Span::styled("  Env:       ", Style::default().fg(palette.muted)),
                            Span::styled(env_str, Style::default().fg(palette.muted)),
                        ]));
                    }
                }
                McpServerConfig::Http { url, headers } | McpServerConfig::Sse { url, headers } => {
                    lines.push(Line::from(vec![
                        Span::styled("  URL:       ", Style::default().fg(palette.muted)),
                        Span::styled(url.clone(), Style::default().fg(palette.text)),
                    ]));
                    if !headers.is_empty() {
                        let headers_str = headers
                            .iter()
                            .map(|(k, v)| format!("{k}: {v}"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        lines.push(Line::from(vec![
                            Span::styled("  Headers:   ", Style::default().fg(palette.muted)),
                            Span::styled(headers_str, Style::default().fg(palette.muted)),
                        ]));
                    }
                }
            }
        }

        if let McpConnectionStatus::Failed(err) = &summary.status {
            lines.push(Line::from(""));
            let err_wrap = textwrap::wrap(err.as_str(), details_width.saturating_sub(14).max(20));
            if err_wrap.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled(
                        "  Error:     ",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(err.clone(), Style::default().fg(Color::Red)),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(
                        "  Error:     ",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(err_wrap[0].to_string(), Style::default().fg(Color::Red)),
                ]));
                for line in &err_wrap[1..] {
                    lines.push(Line::from(vec![
                        Span::styled("             ", Style::default().fg(Color::Red)),
                        Span::styled(line.to_string(), Style::default().fg(Color::Red)),
                    ]));
                }
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!("  Tools ({})", tools.len()),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));

        let desc_wrap_width = details_width.saturating_sub(8).max(20);

        if tools.is_empty() {
            if summary.status == McpConnectionStatus::Connected {
                lines.push(Line::from(Span::styled(
                    "    (No tools registered by this server)",
                    Style::default().fg(palette.muted),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    "    (Press [Enter] to connect and discover tools)",
                    Style::default().fg(palette.muted),
                )));
            }
        } else {
            for (i, tool) in tools.iter().enumerate() {
                if i > 0 {
                    lines.push(Line::from(""));
                }
                let (_, raw_tool_name) = tool.mcp_target().unwrap_or(("", tool.name.as_str()));
                lines.push(Line::from(vec![
                    Span::styled("    ● ", Style::default().fg(palette.accent)),
                    Span::styled(
                        raw_tool_name.to_string(),
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                if !tool.description.trim().is_empty() {
                    for desc_line in tool.description.lines() {
                        let trimmed = desc_line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        for wrapped in textwrap::wrap(trimmed, desc_wrap_width) {
                            lines.push(Line::from(vec![Span::styled(
                                format!("      {wrapped}"),
                                Style::default().fg(palette.muted),
                            )]));
                        }
                    }
                }
            }
        }

        lines
    }
}

impl Component for McpServerPanel {
    fn is_overlay(&self) -> bool {
        true
    }

    fn z_order(&self) -> u8 {
        10
    }

    fn blocks_input(&self) -> bool {
        true
    }

    fn wants_terminal_cursor(&self) -> bool {
        self.query_active || self.editing
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        // ── Editor mode ────────────────────────────────────────────────
        if self.editing {
            let is_stdio = self.edit_draft.kind == "stdio";
            let field_ids: &[usize] = if is_stdio {
                &[0, 1, 2, 3, 4, 5]
            } else {
                &[0, 1, 6, 7]
            };
            let current_pos = field_ids
                .iter()
                .position(|&id| id == self.edit_scroll)
                .unwrap_or(0);

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
                    let next_pos = (current_pos + 1) % field_ids.len();
                    self.edit_scroll = field_ids[next_pos];
                    return Some(Action::Noop);
                }
                KeyCode::BackTab | KeyCode::Up => {
                    let prev_pos = if current_pos == 0 {
                        field_ids.len() - 1
                    } else {
                        current_pos - 1
                    };
                    self.edit_scroll = field_ids[prev_pos];
                    return Some(Action::Noop);
                }
                KeyCode::Char(' ') if self.edit_scroll == 1 => {
                    self.edit_draft.edit_field(1, |_| {});
                    let is_stdio = self.edit_draft.kind == "stdio";
                    let new_field_ids: &[usize] = if is_stdio {
                        &[0, 1, 2, 3, 4, 5]
                    } else {
                        &[0, 1, 6, 7]
                    };
                    if !new_field_ids.contains(&self.edit_scroll) {
                        self.edit_scroll = 1;
                    }
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

        // ── List mode: Search query active ─────────────────────────────
        if self.query_active {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.query_active = false;
                    return Some(Action::Noop);
                }
                KeyCode::Backspace => {
                    if !self.query.is_empty() {
                        self.query.pop();
                        self.refilter();
                    }
                    return Some(Action::Noop);
                }
                KeyCode::Char(ch) => {
                    self.query.push(ch);
                    self.refilter();
                    return Some(Action::Noop);
                }
                _ => return None,
            }
        }

        // ── List mode: Normal navigation ───────────────────────────────
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Some(Action::Overlay(OverlayAction::Close(
                OverlayKind::McpServerPanel,
            ))),
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_up(1);
                Some(Action::Noop)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_down(1);
                Some(Action::Noop)
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.scroll_preview_up(3);
                Some(Action::Noop)
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.scroll_preview_down(3);
                Some(Action::Noop)
            }
            KeyCode::PageUp => {
                for _ in 0..5 {
                    self.move_up(1);
                }
                Some(Action::Noop)
            }
            KeyCode::PageDown => {
                for _ in 0..5 {
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
            KeyCode::Char('/') | KeyCode::Char('s') => {
                self.query_active = true;
                Some(Action::Noop)
            }
            _ => None,
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        let position = Position::new(mouse.column, mouse.row);
        if !area.contains(position) {
            return None;
        }

        let overlay_w = (area.width * 92 / 100)
            .clamp(76, 120)
            .min(area.width.saturating_sub(2));
        let overlay_h = (area.height * 85 / 100)
            .clamp(18, 40)
            .min(area.height.saturating_sub(2));
        let overlay = centered_rect(overlay_w, overlay_h, area);
        if !overlay.contains(position) {
            return None;
        }

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        let inner_w = inner.width as usize;
        let left_w = (inner_w * 24 / 100).clamp(22, 28) as u16;
        let in_left = position.x < inner.x + left_w;

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if in_left {
                    self.move_up(3);
                } else {
                    self.scroll_preview_up(3);
                }
                Some(Action::Consumed)
            }
            MouseEventKind::ScrollDown => {
                if in_left {
                    self.move_down(3);
                } else {
                    self.scroll_preview_down(3);
                }
                Some(Action::Consumed)
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if in_left {
                    let list_start_y = inner.y + 4; // Title(1) + Body Top(1) + Filter(1) + Header(1) + Divider(1)
                    if position.y >= list_start_y && position.y < inner.y + inner.height - 1 {
                        let row = (position.y - list_start_y) as usize;
                        let idx = self.list_scroll + row;
                        if idx < self.filtered_count() {
                            self.selected_index = idx;
                            self.preview_scroll = 0;
                        }
                    }
                }
                Some(Action::Noop)
            }
            _ => Some(Action::Noop),
        }
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
            self.draw_editor(frame, area, ctx);
            return;
        }

        // ── Main panel ─────────────────────────────────────────────────
        let palette = ctx.palette;
        let overlay_w = (area.width * 92 / 100)
            .clamp(76, 120)
            .min(area.width.saturating_sub(2));
        let overlay_h = (area.height * 85 / 100)
            .clamp(18, 40)
            .min(area.height.saturating_sub(2));
        let overlay = centered_rect(overlay_w, overlay_h, area);
        frame.render_widget(Clear, overlay);
        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // ── Title Header ──
        let title_text = if self.summaries.is_empty() {
            " MCP Servers ".to_string()
        } else {
            format!(
                " MCP Servers · {}/{} ",
                (self.selected_index + 1).min(self.filtered_count()),
                self.filtered_count()
            )
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                title_text,
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );

        let body = Rect::new(
            inner.x,
            inner.y + 1,
            inner.width,
            inner.height.saturating_sub(2),
        );

        // ── Empty State ──
        if self.summaries.is_empty() {
            let empty_text = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  No MCP servers configured",
                    Style::default().fg(palette.muted),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Press [n] to add a new server",
                    Style::default().fg(palette.text),
                )),
                Line::from(Span::styled(
                    "  Or configure mcpServers in ~/.config/tidev/mcp.json",
                    Style::default().fg(palette.muted),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Press Esc or q to close",
                    Style::default().fg(palette.muted),
                )),
            ];
            frame.render_widget(
                Paragraph::new(empty_text).style(Style::default().bg(palette.panel_alt)),
                body,
            );

            let footer_y = inner.y + inner.height - 1;
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  [n] add server  •  [Esc] close",
                    Style::default().fg(palette.muted),
                )))
                .style(Style::default().bg(palette.panel_alt)),
                Rect::new(inner.x, footer_y, inner.width, 1),
            );
            return;
        }

        // ── Split Layout: Left list + Right preview ──
        let inner_w = body.width as usize;
        let left_w = (inner_w * 24 / 100).clamp(22, 28) as u16;
        let layout = Layout::horizontal([
            Constraint::Length(left_w),
            Constraint::Length(1),
            Constraint::Min(20),
        ])
        .split(body);
        let left_area = layout[0];
        let sep_area = layout[1];
        let right_area = layout[2];

        // Vertical divider between left and right
        let sep_lines: Vec<Line> = (0..body.height)
            .map(|_| Line::from(Span::styled("│", Style::default().fg(palette.border))))
            .collect();
        frame.render_widget(
            Paragraph::new(sep_lines).style(Style::default().bg(palette.panel_alt)),
            sep_area,
        );

        // ── Left Pane ──
        // 1. Filter bar
        let filter_area = Rect::new(left_area.x, left_area.y, left_area.width, 1);
        let (visible_query, cursor) = single_line_input_cursor(filter_area, 10, &self.query);
        let filter_text = if self.query_active {
            format!("  Search: {visible_query}")
        } else if self.query.is_empty() {
            "  Search... (/)".to_string()
        } else {
            format!("  Search: {}", self.query)
        };
        let filter_style = if self.query_active {
            Style::default().fg(palette.accent)
        } else {
            Style::default().fg(palette.muted)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(filter_text, filter_style)]))
                .style(Style::default().bg(palette.panel_alt)),
            filter_area,
        );
        if self.query_active {
            frame.set_cursor_position(cursor);
        }

        // 2. Column header
        let left_header_area = Rect::new(left_area.x, left_area.y + 1, left_area.width, 1);
        let name_col_w = (left_area.width as usize).saturating_sub(11).max(4);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("   ", Style::default().fg(palette.accent)),
                Span::styled(
                    format!("{:<name_col_w$}", "Server"),
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  Kind",
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            ]))
            .style(Style::default().bg(palette.panel_alt)),
            left_header_area,
        );

        // 3. Divider
        let left_divider_area = Rect::new(left_area.x, left_area.y + 2, left_area.width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(left_area.width as usize),
                Style::default().fg(palette.border),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            left_divider_area,
        );

        // 4. Server list
        let list_content_y = left_area.y + 3;
        let list_content_height = left_area.height.saturating_sub(3);
        let list_area = Rect::new(
            left_area.x,
            list_content_y,
            left_area.width,
            list_content_height,
        );

        let (list_content_area, list_sb_area) = if list_area.width > 2 {
            let chunks =
                Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).split(list_area);
            (chunks[0], Some(chunks[1]))
        } else {
            (list_area, None)
        };

        let visible_items = list_content_area.height as usize;
        let total = self.filtered_count();
        let scroll = self.list_scroll;

        let mut list_lines = Vec::new();
        for i in 0..visible_items.min(total.saturating_sub(scroll)) {
            let idx = scroll + i;
            let Some(&summary_idx) = self.filtered_indices.get(idx) else {
                break;
            };
            let Some(summary) = self.summaries.get(summary_idx) else {
                break;
            };

            let is_selected = idx == self.selected_index;
            let (status_icon, status_color) = match &summary.status {
                McpConnectionStatus::Connected => ("●", Color::Green),
                McpConnectionStatus::Connecting => ("◌", Color::Yellow),
                McpConnectionStatus::Disconnected => ("○", palette.muted),
                McpConnectionStatus::Failed(_) => ("✕", Color::Red),
            };

            let max_name_len = (list_content_area.width as usize).saturating_sub(10).max(4);
            let mut display_name = summary.name.clone();
            if display_name.len() > max_name_len {
                display_name.truncate(max_name_len.saturating_sub(1));
                display_name.push('…');
            }

            let row_bg = if is_selected {
                palette.selection_bg
            } else {
                palette.panel_alt
            };
            let text_fg = if is_selected {
                palette.selection_fg
            } else {
                palette.text
            };
            let muted_fg = if is_selected {
                palette.selection_fg
            } else {
                palette.muted
            };

            let line = Line::from(vec![
                Span::styled(
                    format!(" {status_icon} "),
                    Style::default()
                        .fg(if is_selected { text_fg } else { status_color })
                        .bg(row_bg),
                ),
                Span::styled(
                    format!("{display_name:<max_name_len$}"),
                    Style::default()
                        .fg(text_fg)
                        .bg(row_bg)
                        .add_modifier(if is_selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    format!(" {:>4} ", summary.kind),
                    Style::default().fg(muted_fg).bg(row_bg),
                ),
            ]);
            list_lines.push(line);
        }

        while list_lines.len() < visible_items {
            list_lines.push(Line::from(""));
        }

        frame.render_widget(
            Paragraph::new(list_lines).style(Style::default().bg(palette.panel_alt)),
            list_content_area,
        );

        if let Some(sb_area) = list_sb_area
            && total > visible_items
        {
            render_scrollbar(frame, sb_area, scroll, total, palette, false);
        }

        // ── Right Pane: Details ──
        let right_header_area = Rect::new(right_area.x, right_area.y, right_area.width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "  Server Details",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            right_header_area,
        );

        let right_divider_area = Rect::new(right_area.x, right_area.y + 1, right_area.width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(right_area.width as usize),
                Style::default().fg(palette.border),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            right_divider_area,
        );

        let right_content_y = right_area.y + 2;
        let right_content_height = right_area.height.saturating_sub(2);
        let right_content_area = Rect::new(
            right_area.x,
            right_content_y,
            right_area.width,
            right_content_height,
        );

        let (details_area, details_sb_area) = if right_content_area.width > 2 {
            let chunks = Layout::horizontal([Constraint::Min(1), Constraint::Length(1)])
                .split(right_content_area);
            (chunks[0], Some(chunks[1]))
        } else {
            (right_content_area, None)
        };

        if let Some(summary) = self.selected_summary() {
            let config = self.mcp.server_config(&summary.name);
            let tools = self
                .mcp
                .all_definitions()
                .into_iter()
                .filter(|tool| {
                    tool.mcp_target()
                        .is_some_and(|(server, _)| server == summary.name)
                })
                .collect::<Vec<_>>();

            let all_lines = self.generate_details_lines(
                summary,
                config.as_ref(),
                &tools,
                &palette,
                details_area.width as usize,
            );

            let total_detail_lines = all_lines.len();
            let visible_detail_items = details_area.height as usize;
            let max_scroll = total_detail_lines.saturating_sub(visible_detail_items);
            self.preview_scroll = self.preview_scroll.min(max_scroll);
            let scroll = self.preview_scroll;

            let visible_lines: Vec<Line> = all_lines
                .into_iter()
                .skip(scroll)
                .take(visible_detail_items)
                .collect();

            frame.render_widget(
                Paragraph::new(visible_lines).style(Style::default().bg(palette.panel_alt)),
                details_area,
            );

            if let Some(sb_area) = details_sb_area
                && total_detail_lines > visible_detail_items
            {
                render_scrollbar(frame, sb_area, scroll, total_detail_lines, palette, false);
            }
        }

        // ── Footer Toolbar ──
        let footer_y = inner.y + inner.height - 1;
        let footer_text = if self.query_active {
            "  Enter: confirm search  •  Esc: cancel"
        } else if inner.width >= 86 {
            "  [Enter] Toggle  •  [r] Refresh  •  [n] Add  •  [e] Edit  •  [d] Delete  •  [/] Search  •  [Esc] Close"
        } else if inner.width >= 68 {
            "  [Enter] Toggle  [r] Refresh  [n] Add  [e] Edit  [d] Delete  [/] Search  [Esc] Close"
        } else {
            "  [Enter] Toggle  [r] Sync  [n] Add  [e] Edit  [d] Del  [Esc] Close"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                footer_text,
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, footer_y, inner.width, 1),
        );
    }
}

// ── Editor drawing ─────────────────────────────────────────────────────────

impl McpServerPanel {
    fn draw_editor(&mut self, frame: &mut Frame, area: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let is_stdio = self.edit_draft.kind == "stdio";
        let height = if is_stdio { 16u16 } else { 14u16 };
        let editor_area = centered_rect(area.width.min(68), area.height.min(height), area);

        frame.render_widget(Clear, editor_area);
        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, editor_area);

        let inner = editor_area.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // Title
        let title = if let Some(ref orig) = self.edit_original_name {
            format!(" Edit MCP Server: {} ", orig)
        } else {
            " Add MCP Server ".to_string()
        };

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                title,
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );

        // Divider
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(inner.width as usize),
                Style::default().fg(palette.border),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );

        // Active fields based on kind
        let fields: Vec<(&'static str, &str, usize, &'static str)> = if is_stdio {
            vec![
                ("Name", &self.edit_draft.name, 0, ""),
                (
                    "Kind",
                    &self.edit_draft.kind,
                    1,
                    " (Space to cycle: stdio → http → sse)",
                ),
                ("Command", &self.edit_draft.command, 2, ""),
                ("Args", &self.edit_draft.args, 3, " (optional)"),
                (
                    "Cwd",
                    &self.edit_draft.cwd,
                    4,
                    " (optional, relative to workspace)",
                ),
                ("Env", &self.edit_draft.env, 5, " (KEY=VAL, optional)"),
            ]
        } else {
            vec![
                ("Name", &self.edit_draft.name, 0, ""),
                (
                    "Kind",
                    &self.edit_draft.kind,
                    1,
                    " (Space to cycle: stdio → http → sse)",
                ),
                ("URL", &self.edit_draft.url, 6, ""),
                (
                    "Headers",
                    &self.edit_draft.headers,
                    7,
                    " (Header: value, optional)",
                ),
            ]
        };

        let field_start_y = inner.y + 2;
        for (i, (label, value, field_id, hint)) in fields.iter().enumerate() {
            let y = field_start_y + i as u16;
            let is_active = self.edit_scroll == *field_id;

            let label_style = if is_active {
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.muted)
            };

            let field_area = Rect::new(inner.x, y, inner.width, 1);
            let (visible_value, cursor) = single_line_input_cursor(field_area, 12, value);
            let display_value = if is_active {
                visible_value.to_string()
            } else {
                value.to_string()
            };

            let value_style = if is_active {
                Style::default()
                    .fg(palette.selection_fg)
                    .bg(palette.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text)
            };

            let mut spans = vec![
                Span::styled(format!("  {:<8}: ", label), label_style),
                Span::styled(display_value, value_style),
            ];
            if !hint.is_empty() {
                spans.push(Span::styled(*hint, Style::default().fg(palette.muted)));
            }

            frame.render_widget(
                Paragraph::new(Line::from(spans)).style(Style::default().bg(palette.panel_alt)),
                field_area,
            );
            if is_active && *field_id != 1 {
                frame.set_cursor_position(cursor);
            }
        }

        // Error message
        if let Some(ref err) = self.error_message {
            let err_y = field_start_y + fields.len() as u16;
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!("  Error: {err}"),
                    Style::default().fg(Color::Red),
                )))
                .style(Style::default().bg(palette.panel_alt)),
                Rect::new(inner.x, err_y, inner.width, 1),
            );
        }

        // Help text at bottom
        let help_y = inner.y + inner.height - 1;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  [Tab/↑↓] next field  •  [Enter] save  •  [Esc] cancel",
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel_alt)),
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
