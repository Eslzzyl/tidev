//! Component trait — the uniform interface for all UI components.

use anyhow::Result;
use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::action::Action;
use crate::context::{DrawContext, InitContext, UpdateContext};

/// Base trait for all UI components.
pub(crate) trait Component {
    // ── Lifecycle ──

    /// Initialise with immutable resources (config, workspace_root, …).
    fn init(&mut self, ctx: &InitContext) -> Result<()> {
        let _ = ctx;
        Ok(())
    }

    // ── Event handling ──

    /// Handle a keyboard event. Returns `Some(action)` if consumed.
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        let _ = key;
        None
    }

    /// Handle pasted text from bracketed paste (⌘V / Shift+Insert on macOS).
    ///
    /// Only called when this component is the active overlay or the focused
    /// component. Returns `Some(action)` if the paste triggers an action.
    fn handle_paste(&mut self, text: &str) -> Option<Action> {
        let _ = text;
        None
    }

    /// Handle a mouse event. `area` is the component's layout rect.
    #[allow(dead_code)]
    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        let _ = (mouse, area);
        None
    }

    // ── Action processing ──

    /// Process an Action and return follow-up Actions.
    fn update(&mut self, action: &Action, ctx: &UpdateContext) -> Vec<Action> {
        let _ = (action, ctx);
        vec![]
    }

    // ── Rendering ──

    /// Pure render (never modifies external state).
    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext);

    // ── Dirty tracking ──

    /// Whether the component needs re-drawing.
    fn is_dirty(&self) -> bool {
        true
    }

    /// Mark as clean after rendering.
    fn mark_clean(&mut self) {}

    // ── Overlay support ──

    /// Whether this component is an overlay (lives in OverlayStack).
    #[allow(dead_code)]
    fn is_overlay(&self) -> bool {
        false
    }

    /// Z-order (higher = on top).
    #[allow(dead_code)]
    fn z_order(&self) -> u8 {
        0
    }

    /// Whether this blocks input from reaching lower components.
    fn blocks_input(&self) -> bool {
        false
    }

    /// Whether this overlay should be drawn in the main area only
    /// (as opposed to the full terminal area including sidebar).
    #[allow(dead_code)]
    fn overlay_uses_main_area(&self) -> bool {
        false
    }

    /// Whether this component currently owns the terminal cursor.
    ///
    /// This is deliberately separate from keyboard focus: a list or
    /// confirmation dialog can own keyboard input while still requiring the
    /// terminal cursor to remain hidden.
    fn wants_terminal_cursor(&self) -> bool {
        false
    }
}
