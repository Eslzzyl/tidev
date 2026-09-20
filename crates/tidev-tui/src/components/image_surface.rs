//! Coordinates native terminal image placements with Ratatui's cell buffer.
//!
//! Ratatui can diff character cells, but native image protocols keep a second
//! display surface in the terminal.  This module keeps the two surfaces in
//! step by collecting image candidates during the normal render pass and
//! committing them after overlays have been drawn.

use std::cell::RefCell;
use std::sync::Arc;

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Rect, Size};
use ratatui::prelude::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::sliced::{SignedPosition, SlicedImage, SlicedProtocol};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ImagePlacement {
    id: u64,
    area: Rect,
    protocol: ProtocolType,
}

struct FormulaCandidate {
    id: u64,
    protocol: Arc<SlicedProtocol>,
    protocol_type: ProtocolType,
    render_area: Rect,
    position: SignedPosition,
    visible_area: Rect,
    underlay: Vec<(u16, u16, Cell)>,
}

/// Per-terminal image state shared by the chat renderer and overlays.
pub(crate) struct ImageSurface {
    protocol: Option<ProtocolType>,
    is_tmux: bool,
    overlay_active: bool,
    current: RefCell<Vec<ImagePlacement>>,
    previous: Vec<ImagePlacement>,
    candidates: RefCell<Vec<FormulaCandidate>>,
}

impl ImageSurface {
    pub(crate) fn new(picker: Option<&Picker>) -> Self {
        Self {
            protocol: picker.map(Picker::protocol_type),
            is_tmux: std::env::var_os("TMUX").is_some(),
            overlay_active: false,
            current: RefCell::new(Vec::new()),
            previous: Vec::new(),
            candidates: RefCell::new(Vec::new()),
        }
    }

    /// Start collecting the placements for one complete frame.
    ///
    /// `late_cell_overlay` is true when a widget will paint cells after chat
    /// rendering and before native images are committed.  Formula candidates
    /// retain their original cells in that case so the deferred pass can
    /// suppress any placement touched by the later widget.
    pub(crate) fn begin_frame(&mut self, late_cell_overlay: bool) {
        self.overlay_active = late_cell_overlay;
        self.current.get_mut().clear();
        self.candidates.get_mut().clear();
    }

    /// Save a formula for the post-overlay image pass.
    pub(crate) fn register_formula(
        &self,
        buffer: &Buffer,
        render_area: Rect,
        position: SignedPosition,
        protocol: Arc<SlicedProtocol>,
    ) {
        let Some(visible_area) = image_area(render_area, position, protocol.size()) else {
            return;
        };

        let underlay = if self.overlay_active {
            let mut underlay = Vec::with_capacity(
                usize::from(visible_area.width) * usize::from(visible_area.height),
            );
            for y in visible_area.top()..visible_area.bottom() {
                for x in visible_area.left()..visible_area.right() {
                    if let Some(cell) = buffer.cell((x, y)) {
                        underlay.push((x, y, cell.clone()));
                    }
                }
            }
            underlay
        } else {
            Vec::new()
        };

        self.candidates.borrow_mut().push(FormulaCandidate {
            id: Arc::as_ptr(&protocol) as usize as u64,
            protocol_type: sliced_protocol_type(&protocol),
            protocol,
            render_area,
            position,
            visible_area,
            underlay,
        });
    }

    /// Record a native image rendered directly by an overlay.
    pub(crate) fn register_protocol(&self, protocol: &Protocol, area: Rect) {
        let Some(protocol_type) = self.protocol else {
            return;
        };
        let size = protocol.size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let placement_area = Rect::new(area.x, area.y, size.width, size.height);
        self.register_placement(ImagePlacement {
            id: protocol as *const Protocol as usize as u64,
            area: placement_area,
            protocol: protocol_type,
        });
    }

    /// Draw formulas after all overlays, suppressing any placement whose
    /// underlying cells were touched by an overlay or another image.
    pub(crate) fn render_formulas(&self, frame: &mut Frame) {
        let candidates = self.candidates.borrow();
        for candidate in candidates.iter() {
            if self.overlay_active
                && candidate.underlay.iter().any(|(x, y, expected)| {
                    frame
                        .buffer_mut()
                        .cell((*x, *y))
                        .is_some_and(|actual| actual != expected)
                })
            {
                continue;
            }

            frame.render_widget(
                SlicedImage::new(candidate.protocol.as_ref(), candidate.position),
                candidate.render_area,
            );
            self.register_placement(ImagePlacement {
                id: candidate.id,
                area: candidate.visible_area,
                protocol: candidate.protocol_type,
            });
        }
    }

    fn register_placement(&self, placement: ImagePlacement) {
        if placement.area.width == 0 || placement.area.height == 0 {
            return;
        }
        let mut current = self.current.borrow_mut();
        if !current.contains(&placement) {
            current.push(placement);
        }
    }

    /// Produce protocol cleanup bytes before Ratatui flushes the current cell
    /// buffer.  The current placements are retained for the next diff pass.
    pub(crate) fn finish_frame(&mut self) -> Vec<u8> {
        let current = std::mem::take(self.current.get_mut());
        let stale: Vec<ImagePlacement> = self
            .previous
            .iter()
            .filter(|old| !current.contains(old))
            .cloned()
            .collect();

        let mut cleanup = String::new();
        let has_stale_kitty = stale
            .iter()
            .any(|placement| placement.protocol == ProtocolType::Kitty);
        let has_current_kitty = current
            .iter()
            .any(|placement| placement.protocol == ProtocolType::Kitty);

        // Kitty virtual placements disappear when their placeholders leave
        // the cell buffer.  When the frame contains no remaining Kitty image,
        // deleting all placements also recovers from a terminal that retained
        // a stale placement after a skipped diff.
        if has_stale_kitty && !has_current_kitty {
            append_kitty_delete_all(&mut cleanup, self.is_tmux);
        }

        for placement in &stale {
            if matches!(
                placement.protocol,
                ProtocolType::Sixel | ProtocolType::Iterm2
            ) {
                append_clear_area(&mut cleanup, placement.area, self.is_tmux);
            }
        }

        self.previous = current;
        cleanup.into_bytes()
    }
}

