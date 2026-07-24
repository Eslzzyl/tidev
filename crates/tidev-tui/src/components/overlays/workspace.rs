//! WorkspaceBoundaryDialog — security prompt when a tool tries to access a
//! path outside the workspace root.
//!
//! Three-phase interaction:
//! 1. **Main** — user presses a decision key (Y/A/N/D/Esc).
//! 2. **Input** — if N or D was pressed, user types an optional reason.
//!    Enter submits (reason may be empty), Esc returns to Main.
//! 3. **Confirm** — if an "until-exit" decision was made, user confirms.

use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use unicode_width::UnicodeWidthStr;

use crate::action::{Action, BoundaryDecision, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::bottom_centered_rect;

// ---------------------------------------------------------------------------
// Phase
// ---------------------------------------------------------------------------

enum WbPhase {
    /// Main decision dialog.
    Main,
    /// User is typing a reason after pressing N (DenyOnce) or D (DenyUntilExit).
    Input {
        reason: String,
        base_decision: BoundaryDecision,
    },
    /// Confirm "until-exit" decision.
    Confirm {
        action: BoundaryDecision,
        reason: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

pub(crate) struct WorkspaceBoundaryDialog {
    requested_path: PathBuf,
    workspace_root: PathBuf,
    current_index: usize,
    total: usize,
    phase: WbPhase,
    /// Which button is selected in confirm mode (0 = Confirm, 1 = Cancel).
    selected: usize,
    /// The decision made by the user (set in handle_key_event before Close).
    decision: Option<BoundaryDecision>,
    /// Optional reason attached by the user.
    reason: Option<String>,
}

impl WorkspaceBoundaryDialog {
    pub(crate) fn new(
        requested_path: PathBuf,
        workspace_root: PathBuf,
        current_index: usize,
        total: usize,
    ) -> Self {
        Self {
            requested_path,
            workspace_root,
            current_index,
            total,
            phase: WbPhase::Main,
            selected: 0,
            decision: None,
            reason: None,
        }
    }

    fn title(&self) -> String {
        format!("Security Warning {} of {}", self.current_index, self.total)
    }

    fn path_display(&self) -> String {
        self.requested_path.display().to_string()
    }

    fn workspace_display(&self) -> String {
        self.workspace_root.display().to_string()
    }
}

impl Component for WorkspaceBoundaryDialog {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        match &self.phase {
            WbPhase::Main => match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.decision = Some(BoundaryDecision::AllowOnce);
                    self.reason = None;
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::WorkspaceBoundaryDialog,
                    )))
                }
                KeyCode::Char('a' | 'A') => {
                    self.phase = WbPhase::Confirm {
                        action: BoundaryDecision::AllowUntilExit,
                        reason: None,
                    };
                    self.selected = 0;
                    None
                }
                // N → enter reason input for DenyOnce
                KeyCode::Char('n' | 'N') => {
                    self.phase = WbPhase::Input {
                        reason: String::new(),
                        base_decision: BoundaryDecision::DenyOnce,
                    };
                    None
                }
                // D → enter reason input for DenyUntilExit
                KeyCode::Char('d' | 'D') => {
                    self.phase = WbPhase::Input {
                        reason: String::new(),
                        base_decision: BoundaryDecision::DenyUntilExit,
                    };
                    None
                }
                // Esc → deny once, no reason
                KeyCode::Esc => {
                    self.decision = Some(BoundaryDecision::DenyOnce);
                    self.reason = None;
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::WorkspaceBoundaryDialog,
                    )))
                }
                _ => None,
            },
            WbPhase::Input {
                reason,
                base_decision,
            } => {
                let mut reason = reason.clone();
                let base_decision = base_decision.clone();
                match key.code {
                    KeyCode::Enter
                        if !key.modifiers.contains(KeyModifiers::SHIFT)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        let final_reason = reason.trim().to_string();
                        let reason = if final_reason.is_empty() {
                            None
                        } else {
                            Some(final_reason)
                        };
                        match base_decision {
                            BoundaryDecision::DenyOnce => {
                                self.decision = Some(BoundaryDecision::DenyOnce);
                                self.reason = reason;
                                Some(Action::Overlay(OverlayAction::Close(
                                    OverlayKind::WorkspaceBoundaryDialog,
                                )))
                            }
                            BoundaryDecision::DenyUntilExit => {
                                self.phase = WbPhase::Confirm {
                                    action: BoundaryDecision::DenyUntilExit,
                                    reason,
                                };
                                self.selected = 0;
                                None
                            }
                            _ => {
                                self.phase = WbPhase::Main;
                                None
                            }
                        }
                    }
                    KeyCode::Esc => {
                        self.phase = WbPhase::Main;
                        self.decision = None;
                        self.reason = None;
                        None
                    }
                    KeyCode::Backspace => {
                        reason.pop();
                        self.phase = WbPhase::Input {
                            reason,
                            base_decision,
                        };
                        None
                    }
                    KeyCode::Char(c) => {
                        reason.push(c);
                        self.phase = WbPhase::Input {
                            reason,
                            base_decision,
                        };
                        None
                    }
                    _ => None,
                }
            }
            WbPhase::Confirm { action, reason } => {
                let action = action.clone();
                let reason = reason.clone();
                match key.code {
                    KeyCode::Left => {
                        self.selected = self.selected.saturating_sub(1);
                        None
                    }
                    KeyCode::Right => {
                        self.selected = self.selected.saturating_add(1).min(1);
                        None
                    }
                    KeyCode::Enter => {
                        if self.selected == 0 {
                            // Confirm — close and emit decision
                            self.decision = Some(action);
                            self.reason = reason;
                            Some(Action::Overlay(OverlayAction::Close(
                                OverlayKind::WorkspaceBoundaryDialog,
                            )))
                        } else {
                            // Cancel — back to main
                            self.phase = WbPhase::Main;
                            self.selected = 0;
                            None
                        }
                    }
                    KeyCode::Esc => {
                        // Back to main
                        self.phase = WbPhase::Main;
                        self.selected = 0;
                        None
                    }
                    _ => None,
                }
            }
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::WorkspaceBoundaryDialog)) => {
                if let Some(decision) = self.decision.take() {
                    let reason = self.reason.take();
                    vec![Action::WorkspaceBoundaryResponse {
                        path: self.requested_path.clone(),
                        decision,
                        reason,
                    }]
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let overlay = bottom_centered_rect(rect.width, 10, rect);
        frame.render_widget(Clear, overlay);

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        match &self.phase {
            WbPhase::Main => {
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(1),
                ])
                .split(inner);

                // Title
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        self.title(),
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                // Message
                frame.render_widget(
                    Paragraph::new("A tool is trying to access a path outside the workspace:")
                        .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
                    sections[1],
                );

                // Path info
                let path_text = format!(
                    "Requested: {}\nWorkspace: {}",
                    self.path_display(),
                    self.workspace_display()
                );
                frame.render_widget(
                    Paragraph::new(path_text).style(
                        Style::default()
                            .bg(palette.panel_alt)
                            .fg(palette.accent_soft),
                    ),
                    sections[2],
                );

                // Help
                frame.render_widget(
                    Paragraph::new(
                        "Y allow once · A allow until exit · N deny once (with reason) · D deny until exit · Esc deny once",
                    )
                    .style(
                        Style::default()
                            .bg(palette.panel_alt)
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    sections[3],
                );
            }
            WbPhase::Input { reason, .. } => {
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Length(1),
                ])
                .split(inner);

                // Title
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        self.title(),
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                // Prompt
                frame.render_widget(
                    Paragraph::new("Enter a reason for denying (optional):")
                        .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
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
                    Paragraph::new("Enter confirm · Esc cancel").style(
                        Style::default()
                            .bg(palette.panel_alt)
                            .fg(palette.accent_soft),
                    ),
                    sections[4],
                );
            }
            WbPhase::Confirm { action, .. } => {
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .split(inner);

                // Title
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        self.title(),
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                // Confirmation message
                let action_text = if *action == BoundaryDecision::AllowUntilExit {
                    "allow"
                } else {
                    "deny"
                };
                frame.render_widget(
                    Paragraph::new(format!(
                        "Are you sure you want to {action_text} this path until exit?"
                    ))
                    .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
                    sections[1],
                );

                // Path info
                frame.render_widget(
                    Paragraph::new(self.path_display()).style(
                        Style::default()
                            .bg(palette.panel_alt)
                            .fg(palette.accent_soft),
                    ),
                    sections[2],
                );

                // Buttons
                let buttons =
                    Layout::horizontal([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
                        .split(sections[3]);

                let confirm_style = if self.selected == 0 {
                    Style::default()
                        .fg(palette.panel)
                        .bg(palette.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.text).bg(palette.panel_alt)
                };
                let cancel_style = if self.selected == 1 {
                    Style::default()
                        .fg(palette.panel)
                        .bg(palette.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.text).bg(palette.panel_alt)
                };

                frame.render_widget(Paragraph::new(" Confirm ").style(confirm_style), buttons[0]);
                frame.render_widget(Paragraph::new(" Cancel ").style(cancel_style), buttons[1]);

                // Help
                frame.render_widget(
                    Paragraph::new("← → switch · Enter confirm · Esc cancel").style(
                        Style::default()
                            .bg(palette.panel_alt)
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
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
