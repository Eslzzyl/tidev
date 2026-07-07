//! WorkspaceBoundaryDialog — security prompt when a tool tries to access a
//! path outside the workspace root.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use anyhow::Result;

use crate::action::{Action, BoundaryDecision, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::centered_rect;

// ---------------------------------------------------------------------------
// Phase
// ---------------------------------------------------------------------------

enum WbPhase {
    /// Main decision dialog.
    Main,
    /// Confirm "until-exit" decision.
    Confirm { action: BoundaryDecision },
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
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::WorkspaceBoundaryDialog,
                    )))
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    self.decision = Some(BoundaryDecision::DenyOnce);
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::WorkspaceBoundaryDialog,
                    )))
                }
                KeyCode::Char('a' | 'A') => {
                    self.phase = WbPhase::Confirm {
                        action: BoundaryDecision::AllowUntilExit,
                    };
                    self.selected = 0;
                    None
                }
                KeyCode::Char('d' | 'D') => {
                    self.phase = WbPhase::Confirm {
                        action: BoundaryDecision::DenyUntilExit,
                    };
                    self.selected = 0;
                    None
                }
                _ => None,
            },
            WbPhase::Confirm { action } => match key.code {
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
                        self.decision = Some(action.clone());
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
            },
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::WorkspaceBoundaryDialog)) => {
                if let Some(decision) = self.decision.take() {
                    vec![Action::WorkspaceBoundaryResponse {
                        path: self.requested_path.clone(),
                        decision,
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
        let overlay = centered_rect(60, 10, rect);
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
                    Paragraph::new(
                        "A tool is trying to access a path outside the workspace:",
                    )
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
                        "Y allow once · A allow until exit · N deny once · D deny until exit · Esc deny once",
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
            WbPhase::Confirm { action } => {
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
                    Paragraph::new(format!("This will {} access to:", action_text))
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

                // Options row
                let options = [" Confirm ", " Cancel "];
                let option_spans: Vec<Span> = options
                    .iter()
                    .enumerate()
                    .flat_map(|(i, label)| {
                        let is_sel = i == self.selected;
                        let span = if is_sel {
                            Span::styled(
                                format!("[{}]", label),
                                Style::default()
                                    .fg(palette.accent)
                                    .add_modifier(Modifier::BOLD),
                            )
                        } else {
                            Span::styled(
                                format!(" {} ", label),
                                Style::default().fg(palette.text),
                            )
                        };
                        vec![span, Span::raw("  ")]
                    })
                    .collect();

                frame.render_widget(
                    Paragraph::new(Line::from(option_spans))
                        .alignment(ratatui::layout::Alignment::Center)
                        .style(Style::default().bg(palette.panel_alt)),
                    sections[3],
                );

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
