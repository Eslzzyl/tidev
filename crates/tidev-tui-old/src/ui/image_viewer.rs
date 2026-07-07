use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ratatui::{
    layout::{Alignment, Rect},
    prelude::Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};
use ratatui_image::picker::Picker;
use ratatui_image::{Image, protocol::Protocol};

/// Full-screen image viewer overlay.
///
/// Decodes a data-URL image and renders it using the terminal's graphics
/// protocol (Sixel / Kitty / iTerm2). Closes on any key press.
pub(crate) struct ImageViewerState {
    dyn_img: image::DynamicImage,
    filename: String,
    width: u32,
    height: u32,
    cached_protocol: Option<Protocol>,
    cached_area: Option<Rect>,
}

impl ImageViewerState {
    /// Try to create a viewer from a `data:image/...;base64,...` URL.
    pub fn new(_picker: &Picker, data_url: &str, filename: &str) -> Option<Self> {
        let raw_bytes = decode_data_url(data_url)?;
        let dyn_img = image::load_from_memory(&raw_bytes).ok()?;
        let (width, height) = (dyn_img.width(), dyn_img.height());

        Some(Self {
            dyn_img,
            filename: filename.to_string(),
            width,
            height,
            cached_protocol: None,
            cached_area: None,
        })
    }

    /// Render the image viewer overlay.
    pub fn render(&mut self, frame: &mut Frame, area: Rect, picker: &Picker) {
        let palette = crate::theme::ThemeManager::new("dark").palette();

        // Full-screen clear with dark background
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.background)),
            area,
        );

        let padding_x = (area.width / 10).max(2);
        let padding_y = (area.height / 10).max(2);
        let title_row: u16 = 2;
        let hint_row: u16 = 1;

        let img_area = Rect {
            x: area.x + padding_x,
            y: area.y + padding_y + title_row,
            width: area.width.saturating_sub(padding_x * 2),
            height: area
                .height
                .saturating_sub(padding_y * 2 + title_row + hint_row),
        };

        // Title line
        let title = Line::from(vec![
            Span::styled(
                " Image ",
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
            x: area.x + padding_x,
            y: area.y + padding_y,
            width: area.width.saturating_sub(padding_x * 2),
            height: 1,
        };
        frame.render_widget(Paragraph::new(title), title_area);

        // Only re-create protocol when the area changes (e.g. terminal resize)
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
        }

        // Hint line at the bottom
        let hint = Line::from(Span::styled(
            " Press any key to close ",
            Style::default().fg(palette.muted),
        ));
        let hint_area = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(1),
            width: area.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(hint).alignment(Alignment::Center),
            hint_area,
        );
    }
}

/// Decode a `data:image/png;base64,AAAA...` URL into raw bytes.
fn decode_data_url(data_url: &str) -> Option<Vec<u8>> {
    let base64_part = data_url.find("base64,")?;
    let encoded = &data_url[base64_part + 7..];
    BASE64_STANDARD.decode(encoded).ok()
}
