//! New-architecture App root component.
//!
//! Owns the Runtime, manages the component tree via OverlayStack,
//! routes Actions, and dispatches async commands.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use tidev_tui::theme::{ThemeName, ThemePalette};

use crate::action::{Action, ChatAction, OverlayAction, OverlayKind, ThemeAction};
use crate::component::Component;
use crate::components::overlay_stack::OverlayStack;
use crate::components::overlays::agents::AgentsPanel;
use crate::components::overlays::skills::{SkillItem, SkillsPanel};
use crate::components::overlays::theme::ThemePanel;
use crate::context::{DrawContext, UpdateContext};

pub struct App {
    runtime: tidev_core::Runtime,
    overlays: OverlayStack,
    current_palette: ThemePalette,
    should_quit: bool,
}

impl App {
    pub fn new(runtime: tidev_core::Runtime) -> Self {
        let theme_str = runtime.config().theme;
        let current_palette = ThemePalette::from_name(&theme_str);
        Self {
            runtime,
            overlays: OverlayStack::new(),
            current_palette,
            should_quit: false,
        }
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    // ── Event handling ──

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
                Action::Chat(ChatAction::SetInput(text)) => {
                    // TODO: route to Composer once migrated
                    log::info!("SetInput: {}", text);
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
    }
}
