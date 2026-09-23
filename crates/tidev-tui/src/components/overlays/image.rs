//! ImageViewer — full-screen image viewer overlay.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui_image::picker::Picker;
use ratatui_image::{Image, protocol::Protocol};

use crate::action::{Action, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext};
use crate::i18n::TextKey;

pub(crate) struct ImageViewer {
    dyn_img: image::DynamicImage,
    filename: String,
    width: u32,
    height: u32,
    picker: Option<Picker>,
    cached_protocol: Option<Protocol>,
    cached_area: Option<Rect>,
}

impl ImageViewer {
    pub(crate) fn from_raw(
        data: Vec<u8>,
        filename: String,
        picker: Option<Picker>,
    ) -> Option<Self> {
        let dyn_img = image::load_from_memory(&data).ok()?;
        let (width, height) = (dyn_img.width(), dyn_img.height());
        Some(Self {
            dyn_img,
            filename,
            width,
            height,
            picker,
            cached_protocol: None,
            cached_area: None,
        })
    }

    fn close_action() -> Action {
        Action::Overlay(OverlayAction::Close(OverlayKind::ImageViewer {
            data: Vec::new(),
            filename: String::new(),
        }))
    }
}

impl Component for ImageViewer {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    /// Any key closes the image viewer.
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }
        match key.code {
            KeyCode::Char(_) | KeyCode::Esc | KeyCode::Enter => Some(Self::close_action()),
            _ => None,
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent, _area: Rect) -> Option<Action> {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => Some(Self::close_action()),
            _ => None,
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;

        // Use the cached picker; fall back to a text placeholder.
        let Some(picker) = &self.picker else {
            let placeholder = Paragraph::new(Line::from(vec![
                Span::styled(
                    ctx.ui_text.text(TextKey::ImageLabel),
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(&self.filename),
                Span::raw(format!(" [{}x{}]", self.width, self.height)),
            ]))
            .style(Style::default().bg(palette.background));
            frame.render_widget(Clear, rect);
            frame.render_widget(placeholder, rect);
            return;
        };

        // Full-screen clear
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.background)),
            rect,
        );

        let padding_x = (rect.width / 10).max(2);
        let padding_y = (rect.height / 10).max(2);
        let title_row: u16 = 2;
        let hint_row: u16 = 1;

        let img_area = Rect {
            x: rect.x + padding_x,
            y: rect.y + padding_y + title_row,
            width: rect.width.saturating_sub(padding_x * 2),
            height: rect
                .height
                .saturating_sub(padding_y * 2 + title_row + hint_row),
        };

        // Title line
        let title = Line::from(vec![
            Span::styled(
                format!(" {} ", ctx.ui_text.text(TextKey::ImagePreviewTitle)),
                Style::default()
                    .fg(palette.selection_fg)
                    .bg(palette.selection_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {} ", self.filename),
                Style::default().fg(palette.text),
            ),
            Span::styled(
                format!("[{}x{}]", self.width, self.height),
                Style::default().fg(palette.muted),
            ),
        ]);
        let title_area = Rect {
            x: rect.x + padding_x,
            y: rect.y + padding_y,
            width: rect.width.saturating_sub(padding_x * 2),
            height: 1,
        };
        frame.render_widget(Paragraph::new(title), title_area);

        // Re-create protocol only when area changes
        if img_area.width > 0 && img_area.height > 0 && self.cached_area != Some(img_area) {
            self.cached_protocol = picker
                .new_protocol(
                    self.dyn_img.clone(),
                    (img_area.width, img_area.height).into(),
                    ratatui_image::Resize::Fit(None),
                )
                .ok();
            self.cached_area = Some(img_area);
        }

        if let Some(protocol) = &self.cached_protocol {
            let image_widget = Image::new(protocol);
            frame.render_widget(image_widget, img_area);
            if protocol.needs_placeholder(img_area).is_none()
                && let Some(image_surface) = ctx.image_surface
            {
                image_surface.register_protocol(protocol, img_area);
            }
        }

        // Hint at the bottom
        let hint = Line::from(Span::styled(
            format!(" {} ", ctx.ui_text.text(TextKey::ImageCloseHint)),
            Style::default().fg(palette.muted),
        ));
        let hint_area = Rect {
            x: rect.x,
            y: rect.y + rect.height.saturating_sub(1),
            width: rect.width,
            height: 1,
        };
        frame.render_widget(Paragraph::new(hint).alignment(Alignment::Center), hint_area);
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn z_order(&self) -> u8 {
        10
    }

    fn blocks_input(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyModifiers, MouseEvent};
    use ratatui::layout::Rect;

    use super::ImageViewer;
    use crate::action::{Action, OverlayAction, OverlayKind};
    use crate::component::Component;

    fn test_viewer() -> ImageViewer {
        ImageViewer {
            dyn_img: image::DynamicImage::new_rgb8(1, 1),
            filename: "test.png".to_owned(),
            width: 1,
            height: 1,
            picker: None,
            cached_protocol: None,
            cached_area: None,
        }
    }

    fn mouse_event(kind: crossterm::event::MouseEventKind) -> MouseEvent {
        MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn left_click_closes_image_viewer() {
        let mut viewer = test_viewer();
        let action = viewer.handle_mouse_event(
            mouse_event(crossterm::event::MouseEventKind::Down(
                crossterm::event::MouseButton::Left,
            )),
            Rect::default(),
        );

        assert!(matches!(
            action,
            Some(Action::Overlay(OverlayAction::Close(
                OverlayKind::ImageViewer { .. }
            )))
        ));
    }

    #[test]
    fn other_mouse_events_leave_image_viewer_open() {
        let mut viewer = test_viewer();

        for kind in [
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Right),
            crossterm::event::MouseEventKind::ScrollDown,
        ] {
            assert!(
                viewer
                    .handle_mouse_event(mouse_event(kind), Rect::default())
                    .is_none()
            );
        }
    }
}
