// The codex renderer uses a dedicated wrapping layer. TiDev keeps the module
// boundary in place so the markdown pipeline can grow into the full adaptive
// wrap implementation without changing call sites later.

use ratatui::text::Line;
use ratatui::text::Span;

pub struct RtOptions<'a> {
    pub width: usize,
    pub initial_indent: Span<'a>,
    pub subsequent_indent: Span<'a>,
}

impl<'a> RtOptions<'a> {
    pub fn new(width: usize) -> Self {
        Self {
            width,
            initial_indent: Span::from(""),
            subsequent_indent: Span::from(""),
        }
    }

    pub fn initial_indent(mut self, initial_indent: Span<'a>) -> Self {
        self.initial_indent = initial_indent;
        self
    }

    pub fn subsequent_indent(mut self, subsequent_indent: Span<'a>) -> Self {
        self.subsequent_indent = subsequent_indent;
        self
    }
}

pub fn adaptive_wrap_line(line: &Line<'_>, _base: RtOptions<'_>) -> Vec<Line<'static>> {
    vec![crate::render::line_utils::line_to_static(line)]
}
