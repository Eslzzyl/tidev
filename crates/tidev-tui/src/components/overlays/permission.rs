//! PermissionDialog — final approve / reject prompt for tool execution.
//!
//! Two-phase interaction:
//! 1. **Select** — user chooses Allow/Deny (Y/N/R/X/Esc).
//! 2. **Input** — if Deny was chosen, an optional reason may be typed.
//!    Enter submits (reason may be empty), Esc returns to Select.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use unicode_width::UnicodeWidthStr;

use crate::action::{Action, OverlayAction, OverlayKind, PermissionDecision};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::{centered_rect, pretty_tool_arguments};

// ---------------------------------------------------------------------------
// Phase
// ---------------------------------------------------------------------------

enum Phase {
    /// Initial mode — user presses a decision key.
    Select,
    /// Reason input mode — user types a reason after pressing N/X.
    Input {
        reason: String,
        /// The decision that triggered input (Deny or DenyAndRemember).
        base_decision: PermissionDecision,
    },
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

pub(crate) struct PermissionDialog {
    display_name: String,
    arguments: String,
    current_index: usize,
    total: usize,
    phase: Phase,
    /// Final decision + optional reason, set before Close.
    decision: Option<PermissionDecision>,
    reason: Option<String>,
}

impl PermissionDialog {
    pub(crate) fn new(
        _permission_key: String,
        display_name: String,
        arguments: String,
        current_index: usize,
        total: usize,
    ) -> Self {
        Self {
            display_name,
            arguments,
            current_index,
            total,
            phase: Phase::Select,
            decision: None,
            reason: None,
        }
    }

    fn title(&self) -> String {
        format!(
            "Approve tool call {} of {} · {}",
            self.current_index, self.total, self.display_name
        )
    }
}

impl Component for PermissionDialog {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        match &self.phase {
            Phase::Select => match key.code {
                // Allow — no reason needed, close immediately.
                KeyCode::Char('y' | 'Y') => {
                    self.decision = Some(PermissionDecision::Allow);
                    self.reason = None;
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::PermissionDialog,
                    )))
                }
                KeyCode::Char('r' | 'R') => {
                    self.decision = Some(PermissionDecision::AllowAndRemember);
                    self.reason = None;
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::PermissionDialog,
                    )))
                }
                // Deny — transition to reason input.
                KeyCode::Char('n' | 'N') => {
                    self.phase = Phase::Input {
                        reason: String::new(),
                        base_decision: PermissionDecision::Deny,
                    };
                    None
                }
                KeyCode::Char('x' | 'X') => {
                    self.phase = Phase::Input {
                        reason: String::new(),
                        base_decision: PermissionDecision::DenyAndRemember,
                    };
                    None
                }
                // Esc in select mode → deny without reason.
                KeyCode::Esc => {
                    self.decision = Some(PermissionDecision::Deny);
                    self.reason = None;
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::PermissionDialog,
                    )))
                }
                _ => None,
            },
            Phase::Input { reason, .. } => {
                let mut reason = reason.clone();
                match key.code {
                    KeyCode::Enter
                        if !key.modifiers.contains(KeyModifiers::SHIFT)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        // Submit: use the base decision + reason.
                        match &self.phase {
                            Phase::Input { base_decision, .. } => {
                                let final_reason = reason.trim().to_string();
                                self.decision = Some(base_decision.clone());
                                self.reason = if final_reason.is_empty() {
                                    None
                                } else {
                                    Some(final_reason)
                                };
                            }
                            _ => unreachable!(),
                        }
                        Some(Action::Overlay(OverlayAction::Close(
                            OverlayKind::PermissionDialog,
                        )))
                    }
                    KeyCode::Esc => {
                        // Cancel: return to select mode, discard typed reason.
                        self.phase = Phase::Select;
                        self.decision = None;
                        self.reason = None;
                        None
                    }
                    KeyCode::Backspace => {
                        reason.pop();
                        self.phase = Phase::Input {
                            reason,
                            base_decision: match &self.phase {
                                Phase::Input { base_decision, .. } => base_decision.clone(),
                                _ => unreachable!(),
                            },
                        };
                        None
                    }
                    KeyCode::Char(c) => {
                        reason.push(c);
                        self.phase = Phase::Input {
                            reason,
                            base_decision: match &self.phase {
                                Phase::Input { base_decision, .. } => base_decision.clone(),
                                _ => unreachable!(),
                            },
                        };
                        None
                    }
                    _ => None,
                }
            }
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::PermissionDialog)) => {
                if let Some(decision) = self.decision.take() {
                    let reason = self.reason.take();
                    vec![Action::PermissionResponse { decision, reason }]
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let preview = pretty_tool_arguments(&self.arguments);
        let preview_height = preview.lines().count().min(8) as u16;

        match &self.phase {
            Phase::Select => {
                let overlay = centered_rect(rect.width, preview_height.saturating_add(10), rect);
                frame.render_widget(Clear, overlay);

                let block = Block::default().style(Style::default().bg(palette.panel_alt));
                frame.render_widget(block, overlay);

                let inner = overlay.inner(Margin {
                    horizontal: 1,
                    vertical: 1,
                });

                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Min(4),
                    Constraint::Length(2),
                ])
                .split(inner);

                // Title bar
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        " Tool approval ",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                // Title
                frame.render_widget(
                    Paragraph::new(self.title())
                        .alignment(ratatui::layout::Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.text)
                                .add_modifier(Modifier::BOLD),
                        ),
                    sections[1],
                );

                // Warning text
                frame.render_widget(
                    Paragraph::new(
                        "This tool can change state. Review the arguments and choose whether to allow it.",
                    )
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                    sections[2],
                );

                // Tool arguments preview
                frame.render_widget(
                    Paragraph::new(preview)
                        .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                        .wrap(Wrap { trim: false }),
                    sections[3],
                );

                // Help / action hints
                frame.render_widget(
                    Paragraph::new(
                        "Y allow · N deny (with reason) · R allow and remember · X deny and remember · Esc",
                    )
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(
                        Style::default()
                            .bg(palette.panel_alt)
                            .fg(palette.accent_soft),
                    ),
                    sections[4],
                );
            }
            Phase::Input { reason, .. } => {
                let overlay = centered_rect(rect.width, preview_height.saturating_add(10), rect);
                frame.render_widget(Clear, overlay);

                let block = Block::default().style(Style::default().bg(palette.panel_alt));
                frame.render_widget(block, overlay);

                let inner = overlay.inner(Margin {
                    horizontal: 1,
                    vertical: 1,
                });

                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Length(2),
                ])
                .split(inner);

                // Title bar
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        " Reason (optional) ",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                // Title
                frame.render_widget(
                    Paragraph::new(self.title())
                        .alignment(ratatui::layout::Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.text)
                                .add_modifier(Modifier::BOLD),
                        ),
                    sections[1],
                );

                // Input field
                let input_style = Style::default().bg(palette.background).fg(palette.text);
                frame.render_widget(
                    Paragraph::new(reason.clone())
                        .style(input_style)
                        .wrap(Wrap { trim: false }),
                    sections[3],
                );
                let text_w = UnicodeWidthStr::width(reason.as_str()) as u16;
                let col = text_w % sections[3].width;
                let row = text_w / sections[3].width;
                frame.set_cursor_position((sections[3].x + col, sections[3].y + row));

                // Help
                frame.render_widget(
                    Paragraph::new("Enter confirm · Esc cancel")
                        .alignment(ratatui::layout::Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.accent_soft),
                        ),
                    sections[4],
                );
            }
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
}
