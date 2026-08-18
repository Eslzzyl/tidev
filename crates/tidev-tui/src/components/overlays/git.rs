//! Git workspace panel.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use tidev_core::{GitChangeKind, GitDiffScope, GitHistoryPage, GitStatusSnapshot};

use crate::action::{Action, GitAction, GitQueryKind, GitTab, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::diff_render::render_unified_diff_text;
use crate::utils::{centered_rect, shorten};

const PANEL_WIDTH: u16 = 132;
const PANEL_HEIGHT: u16 = 38;
const FILE_LIST_WIDTH: u16 = 34;

pub(crate) struct GitPanel {
    tab: GitTab,
    status: Option<GitStatusSnapshot>,
    history: Vec<tidev_core::GitCommitSummary>,
    history_head: Option<String>,
    history_has_more: bool,
    history_skip: usize,
    history_selected: usize,
    diff: Option<tidev_core::GitDiffSnapshot>,
    diff_scope: Option<GitDiffScope>,
    status_selected: usize,
    diff_scroll: u16,
    loading: Option<(u64, GitQueryKind)>,
    error: Option<String>,
}

impl GitPanel {
    pub(crate) fn new() -> Self {
        Self {
            tab: GitTab::Status,
            status: None,
            history: Vec::new(),
            history_head: None,
            history_has_more: false,
            history_skip: 0,
            history_selected: 0,
            diff: None,
            diff_scope: None,
            status_selected: 0,
            diff_scroll: 0,
            loading: None,
            error: None,
        }
    }

    fn move_status(&mut self, delta: isize) {
        let len = self.status.as_ref().map_or(0, |status| status.files.len());
        self.status_selected = move_index(self.status_selected, len, delta);
    }

    fn move_history(&mut self, delta: isize) -> Option<Action> {
        let len = self.history.len();
        if len == 0 {
            return None;
        }
        let old = self.history_selected;
        self.history_selected = move_index(old, len, delta);
        if delta > 0
            && old == len.saturating_sub(1)
            && self.history_has_more
            && self.loading.is_none()
        {
            let skip = self.history.len();
            self.history_skip = skip;
            return Some(Action::Git(GitAction::LoadHistory {
                head: self.history_head.clone(),
                skip,
            }));
        }
        if self.history_selected != old {
            self.diff_scroll = 0;
            return self.selected_commit_diff_action();
        }
        None
    }

    fn selected_commit(&self) -> Option<String> {
        self.history
            .get(self.history_selected)
            .map(|commit| commit.id.clone())
    }

    fn selected_commit_diff_action(&self) -> Option<Action> {
        self.selected_commit().map(|commit| {
            Action::Git(GitAction::LoadDiff {
                scope: GitDiffScope::Commit(commit),
            })
        })
    }

    fn switch_tab_action(&self, tab: GitTab) -> Action {
        Action::Git(GitAction::SwitchTab(tab))
    }

    fn tab_title(tab: GitTab) -> &'static str {
        match tab {
            GitTab::Status => "Status",
            GitTab::History => "History",
        }
    }

    fn selected_style(palette: crate::theme::ThemePalette) -> Style {
        Style::default()
            .fg(palette.selection_fg)
            .bg(palette.selection_bg)
    }

    fn change_marker(kind: GitChangeKind) -> (&'static str, bool) {
        match kind {
            GitChangeKind::Added => ("A", true),
            GitChangeKind::Deleted => ("D", false),
            GitChangeKind::Modified => ("M", true),
            GitChangeKind::Renamed => ("R", true),
            GitChangeKind::Copied => ("C", true),
            GitChangeKind::Conflicted => ("U", false),
            GitChangeKind::Untracked => ("?", true),
            GitChangeKind::TypeChanged => ("T", true),
            GitChangeKind::Unknown => (" ", true),
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let tabs = [GitTab::Status, GitTab::History];
        let mut spans = Vec::new();
        for (index, tab) in tabs.into_iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("  "));
            }
            let style = if tab == self.tab {
                Self::selected_style(palette).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.muted)
            };
            spans.push(Span::styled(
                format!(" {} {} ", index + 1, Self::tab_title(tab)),
                style,
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn draw_status(&self, frame: &mut Frame, left: Rect, right: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let Some(status) = &self.status else {
            self.draw_message(frame, right, "Loading workspace status...", palette.muted);
            return;
        };

        let mut left_lines = Vec::new();
        for (index, file) in status.files.iter().enumerate() {
            let (marker, positive) = Self::change_marker(file.kind);
            let selected = index == self.status_selected;
            let mut line = Line::from(vec![
                Span::styled(
                    format!("{marker} "),
                    if selected {
                        Self::selected_style(palette)
                    } else {
                        Style::default().fg(if positive {
                            palette.diff_add
                        } else {
                            palette.error
                        })
                    },
                ),
                Span::styled(
                    shorten(&file.path, left.width.saturating_sub(4) as usize),
                    if selected {
                        Self::selected_style(palette)
                    } else {
                        Style::default().fg(palette.text)
                    },
                ),
            ]);
            if selected {
                line = line.style(Self::selected_style(palette));
            }
            left_lines.push(line);
        }
        if left_lines.is_empty() {
            left_lines.push(Line::from(Span::styled(
                "Clean working tree",
                Style::default().fg(palette.success),
            )));
        }
        frame.render_widget(Paragraph::new(left_lines), left);

        let branch = status
            .repo
            .branch
            .as_deref()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "(detached HEAD)".to_string());
        let ahead_behind = match (status.repo.ahead, status.repo.behind) {
            (Some(ahead), Some(behind)) => format!("  ↑{ahead} ↓{behind}"),
            _ => String::new(),
        };
        let summary = vec![
            Line::from(vec![
                Span::styled("Branch  ", Style::default().fg(palette.accent)),
                Span::styled(
                    branch,
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(ahead_behind, Style::default().fg(palette.muted)),
            ]),
            Line::from(vec![
                Span::styled("Changes ", Style::default().fg(palette.accent)),
                Span::styled(
                    format!(
                        "{} files · {} staged · {} modified · {} untracked · {} conflicts",
                        status.files.len(),
                        status.counts.staged,
                        status.counts.unstaged,
                        status.counts.untracked,
                        status.counts.conflicted,
                    ),
                    Style::default().fg(palette.text),
                ),
            ]),
        ];
        let right_chunks =
            Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(right);
        self.render_lines(frame, right_chunks[0], summary, palette);

        let Some(file) = status.files.get(self.status_selected) else {
            self.draw_message(
                frame,
                right_chunks[1],
                "Working tree is clean",
                palette.success,
            );
            return;
        };
        let Some(diff) = &self.diff else {
            self.draw_message(
                frame,
                right_chunks[1],
                "Loading worktree diff...",
                palette.muted,
            );
            return;
        };
        if !matches!(&self.diff_scope, Some(GitDiffScope::Worktree)) {
            self.draw_message(
                frame,
                right_chunks[1],
                "Loading worktree diff...",
                palette.muted,
            );
            return;
        }
        let Some(diff_file) = diff
            .files
            .iter()
            .find(|diff_file| diff_file.path == file.path)
        else {
            self.draw_message(
                frame,
                right_chunks[1],
                "No textual diff for selected file",
                palette.muted,
            );
            return;
        };
        self.draw_diff_patch(
            frame,
            right_chunks[1],
            &diff_file.patch,
            diff_file.binary,
            ctx,
        );
    }

    fn draw_history(&self, frame: &mut Frame, left: Rect, right: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let mut left_lines = Vec::new();
        for (index, commit) in self.history.iter().enumerate() {
            let selected = index == self.history_selected;
            let subject = shorten(&commit.subject, left.width.saturating_sub(9) as usize);
            let mut line = Line::from(vec![
                Span::styled(
                    format!("{} ", commit.short_id),
                    if selected {
                        Self::selected_style(palette)
                    } else {
                        Style::default().fg(palette.accent_soft)
                    },
                ),
                Span::styled(
                    subject,
                    if selected {
                        Self::selected_style(palette)
                    } else {
                        Style::default().fg(palette.text)
                    },
                ),
            ]);
            if selected {
                line = line.style(Self::selected_style(palette));
            }
            left_lines.push(line);
        }
        if left_lines.is_empty() {
            left_lines.push(Line::from(Span::styled(
                "No commits",
                Style::default().fg(palette.muted),
            )));
        }
        frame.render_widget(Paragraph::new(left_lines), left);

        let Some(commit) = self.history.get(self.history_selected) else {
            self.draw_message(frame, right, "Select a commit", palette.muted);
            return;
        };

        let right_chunks =
            Layout::vertical([Constraint::Length(5), Constraint::Min(1)]).split(right);
        let mut summary = vec![
            Line::from(Span::styled(
                commit.subject.clone(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::styled("commit ", Style::default().fg(palette.accent)),
                Span::styled(commit.id.clone(), Style::default().fg(palette.muted)),
            ]),
            Line::from(vec![
                Span::styled("author ", Style::default().fg(palette.accent)),
                Span::styled(
                    format!("{} <{}>", commit.author, commit.author_email),
                    Style::default().fg(palette.muted),
                ),
            ]),
            Line::from(vec![
                Span::styled("date   ", Style::default().fg(palette.accent)),
                Span::styled(
                    commit.authored_at.clone(),
                    Style::default().fg(palette.muted),
                ),
            ]),
        ];
        if !commit.refs.is_empty() {
            summary.push(Line::from(vec![
                Span::styled("refs   ", Style::default().fg(palette.accent)),
                Span::styled(commit.refs.join(", "), Style::default().fg(palette.muted)),
            ]));
        }
        self.render_lines(frame, right_chunks[0], summary, palette);

        let Some(diff) = &self.diff else {
            self.draw_message(
                frame,
                right_chunks[1],
                "Loading commit diff...",
                palette.muted,
            );
            return;
        };
        if !matches!(&self.diff_scope, Some(GitDiffScope::Commit(id)) if id == &commit.id) {
            self.draw_message(
                frame,
                right_chunks[1],
                "Loading commit diff...",
                palette.muted,
            );
            return;
        }
        self.draw_diff_patch(
            frame,
            right_chunks[1],
            &diff.patch,
            diff.files.iter().any(|file| file.binary),
            ctx,
        );
    }

    fn draw_diff_patch(
        &self,
        frame: &mut Frame,
        area: Rect,
        patch: &str,
        binary: bool,
        ctx: &DrawContext,
    ) {
        let palette = ctx.palette;
        if binary {
            self.draw_message(frame, area, "Binary file changed", palette.warning);
            return;
        }
        if patch.is_empty() {
            self.draw_message(frame, area, "No diff", palette.muted);
            return;
        }
        let rendered = render_unified_diff_text(patch, area.width as usize, palette, 4)
            .map(|(lines, _)| lines)
            .unwrap_or_else(|| {
                vec![Line::from(Span::styled(
                    "Unable to render diff",
                    Style::default().fg(palette.error),
                ))]
            });
        let max_scroll = rendered.len().saturating_sub(area.height as usize) as u16;
        let scroll = self.diff_scroll.min(max_scroll);
        frame.render_widget(
            Paragraph::new(rendered)
                .style(Style::default().bg(palette.panel_alt))
                .scroll((scroll, 0)),
            area,
        );
    }

    fn render_lines(
        &self,
        frame: &mut Frame,
        area: Rect,
        lines: Vec<Line<'static>>,
        palette: crate::theme::ThemePalette,
    ) {
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().fg(palette.text).bg(palette.panel_alt)),
            area,
        );
    }

    fn draw_message(
        &self,
        frame: &mut Frame,
        area: Rect,
        message: &str,
        color: ratatui::style::Color,
    ) {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                message,
                Style::default().fg(color),
            ))),
            area,
        );
    }
}

