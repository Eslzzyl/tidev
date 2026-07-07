//! SensitiveFileDialog — security prompt when a tool tries to read a file
//! listed in `.tidev/sensitive.txt`.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use anyhow::Result;

use crate::action::{Action, OverlayAction, OverlayKind, SensitiveFileDecision};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::centered_rect;

// ---------------------------------------------------------------------------
// Phase
// ---------------------------------------------------------------------------

enum SfPhase {
    Main,
    Confirm { action: SensitiveFileDecision },
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

pub(crate) struct SensitiveFileDialog {
    sensitive_path: PathBuf,
    workspace_root: PathBuf,
    current_index: usize,
    total: usize,
    phase: SfPhase,
    selected: usize,
    decision: Option<SensitiveFileDecision>,
}

impl SensitiveFileDialog {
    pub(crate) fn new(
        sensitive_path: PathBuf,
        workspace_root: PathBuf,
        current_index: usize,
        total: usize,
    ) -> Self {
        Self {
            sensitive_path,
            workspace_root,
            current_index,
            total,
            phase: SfPhase::Main,
            selected: 0,
            decision: None,
        }
    }

    fn title(&self) -> String {
        format!(
            "Sensitive File Warning {} of {}",
            self.current_index, self.total
        )
    }

    fn path_display(&self) -> String {
        self.sensitive_path.display().to_string()
    }

    fn workspace_display(&self) -> String {
        self.workspace_root.display().to_string()
    }
}

impl Component for SensitiveFileDialog {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        match &self.phase {
            SfPhase::Main => match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.decision = Some(SensitiveFileDecision::AllowOnce);
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::SensitiveFileDialog,
                    )))
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    self.decision = Some(SensitiveFileDecision::DenyOnce);
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::SensitiveFileDialog,
                    )))
                }
                KeyCode::Char('a' | 'A') => {
                    self.phase = SfPhase::Confirm {
                        action: SensitiveFileDecision::AllowUntilExit,
                    };
                    self.selected = 0;
                    None
                }
                KeyCode::Char('d' | 'D') => {
                    self.phase = SfPhase::Confirm {
                        action: SensitiveFileDecision::DenyUntilExit,
                    };
                    self.selected = 0;
                    None
                }
                _ => None,
            },
            SfPhase::Confirm { action } => match key.code {
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
                        self.decision = Some(action.clone());
                        Some(Action::Overlay(OverlayAction::Close(
                            OverlayKind::SensitiveFileDialog,
                        )))
                    } else {
                        self.phase = SfPhase::Main;
                        self.selected = 0;
                        None
                    }
                }
                KeyCode::Esc => {
                    self.phase = SfPhase::Main;
                    self.selected = 0;
                    None
                }
                _ => None,
            },
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::SensitiveFileDialog)) => {
                if let Some(decision) = self.decision.take() {
                    vec![Action::SensitiveFileResponse {
                        path: self.sensitive_path.clone(),
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
            SfPhase::Main => {
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
                    Paragraph::new("A tool is trying to read a sensitive file:")
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
            SfPhase::Confirm { action } => {
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
                let action_text = if *action == SensitiveFileDecision::AllowUntilExit {
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
