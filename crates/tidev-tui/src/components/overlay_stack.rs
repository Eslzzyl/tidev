//! OverlayStack — a z-ordered stack of overlay components.

use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::action::Action;
use crate::component::Component;
use crate::context::{DrawContext, UpdateContext};

pub(crate) struct OverlayStack {
    overlays: Vec<Box<dyn Component>>,
}

impl OverlayStack {
    pub fn new() -> Self {
        Self {
            overlays: Vec::new(),
        }
    }

    pub fn push(&mut self, component: Box<dyn Component>) {
        self.overlays.push(component);
    }

    pub fn last_mut(&mut self) -> Option<&mut Box<dyn Component>> {
        self.overlays.last_mut()
    }

    pub fn pop(&mut self) -> Option<Box<dyn Component>> {
        self.overlays.pop()
    }

    pub fn is_empty(&self) -> bool {
        self.overlays.is_empty()
    }

    /// Route a key event top-first.
    pub fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        for overlay in self.overlays.iter_mut().rev() {
            if let Some(action) = overlay.handle_key_event(key) {
                return Some(action);
            }
            if overlay.blocks_input() {
                return Some(Action::Noop);
            }
        }
        None
    }

    /// Broadcast an Action to all overlays.
    pub fn update_all(&mut self, action: &Action, ctx: &UpdateContext) -> Vec<Action> {
        let mut follow_ups = Vec::new();
        for overlay in self.overlays.iter_mut() {
            follow_ups.extend(overlay.update(action, ctx));
        }
        follow_ups
    }

    /// Route a mouse event top-first.
    pub fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        for overlay in self.overlays.iter_mut().rev() {
            if let Some(action) = overlay.handle_mouse_event(mouse, area) {
                return Some(action);
            }
            if overlay.blocks_input() {
                return Some(Action::Noop);
            }
        }
        None
    }

    /// Draw overlays bottom-up.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &DrawContext) {
        for overlay in self.overlays.iter_mut() {
            overlay.draw(frame, area, ctx);
        }
    }
}