impl Component for GitPanel {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                Some(Action::Overlay(OverlayAction::Close(OverlayKind::GitPanel)))
            }
            KeyCode::Char('1') => Some(self.switch_tab_action(GitTab::Status)),
            KeyCode::Char('2') => Some(self.switch_tab_action(GitTab::History)),
            KeyCode::Tab => {
                let tab = match self.tab {
                    GitTab::Status => GitTab::History,
                    GitTab::History => GitTab::Status,
                };
                Some(self.switch_tab_action(tab))
            }
            KeyCode::BackTab => {
                let tab = match self.tab {
                    GitTab::Status => GitTab::History,
                    GitTab::History => GitTab::Status,
                };
                Some(self.switch_tab_action(tab))
            }
            KeyCode::Char('r') if key.modifiers.is_empty() => Some(Action::Git(GitAction::Refresh)),
            KeyCode::Up | KeyCode::Char('k') => match self.tab {
                GitTab::Status => {
                    self.move_status(-1);
                    self.diff_scroll = 0;
                    None
                }
                GitTab::History => self.move_history(-1),
            },
            KeyCode::Down | KeyCode::Char('j') => match self.tab {
                GitTab::Status => {
                    self.move_status(1);
                    self.diff_scroll = 0;
                    None
                }
                GitTab::History => self.move_history(1),
            },
            KeyCode::PageUp => {
                if self.tab == GitTab::Status || self.tab == GitTab::History {
                    if self.tab == GitTab::History {
                        let old = self.history_selected;
                        self.history_selected = self.history_selected.saturating_sub(10);
                        if self.history_selected != old {
                            self.diff_scroll = 0;
                            return self.selected_commit_diff_action();
                        }
                    } else {
                        self.diff_scroll = self.diff_scroll.saturating_sub(10);
                    }
                }
                None
            }
            KeyCode::PageDown => {
                if self.tab == GitTab::Status {
                    self.diff_scroll = self.diff_scroll.saturating_add(10);
                } else if self.tab == GitTab::History {
                    let old = self.history_selected;
                    self.history_selected =
                        (self.history_selected + 10).min(self.history.len().saturating_sub(1));
                    if self.history_selected != old {
                        self.diff_scroll = 0;
                        return self.selected_commit_diff_action();
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        let overlay = centered_rect(PANEL_WIDTH, PANEL_HEIGHT, area);
        if !overlay.contains(Position::new(mouse.column, mouse.row)) {
            return None;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if self.tab == GitTab::Status || self.tab == GitTab::History {
                    self.diff_scroll = self.diff_scroll.saturating_sub(3);
                }
                Some(Action::Consumed)
            }
            MouseEventKind::ScrollDown => {
                if self.tab == GitTab::Status || self.tab == GitTab::History {
                    self.diff_scroll = self.diff_scroll.saturating_add(3);
                }
                Some(Action::Consumed)
            }
            _ => Some(Action::Noop),
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        let Action::Git(action) = action else {
            return Vec::new();
        };
        match action {
            GitAction::Refresh => {
                self.tab = GitTab::Status;
                self.status = None;
                self.history.clear();
                self.history_head = None;
                self.history_has_more = false;
                self.history_skip = 0;
                self.history_selected = 0;
                self.diff = None;
                self.diff_scope = None;
                self.status_selected = 0;
                self.diff_scroll = 0;
                self.error = None;
            }
            GitAction::SwitchTab(tab) => {
                self.tab = *tab;
                self.error = None;
                match tab {
                    GitTab::Status if self.status.is_none() => {
                        return vec![Action::Git(GitAction::Refresh)];
                    }
                    GitTab::Status => {
                        return vec![Action::Git(GitAction::LoadDiff {
                            scope: GitDiffScope::Worktree,
                        })];
                    }
                    GitTab::History if self.history.is_empty() => {
                        return vec![Action::Git(GitAction::LoadHistory {
                            head: self.history_head.clone(),
                            skip: 0,
                        })];
                    }
                    GitTab::History => {
                        return self.selected_commit_diff_action().into_iter().collect();
                    }
                }
            }
            GitAction::LoadHistory { head, skip } => {
                self.tab = GitTab::History;
                self.history_head = head.clone();
                self.history_skip = *skip;
                if *skip == 0 {
                    self.history.clear();
                    self.history_selected = 0;
                    self.diff = None;
                    self.diff_scope = None;
                    self.diff_scroll = 0;
                }
                self.error = None;
            }
            GitAction::LoadDiff { scope } => {
                self.diff = None;
                self.diff_scope = Some(scope.clone());
                self.diff_scroll = 0;
                self.error = None;
            }
            GitAction::Loading { request_id, query } => {
                self.loading = Some((*request_id, *query));
                self.error = None;
            }
            GitAction::StatusReady { request_id, result } => {
                if self.loading != Some((*request_id, GitQueryKind::Status)) {
                    return Vec::new();
                }
                self.loading = None;
                match result {
                    Ok(status) => {
                        self.status = Some(status.clone());
                        if self.status_selected >= status.files.len() {
                            self.status_selected = status.files.len().saturating_sub(1);
                        }
                        if self.tab == GitTab::Status {
                            return vec![Action::Git(GitAction::LoadDiff {
                                scope: GitDiffScope::Worktree,
                            })];
                        }
                    }
                    Err(error) => self.error = Some(error.clone()),
                }
            }
            GitAction::HistoryReady { request_id, result } => {
                if self.loading != Some((*request_id, GitQueryKind::History)) {
                    return Vec::new();
                }
                self.loading = None;
                match result {
                    Ok(page) => {
                        self.apply_history_page(page);
                        if self.tab == GitTab::History {
                            return self.selected_commit_diff_action().into_iter().collect();
                        }
                    }
                    Err(error) => self.error = Some(error.clone()),
                }
            }
            GitAction::DiffReady { request_id, result } => {
                if self.loading != Some((*request_id, GitQueryKind::Diff)) {
                    return Vec::new();
                }
                self.loading = None;
                match result {
                    Ok(diff) => self.diff = Some(diff.clone()),
                    Err(error) => self.error = Some(error.clone()),
                }
            }
        }
        Vec::new()
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let overlay = centered_rect(PANEL_WIDTH, PANEL_HEIGHT, rect);
        frame.render_widget(Clear, overlay);
        frame.render_widget(
            Block::default()
                .title(" Git ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.border))
                .style(Style::default().bg(palette.panel_alt)),
            overlay,
        );

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);
        self.draw_header(frame, chunks[0], ctx);
        frame.render_widget(
            Paragraph::new(Line::from("─".repeat(chunks[1].width as usize)))
                .style(Style::default().fg(palette.border)),
            chunks[1],
        );

        let body = Layout::horizontal([Constraint::Length(FILE_LIST_WIDTH), Constraint::Min(1)])
            .split(chunks[2]);
        match self.tab {
            GitTab::Status => self.draw_status(frame, body[0], body[1], ctx),
            GitTab::History => self.draw_history(frame, body[0], body[1], ctx),
        }
        let footer = if let Some((_, query)) = self.loading {
            format!(
                "Loading {:?}...  ·  Tab switch  ·  r refresh  ·  Esc close",
                query
            )
        } else if let Some(error) = &self.error {
            format!("Error: {error}")
        } else {
            "1/2 tabs  ·  ↑/↓ select  ·  PgUp/PgDown scroll  ·  r refresh  ·  Esc close".to_string()
        };
        let footer_color = if self.error.is_some() {
            palette.error
        } else {
            palette.muted
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                footer,
                Style::default().fg(footer_color),
            ))),
            chunks[3],
        );
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn z_order(&self) -> u8 {
        30
    }

    fn blocks_input(&self) -> bool {
        true
    }
}

impl GitPanel {
    fn apply_history_page(&mut self, page: &GitHistoryPage) {
        if self.history_skip == 0 {
            self.history = page.commits.clone();
            self.history_selected = 0;
        } else {
            self.history.extend(page.commits.clone());
        }
        self.history_head = page.head.clone();
        self.history_has_more = page.has_more;
        if self.history_selected >= self.history.len() {
            self.history_selected = self.history.len().saturating_sub(1);
        }
    }
}

fn move_index(index: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (index as isize + delta).rem_euclid(len as isize) as usize
}
