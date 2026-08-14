//! QuestionDialog — interactive prompt when the LLM asks questions via the
//! `question` tool. Supports single/multi-select options and custom text input.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};

use anyhow::Result;
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use textwrap::wrap;
use tidev_tools::types::QuestionInfo;

use crate::action::{Action, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::{bottom_centered_rect, wrapped_input_tail};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

pub(crate) struct QuestionDialog {
    /// Index of the current question being answered.
    current_index: usize,
    questions: Vec<QuestionInfo>,
    /// Per-question list of selected answer labels.
    answers: Vec<Vec<String>>,
    /// Per-question cursor position in the options list.
    selected_indices: Vec<usize>,
    /// Per-question custom text input buffer.
    custom_inputs: Vec<String>,
    /// Whether the user is currently editing a custom answer.
    editing_custom: bool,
    /// Whether the dialog was dismissed (Esc) or completed.
    dismissed: bool,
}

impl QuestionDialog {
    pub(crate) fn new(questions: Vec<QuestionInfo>) -> Self {
        let count = questions.len();
        Self {
            current_index: 0,
            questions,
            answers: vec![Vec::new(); count],
            selected_indices: vec![0; count],
            custom_inputs: vec![String::new(); count],
            editing_custom: false,
            dismissed: false,
        }
    }

    // ── Helpers ──

    fn total(&self) -> usize {
        self.questions.len()
    }

    fn is_last(&self) -> bool {
        self.current_index + 1 >= self.total()
    }

    fn current_question(&self) -> Option<&QuestionInfo> {
        self.questions.get(self.current_index)
    }

    fn title(&self) -> String {
        let count = self.total();
        if count == 0 {
            return "Questions".to_string();
        }
        let header = self
            .current_question()
            .and_then(|q| {
                let h = q.header.trim();
                if h.is_empty() { None } else { Some(h) }
            })
            .unwrap_or("Questions");
        format!(
            "Question {} of {} · {}",
            self.current_index + 1,
            count,
            header
        )
    }

    fn body_title(&self) -> String {
        self.current_question()
            .and_then(|q| {
                let qt = q.question.trim();
                if qt.is_empty() { None } else { Some(qt) }
            })
            .unwrap_or("Ask a question")
            .to_string()
    }

    fn current_option_count(&self) -> usize {
        self.current_question()
            .map(|q| q.options.len() + usize::from(q.custom.unwrap_or(true)))
            .unwrap_or(0)
    }

    fn custom_option_index(&self) -> Option<usize> {
        self.current_question()
            .and_then(|q| q.custom.unwrap_or(true).then_some(q.options.len()))
    }

    fn selected_index(&self) -> usize {
        self.selected_indices
            .get(self.current_index)
            .copied()
            .unwrap_or(0)
    }

    fn set_selected_index(&mut self, index: usize) {
        let count = self.current_option_count();
        if count == 0 {
            return;
        }
        if let Some(slot) = self.selected_indices.get_mut(self.current_index) {
            *slot = index.min(count.saturating_sub(1));
        }
    }

    fn move_selection(&mut self, step: isize) {
        let count = self.current_option_count();
        if count == 0 {
            return;
        }
        let current = self.selected_index() as isize;
        let next = (current + step).rem_euclid(count as isize) as usize;
        self.set_selected_index(next);
    }

    fn is_option_selected(&self, option_index: usize) -> bool {
        self.current_question()
            .and_then(|q| q.options.get(option_index))
            .map(|opt| {
                let value = opt.label.trim();
                self.answers
                    .get(self.current_index)
                    .is_some_and(|answers| answers.iter().any(|a| a == value))
            })
            .unwrap_or(false)
    }

    fn current_custom_input(&self) -> &str {
        self.custom_inputs
            .get(self.current_index)
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    fn is_custom_answer_selected(&self) -> bool {
        let value = self.current_custom_input().trim();
        !value.is_empty()
            && self
                .answers
                .get(self.current_index)
                .is_some_and(|answers| answers.iter().any(|a| a == value))
    }

    fn sync_current_custom_input(&mut self, text: &str) {
        let allow_multiple = self
            .current_question()
            .map(|q| q.multiple.unwrap_or(false))
            .unwrap_or(false);
        let previous = self.current_custom_input().trim().to_string();
        let normalized = text.trim().to_string();

        if let Some(slot) = self.custom_inputs.get_mut(self.current_index) {
            *slot = text.to_string();
        }

        if let Some(slot) = self.answers.get_mut(self.current_index) {
            if allow_multiple {
                if !previous.is_empty() {
                    slot.retain(|a| a != &previous);
                }
                if !normalized.is_empty() && !slot.iter().any(|a| a == &normalized) {
                    slot.push(normalized);
                }
            } else {
                *slot = if normalized.is_empty() {
                    Vec::new()
                } else {
                    vec![normalized]
                };
            }
        }
    }

    fn toggle_regular_option(&mut self, option_index: usize) {
        let Some(question) = self.current_question() else {
            return;
        };
        let Some(option) = question.options.get(option_index) else {
            return;
        };

        let value = option.label.trim().to_string();
        let allow_multiple = question.multiple.unwrap_or(false);

        if let Some(slot) = self.answers.get_mut(self.current_index) {
            if allow_multiple {
                if let Some(existing) = slot.iter().position(|a| a == &value) {
                    slot.remove(existing);
                } else {
                    slot.push(value);
                }
            } else {
                *slot = vec![value];
                if let Some(custom) = self.custom_inputs.get_mut(self.current_index) {
                    custom.clear();
                }
            }
        }
    }

    fn toggle_custom_option(&mut self) {
        let (allow_custom, allow_multiple) = self
            .current_question()
            .map(|q| (q.custom.unwrap_or(true), q.multiple.unwrap_or(false)))
            .unwrap_or((false, false));

        if !allow_custom {
            return;
        }

        if allow_multiple && self.is_custom_answer_selected() {
            self.sync_current_custom_input("");
            self.editing_custom = false;
        } else {
            self.editing_custom = true;
        }
    }

    fn move_next(&mut self) {
        if !self.is_last() {
            self.current_index += 1;
            self.editing_custom = false;
        }
    }

    fn move_previous(&mut self) {
        if self.current_index > 0 {
            self.current_index -= 1;
            self.editing_custom = false;
        }
    }

    fn formatted_output(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (index, question) in self.questions.iter().enumerate() {
            let answer = self.answers.get(index).cloned().unwrap_or_default();
            let value = if answer.is_empty() {
                "Unanswered".to_string()
            } else {
                answer.join(", ")
            };
            parts.push(format!("Q{}: {}", index + 1, question.question));
            parts.push(format!("A: {}", value));
        }
        parts.join("\n")
    }

    fn regular_options_lines(&self, width: u16) -> Vec<String> {
        let Some(question) = self.current_question() else {
            return vec!["No questions available.".to_string()];
        };

        let wrap_width = width.max(1) as usize;
        let mut lines = Vec::new();

        if question.options.is_empty() {
            if question.custom.unwrap_or(true) {
                push_wrapped_line(
                    &mut lines,
                    wrap_width,
                    "No predefined options were provided. Type a freeform answer below."
                        .to_string(),
                );
                return lines;
            }
            return vec!["No predefined options were provided.".to_string()];
        }

        push_wrapped_line(
            &mut lines,
            wrap_width,
            format!(
                "{}{}",
                if question.multiple.unwrap_or(false) {
                    "Select one or more options. "
                } else {
                    "Select one option. "
                },
                if question.custom.unwrap_or(true) {
                    "Type your own answer if needed."
                } else {
                    "Type the option number or label."
                }
            ),
        );

        for (index, option) in question.options.iter().enumerate() {
            let selected = self.selected_index() == index;
            let checked = if self.is_option_selected(index) {
                "✓"
            } else {
                " "
            };
            let cursor = if selected { ">" } else { " " };
            push_wrapped_line(
                &mut lines,
                wrap_width,
                format!(
                    "{} {}. [{}] {}",
                    cursor,
                    index + 1,
                    checked,
                    option.label.trim()
                ),
            );
            if let Some(desc) = option
                .description
                .as_deref()
                .filter(|d| !d.trim().is_empty())
            {
                for wrapped in wrap(desc, wrap_width.saturating_sub(4).max(1)) {
                    lines.push(format!("    {}", wrapped));
                }
            }
        }

        lines
    }

    fn custom_option_lines(&self, width: u16) -> Vec<String> {
        let Some(question) = self.current_question() else {
            return Vec::new();
        };
        if !question.custom.unwrap_or(true) {
            return Vec::new();
        }

        let wrap_width = width.max(1) as usize;
        let custom_index = question.options.len();
        let mut lines = Vec::new();
        let selected = self.selected_index() == custom_index;
        let checked = if self.is_custom_answer_selected() {
            "✓"
        } else {
            " "
        };
        let cursor = if selected { ">" } else { " " };
        push_wrapped_line(
            &mut lines,
            wrap_width,
            format!(
                "{} {}. [{}] Type your own answer",
                cursor,
                custom_index + 1,
                checked
            ),
        );
        let custom_input = self.current_custom_input().trim();
        if !custom_input.is_empty() {
            for wrapped in wrap(custom_input, wrap_width.saturating_sub(4).max(1)) {
                lines.push(format!("    {}", wrapped));
            }
        }
        lines
    }

    fn options_lines(&self, width: u16) -> Vec<String> {
        let mut lines = self.regular_options_lines(width);
        lines.extend(self.custom_option_lines(width));
        lines
    }

    fn body_height(&self, width: u16) -> u16 {
        let body = self.body_title();
        if body.is_empty() {
            return 2;
        }
        let wrap_width = width.max(1) as usize;
        let wrapped = wrap(&body, wrap_width);
        wrapped.len().max(2) as u16
    }
}

impl Component for QuestionDialog {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        // Editing mode
        if self.editing_custom {
            match key.code {
                KeyCode::Esc => {
                    self.editing_custom = false;
                    return None;
                }
                KeyCode::Enter
                    if !key.modifiers.contains(KeyModifiers::SHIFT)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    // Save current custom input
                    let current_text = self.current_custom_input().to_string();
                    self.sync_current_custom_input(&current_text);
                    self.editing_custom = false;

                    let allow_multiple = self
                        .current_question()
                        .map(|q| q.multiple.unwrap_or(false))
                        .unwrap_or(false);

                    if !allow_multiple {
                        // Advance to next question or finish
                        if self.is_last() {
                            return Some(Action::Overlay(OverlayAction::Close(
                                OverlayKind::QuestionDialog,
                            )));
                        }
                        self.move_next();
                    }
                    return None;
                }
                KeyCode::Char(c) => {
                    let mut new_text = self.current_custom_input().to_string();
                    new_text.push(c);
                    self.sync_current_custom_input(&new_text);
                    return None;
                }
                KeyCode::Backspace => {
                    let mut new_text = self.current_custom_input().to_string();
                    new_text.pop();
                    self.sync_current_custom_input(&new_text);
                    return None;
                }
                _ => return None,
            }
        }

        // Normal mode
        match key.code {
            KeyCode::Esc => {
                // Dismiss — reject all
                self.dismissed = true;
                Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::QuestionDialog,
                )))
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                None
            }
            KeyCode::Char(' ') => {
                let selected = self.selected_index();
                if let Some(custom_idx) = self.custom_option_index() {
                    if selected == custom_idx {
                        self.toggle_custom_option();
                    } else {
                        self.toggle_regular_option(selected);
                    }
                } else {
                    self.toggle_regular_option(selected);
                }
                None
            }
            KeyCode::Enter => {
                let selected = self.selected_index();
                if let Some(custom_idx) = self.custom_option_index()
                    && selected == custom_idx
                {
                    // Start editing custom answer
                    self.editing_custom = true;
                    return None;
                }
                if let Some(question) = self.current_question() {
                    if question.multiple.unwrap_or(false) {
                        // In multi-select mode, Enter toggles the option
                        self.toggle_regular_option(selected);
                    } else {
                        // In single-select mode, Enter confirms and advances
                        self.toggle_regular_option(selected);
                        if self.is_last() {
                            return Some(Action::Overlay(OverlayAction::Close(
                                OverlayKind::QuestionDialog,
                            )));
                        }
                        self.move_next();
                    }
                }
                None
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_previous();
                None
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.is_last() {
                    return Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::QuestionDialog,
                    )));
                }
                self.move_next();
                None
            }
            KeyCode::Left => {
                self.move_previous();
                None
            }
            KeyCode::Right => {
                if self.is_last() {
                    return Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::QuestionDialog,
                    )));
                }
                self.move_next();
                None
            }
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::QuestionDialog)) => {
                if self.dismissed {
                    vec![Action::QuestionResponse { output: None }]
                } else {
                    let output = self.formatted_output();
                    vec![Action::QuestionResponse {
                        output: Some(output),
                    }]
                }
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        // Dynamic height: body + options + custom_input area + padding
        let inner_w = rect.width.saturating_sub(4).max(20);
        let options_lines = self.options_lines(inner_w);
        let options_height = options_lines.len().max(2) as u16;
        let body_h = self.body_height(inner_w);
        let input_h: u16 = if self.editing_custom { 3 } else { 0 };
        let total_h = body_h
            .saturating_add(options_height)
            .saturating_add(input_h)
            .saturating_add(6); // title + footers + padding
        let overlay = bottom_centered_rect(rect.width, total_h.min(30), rect);
        frame.render_widget(Clear, overlay);

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let sections = if self.editing_custom {
            Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(body_h),
                Constraint::Min(options_height),
                Constraint::Length(input_h),
                Constraint::Length(1),
            ])
            .split(inner)
        } else {
            Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(body_h),
                Constraint::Min(options_height),
                Constraint::Length(1),
            ])
            .split(inner)
        };

        let footer_text = if self.editing_custom {
            "Enter save custom answer · Esc cancel · Ctrl+P/Ctrl+N/←/→ previous/next"
        } else {
            "Enter select · Space toggle · Ctrl+P/Ctrl+N/←/→ previous/next · Esc dismiss"
        };

        // Title
        frame.render_widget(
            Paragraph::new(self.title())
                .alignment(ratatui::layout::Alignment::Center)
                .wrap(Wrap { trim: false })
                .style(
                    Style::default()
                        .bg(palette.panel_alt)
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
            sections[0],
        );

        // Body (question text)
        frame.render_widget(
            Paragraph::new(self.body_title())
                .wrap(Wrap { trim: false })
                .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
            sections[1],
        );

        // Options
        let options_text = self.options_lines(inner_w).join("\n");
        frame.render_widget(
            Paragraph::new(options_text)
                .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
            sections[2],
        );

        // Custom input area (editing mode)
        if self.editing_custom {
            let input_style = Style::default().bg(palette.background).fg(palette.text);
            let (visible_lines, cursor) =
                wrapped_input_tail(self.current_custom_input(), sections[3]);
            frame.render_widget(
                Paragraph::new(visible_lines.join("\n"))
                    .style(input_style)
                    .wrap(Wrap { trim: false }),
                sections[3],
            );
            frame.set_cursor_position(cursor);
        }

        // Footer
        let footer_idx = if self.editing_custom { 4 } else { 3 };
        if let Some(section) = sections.get(footer_idx) {
            frame.render_widget(
                Paragraph::new(footer_text)
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(
                        Style::default()
                            .bg(palette.panel_alt)
                            .fg(palette.accent_soft),
                    ),
                *section,
            );
        }
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn z_order(&self) -> u8 {
        20
    }

    fn blocks_input(&self) -> bool {
        true
    }

    fn overlay_uses_main_area(&self) -> bool {
        true
    }

    fn wants_terminal_cursor(&self) -> bool {
        self.editing_custom
    }
}

fn push_wrapped_line(lines: &mut Vec<String>, wrap_width: usize, line: String) {
    let wrapped = wrap(&line, wrap_width);
    if wrapped.is_empty() {
        lines.push(String::new());
    } else {
        lines.extend(wrapped.into_iter().map(|w| w.into_owned()));
    }
}