fn image_area(render_area: Rect, position: SignedPosition, size: Size) -> Option<Rect> {
    let left = i32::from(render_area.x) + i32::from(position.x);
    let top = i32::from(render_area.y) + i32::from(position.y);
    let right = left + i32::from(size.width);
    let bottom = top + i32::from(size.height);

    let clipped_left = left.max(i32::from(render_area.left()));
    let clipped_top = top.max(i32::from(render_area.top()));
    let clipped_right = right.min(i32::from(render_area.right()));
    let clipped_bottom = bottom.min(i32::from(render_area.bottom()));
    if clipped_left >= clipped_right || clipped_top >= clipped_bottom {
        return None;
    }

    Some(Rect::new(
        clipped_left as u16,
        clipped_top as u16,
        (clipped_right - clipped_left) as u16,
        (clipped_bottom - clipped_top) as u16,
    ))
}

fn sliced_protocol_type(protocol: &SlicedProtocol) -> ProtocolType {
    match protocol {
        SlicedProtocol::Sliced(_) => ProtocolType::Iterm2,
        SlicedProtocol::Kitty(_) => ProtocolType::Kitty,
        SlicedProtocol::Sixel(_) => ProtocolType::Sixel,
        SlicedProtocol::Halfblocks(_) => ProtocolType::Halfblocks,
    }
}

fn append_clear_area(output: &mut String, area: Rect, is_tmux: bool) {
    let (start, escape, end) = tmux_escape_parts(is_tmux);
    output.push_str(start);
    output.push_str(&format!("{escape}[{};{}H", area.y + 1, area.x + 1));
    if area.height == 1 {
        output.push_str(&format!("{escape}[{}X", area.width));
    } else {
        for _ in 0..area.height {
            output.push_str(&format!("{escape}[{}X{escape}[1B", area.width));
        }
        output.push_str(&format!("{escape}[{}A", area.height));
    }
    output.push_str(end);
}

fn append_kitty_delete_all(output: &mut String, is_tmux: bool) {
    let (start, escape, end) = tmux_escape_parts(is_tmux);
    output.push_str(start);
    output.push_str(&format!("{escape}_Ga=d,d=A{escape}\\"));
    output.push_str(end);
}

fn tmux_escape_parts(is_tmux: bool) -> (&'static str, &'static str, &'static str) {
    if is_tmux {
        ("\x1bPtmux;", "\x1b\x1b", "\x1b\\")
    } else {
        ("", "\x1b", "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_area_clips_to_render_area() {
        let area = image_area(
            Rect::new(10, 20, 8, 5),
            SignedPosition { x: -2, y: -1 },
            Size::new(6, 4),
        )
        .unwrap();
        assert_eq!(area, Rect::new(10, 20, 4, 3));
    }

    #[test]
    fn clear_area_uses_absolute_terminal_coordinates() {
        let mut output = String::new();
        append_clear_area(&mut output, Rect::new(3, 4, 5, 2), false);
        assert_eq!(output, "\x1b[5;4H\x1b[5X\x1b[1B\x1b[5X\x1b[1B\x1b[2A");
    }

    #[test]
    fn kitty_delete_all_is_tmux_wrapped() {
        let mut output = String::new();
        append_kitty_delete_all(&mut output, true);
        assert_eq!(output, "\x1bPtmux;\x1b\x1b_Ga=d,d=A\x1b\x1b\\\x1b\\");
    }

    #[test]
    fn stale_native_placement_is_cleared_on_next_frame() {
        let mut surface = ImageSurface {
            protocol: Some(ProtocolType::Sixel),
            is_tmux: false,
            overlay_active: false,
            current: RefCell::new(Vec::new()),
            previous: Vec::new(),
            candidates: RefCell::new(Vec::new()),
        };
        let placement = ImagePlacement {
            id: 1,
            area: Rect::new(2, 3, 4, 2),
            protocol: ProtocolType::Sixel,
        };

        surface.begin_frame(false);
        surface.register_placement(placement);
        assert!(surface.finish_frame().is_empty());

        surface.begin_frame(false);
        let cleanup = surface.finish_frame();
        assert!(!cleanup.is_empty());
        assert!(String::from_utf8(cleanup).unwrap().contains("\x1b[4;3H"));
    }

    #[test]
    fn unchanged_native_placement_is_not_repainted_or_cleared() {
        let mut surface = ImageSurface {
            protocol: Some(ProtocolType::Iterm2),
            is_tmux: false,
            overlay_active: false,
            current: RefCell::new(Vec::new()),
            previous: Vec::new(),
            candidates: RefCell::new(Vec::new()),
        };
        let placement = ImagePlacement {
            id: 7,
            area: Rect::new(1, 1, 2, 1),
            protocol: ProtocolType::Iterm2,
        };

        surface.begin_frame(false);
        surface.register_placement(placement.clone());
        surface.finish_frame();

        surface.begin_frame(false);
        surface.register_placement(placement);
        assert!(surface.finish_frame().is_empty());
    }
}
