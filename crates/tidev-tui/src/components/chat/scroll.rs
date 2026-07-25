use ratatui::layout::Rect;

use crate::components::chat::MessageList;
use crate::components::chat::render as render_mod;

/// State for an in-progress scrollbar drag interaction.
#[derive(Clone, Debug)]
pub(crate) struct ScrollbarDrag {
    /// Scroll offset when the drag started.
    start_scroll: usize,
    /// Mouse Y position (screen coordinate) when the drag started.
    start_mouse_y: u16,
    /// Maximum scroll value at drag start.
    max_scroll: usize,
}

impl MessageList {
    /// Start a scrollbar drag: call on mouse down on scrollbar.
    /// Jumps to the clicked position first (click-to-jump), then starts drag tracking.
    pub fn start_scrollbar_drag(&mut self, mouse_y: u16) {
        let Some(sb_area) = self.scrollbar_area() else {
            return;
        };
        let max_scroll = self.max_scroll();
        if max_scroll == 0 {
            return;
        }
        // Click-to-jump: map click position to scroll offset
        let track_height = sb_area.height as usize;
        let click_y = mouse_y.saturating_sub(sb_area.y) as f32;
        let target_scroll =
            ((click_y / track_height.max(1) as f32) * max_scroll as f32).round() as usize;
        self.scroll_offset = target_scroll.min(max_scroll);
        self.follow_tail = self.scroll_offset >= max_scroll;
        self.dirty = true;
        // Start drag tracking for subsequent drag events
        self.scrollbar_drag = Some(ScrollbarDrag {
            start_scroll: self.scroll_offset,
            start_mouse_y: mouse_y,
            max_scroll,
        });
    }

    /// Continue a scrollbar drag: call on mouse drag.
    pub fn continue_scrollbar_drag(&mut self, mouse_y: u16) {
        let Some(ref drag) = self.scrollbar_drag else {
            return;
        };
        let track_height = self.content_area.map_or(1, |a| a.height as usize);
        if track_height == 0 {
            return;
        }
        let delta_y = mouse_y as isize - drag.start_mouse_y as isize;
        let scroll_delta = (delta_y as f32 / track_height as f32) * drag.max_scroll as f32;
        let new_scroll = (drag.start_scroll as isize + scroll_delta.round() as isize)
            .max(0)
            .min(drag.max_scroll as isize) as usize;
        self.scroll_offset = new_scroll;
        self.follow_tail = self.scroll_offset >= drag.max_scroll;
    }

    /// End a scrollbar drag.
    pub fn end_scrollbar_drag(&mut self) {
        self.scrollbar_drag = None;
    }

    /// Whether a scrollbar drag is in progress.
    pub fn is_scrollbar_dragging(&self) -> bool {
        self.scrollbar_drag.is_some()
    }

    /// Return the scrollbar area, if visible.
    pub fn scrollbar_area(&self) -> Option<Rect> {
        self.scrollbar_rect
    }

    /// Set whether the mouse is hovering over the scrollbar.
    pub fn set_scrollbar_hovered(&mut self, hovered: bool) {
        if self.scrollbar_hovered != hovered {
            self.scrollbar_hovered = hovered;
            self.dirty = true;
        }
    }

    /// Maximum scroll offset.
    pub fn max_scroll(&self) -> usize {
        self.layout_index
            .total_lines
            .saturating_sub(self.content_area.map_or(0, |a| a.height as usize))
    }
}

/// Compute the scrollbar rect from the full chat area.
/// Mirrors the scrollbar positioning logic in `render::compute_content_layout`.
pub(super) fn compute_scrollbar_rect(rect: Rect) -> Option<Rect> {
    if rect.width <= render_mod::LEFT_MARGIN + render_mod::SCROLLBAR_WIDTH {
        return None;
    }
    Some(Rect {
        x: rect.x + rect.width - render_mod::SCROLLBAR_WIDTH,
        y: rect.y,
        width: render_mod::SCROLLBAR_WIDTH,
        height: rect.height,
    })
}
