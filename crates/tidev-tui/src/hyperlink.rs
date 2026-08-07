//! Semantic terminal hyperlinks carried separately from visible TUI text.
//!
//! Layout code measures and wraps ordinary ratatui lines. Hyperlink annotations
//! are applied only when text reaches the frame buffer so OSC 8 bytes never
//! affect geometry. Ported from Codex's `terminal_hyperlinks` module and
//! adapted to tidev's pre-wrapped chat rendering (one logical line per row).

use std::ops::Range;

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;
use url::Url;

/// A hyperlink spanning display columns of a single rendered line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HyperlinkRange {
    /// Display-column range within the line (start inclusive, end exclusive).
    pub(crate) columns: Range<usize>,
    pub(crate) destination: String,
}

impl HyperlinkRange {
    pub(crate) fn web(columns: Range<usize>, destination: String) -> Self {
        Self {
            columns,
            destination,
        }
    }
}

/// A rendered line plus the hyperlinks attached to it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct HyperlinkLine {
    pub(crate) line: Line<'static>,
    pub(crate) hyperlinks: Vec<HyperlinkRange>,
}

impl HyperlinkLine {
    pub(crate) fn new(line: Line<'static>) -> Self {
        Self {
            line,
            hyperlinks: Vec::new(),
        }
    }

    pub(crate) fn width(&self) -> usize {
        self.line.width()
    }

    /// Push a span, recording a hyperlink over it when `destination` is a
    /// web destination (http/https with a host).
    pub(crate) fn push_span(&mut self, span: Span<'static>, destination: Option<&str>) {
        let start = self.width();
        let end = start + UnicodeWidthStr::width(span.content.as_ref());
        self.line.push_span(span);
        if end > start
            && let Some(destination) = destination.and_then(web_destination)
        {
            self.hyperlinks
                .push(HyperlinkRange::web(start..end, destination));
        }
    }

    pub(crate) fn style(mut self, style: ratatui::style::Style) -> Self {
        self.line = self.line.style(style);
        self
    }
}

impl From<Line<'static>> for HyperlinkLine {
    fn from(line: Line<'static>) -> Self {
        Self::new(line)
    }
}

impl From<&'static str> for HyperlinkLine {
    fn from(text: &'static str) -> Self {
        Self::new(Line::from(text))
    }
}

impl From<String> for HyperlinkLine {
    fn from(text: String) -> Self {
        Self::new(Line::from(text))
    }
}

pub(crate) fn plain_hyperlink_lines(lines: Vec<Line<'static>>) -> Vec<HyperlinkLine> {
    lines.into_iter().map(HyperlinkLine::new).collect()
}

/// Detect bare web URLs inside a plain-text line and annotate them.
pub(crate) fn annotate_web_urls_in_line(line: Line<'static>) -> HyperlinkLine {
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    let mut out = HyperlinkLine::new(line);
    out.hyperlinks = web_links_in_text(&text);
    out
}

/// Re-attach source hyperlink ranges after visible-text wrapping has split a line.
///
/// Link text is matched in display order so a URL split across rows retains the
/// complete destination on every rendered fragment. Whitespace inserted or
/// removed at line boundaries is ignored while matching; hyperlink destinations
/// themselves are never reconstructed from output.
pub(crate) fn remap_wrapped_line(
    source: &HyperlinkLine,
    wrapped: Vec<Line<'static>>,
) -> Vec<HyperlinkLine> {
    let mut out = plain_hyperlink_lines(wrapped);
    if source.hyperlinks.is_empty() {
        return out;
    }

    let source_text = line_text(&source.line);
    let mut source_byte = 0usize;
    let mut source_column = 0usize;
    let mut link_index = 0usize;
    for (index, line) in out.iter_mut().enumerate() {
        if index > 0 {
            let trimmed = source_text[source_byte..].trim_start_matches(char::is_whitespace);
            let skipped = source_text[source_byte..].len() - trimmed.len();
            source_column += UnicodeWidthStr::width(&source_text[source_byte..source_byte + skipped]);
            source_byte += skipped;
        }

        let rendered = line_text(&line.line);
        let remaining = &source_text[source_byte..];
        let Some(rendered_start) = longest_suffix_matching_prefix(&rendered, remaining) else {
            continue;
        };
        let mapped = &rendered[rendered_start..];
        let mut output_column = UnicodeWidthStr::width(&rendered[..rendered_start]);
        for ch in mapped.chars() {
            let width = UnicodeWidthChar::width(ch).unwrap_or(0);
            while source
                .hyperlinks
                .get(link_index)
                .is_some_and(|link| link.columns.end <= source_column)
            {
                link_index += 1;
            }
            if let Some(link) = source
                .hyperlinks
                .get(link_index)
                .filter(|link| link.columns.contains(&source_column))
            {
                push_link_range(line, output_column..output_column + width, link);
            }
            source_column += width;
            output_column += width;
        }
        source_byte += mapped.len();
    }
    out
}

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

fn longest_suffix_matching_prefix(rendered: &str, source: &str) -> Option<usize> {
    rendered
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(rendered.len()))
        .find(|index| source.starts_with(&rendered[*index..]) && *index < rendered.len())
}

fn push_link_range(line: &mut HyperlinkLine, range: Range<usize>, link: &HyperlinkRange) {
    if range.is_empty() {
        return;
    }
    if let Some(previous) = line.hyperlinks.last_mut()
        && previous.destination == link.destination
        && previous.columns.end == range.start
    {
        previous.columns.end = range.end;
        return;
    }
    line.hyperlinks.push(HyperlinkRange::web(range, link.destination.clone()));
}

/// Find bare web URLs in `text`, returning display-column ranges.
///
/// A URL may be glued to surrounding text without whitespace (very common in
/// CJK text, e.g. `链接：https://example.com` or `见https://example.com`), so
/// detection scans for `scheme://` starts inside each whitespace-delimited
/// token instead of only trimming the token edges.
pub(crate) fn web_links_in_text(text: &str) -> Vec<HyperlinkRange> {
    let mut links = Vec::new();
    let mut search_from = 0usize;
    for raw_token in text.split_ascii_whitespace() {
        let Some(relative_start) = text[search_from..].find(raw_token) else {
            continue;
        };
        let raw_start = search_from + relative_start;
        search_from = raw_start + raw_token.len();

        let mut candidate_offset = 0usize;
        loop {
            // Skip leading punctuation before this candidate.
            candidate_offset += raw_token[candidate_offset..]
                .find(|ch: char| !is_leading_punctuation(ch))
                .unwrap_or(raw_token.len() - candidate_offset);

            let Some(sep) = raw_token[candidate_offset..].find("://") else {
                // No scheme: a bare host URL is only recognized when it starts
                // the token — a bare host glued to preceding text is ambiguous.
                let rest = &raw_token[candidate_offset..];
                let trimmed_end = trailing_url_end(rest);
                let trimmed = &rest[..trimmed_end];
                if !trimmed.is_empty()
                    && candidate_offset == 0
                    && let Some(destination) = web_destination(trimmed)
                    && !destination.contains("://")
                {
                    push_web_link(
                        &mut links,
                        text,
                        raw_start + candidate_offset,
                        trimmed,
                        destination,
                    );
                }
                break;
            };

            // Walk back from the scheme separator to the start of the scheme,
            // which may be glued to preceding text ("URL：https://…").
            let scheme_start = raw_token[..candidate_offset + sep]
                .char_indices()
                .rev()
                .find(|(_, ch)| {
                    !(ch.is_ascii_alphanumeric() || *ch == '+' || *ch == '-' || *ch == '.')
                })
                .map(|(idx, ch)| idx + ch.len_utf8())
                .unwrap_or(0);
            let scheme_end = candidate_offset + sep;
            if scheme_start >= scheme_end {
                break;
            }
            let scheme = &raw_token[scheme_start..scheme_end];
            if !scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
                // Not a real scheme (e.g. "1://…"); skip past the separator.
                candidate_offset = scheme_end + 3;
                if candidate_offset >= raw_token.len() {
                    break;
                }
                continue;
            }

            let candidate = &raw_token[scheme_start..];
            let trimmed_end = trailing_url_end(candidate);
            let candidate = &candidate[..trimmed_end];
            if let Some(destination) = web_destination(candidate) {
                push_web_link(&mut links, text, raw_start + scheme_start, candidate, destination);
            }
            // Continue scanning after this candidate for further URLs.
            candidate_offset = scheme_start + candidate.len();
            if candidate_offset >= raw_token.len() {
                break;
            }
        }
    }
    links
}

fn push_web_link(
    links: &mut Vec<HyperlinkRange>,
    text: &str,
    start_byte: usize,
    candidate: &str,
    destination: String,
) {
    let start = text[..start_byte].width();
    let end = start + candidate.width();
    links.push(HyperlinkRange::web(start..end, destination));
}

fn is_leading_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | '.' | ';' | ':' | '!' | '\'' | '"'
            | '（' | '【' | '《' | '「' | '『'
    )
}

fn trailing_url_end(candidate: &str) -> usize {
    let mut end = candidate.len();
    while end > 0 {
        let remaining = &candidate[..end];
        let Some(ch) = remaining.chars().next_back() else {
            break;
        };
        let trim = matches!(
            ch,
            ',' | '.' | ';' | '!' | '\'' | '"' | '。' | '，' | '、' | '；' | '！' | '？'
        ) || matches!(ch, ')' | ']' | '}' | '>' | '）' | '】' | '》' | '」' | '』')
            && has_unmatched_closing_delimiter(remaining, ch);
        if !trim {
            break;
        }
        end -= ch.len_utf8();
    }
    end
}

fn has_unmatched_closing_delimiter(candidate: &str, closing: char) -> bool {
    let opening = match closing {
        ')' => '(',
        ']' => '[',
        '}' => '{',
        '>' => '<',
        '）' => '（',
        '】' => '【',
        '》' => '《',
        '」' => '「',
        '』' => '『',
        _ => return false,
    };
    candidate.chars().filter(|ch| *ch == closing).count()
        > candidate.chars().filter(|ch| *ch == opening).count()
}

/// Returns the destination when it is safe to emit as an OSC 8 hyperlink:
/// an `http`/`https` URL with a host, stripped of control characters.
pub(crate) fn web_destination(destination: &str) -> Option<String> {
    let safe_destination = sanitized_destination(destination);
    let parsed = Url::parse(&safe_destination).ok()?;
    matches!(parsed.scheme(), "http" | "https")
        .then(|| parsed.host_str())
        .flatten()?;
    Some(safe_destination)
}

fn sanitized_destination(destination: &str) -> String {
    destination.chars().filter(|ch| !ch.is_control()).collect()
}

/// Wrap `text` in an OSC 8 hyperlink sequence when `destination` is a web
/// destination; otherwise return `text` unchanged.
pub(crate) fn osc8_hyperlink(destination: &str, text: &str) -> String {
    let Some(safe_destination) = web_destination(destination) else {
        return text.to_string();
    };
    format!("\x1b]8;;{safe_destination}\x07{text}\x1b]8;;\x07")
}

/// Remove OSC 8 sequences from a string (used when extracting copyable text
/// from a buffer whose cells carry injected escape sequences).
pub(crate) fn strip_osc8(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut stripped = String::with_capacity(text.len());
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index..].starts_with(b"\x1b]8;;") {
            index += 5;
            while index < bytes.len() {
                if bytes[index] == b'\x07' {
                    index += 1;
                    break;
                }
                if index + 1 < bytes.len() && bytes[index] == b'\x1b' && bytes[index + 1] == b'\\' {
                    index += 2;
                    break;
                }
                index += 1;
            }
            continue;
        }
        let ch = text[index..]
            .chars()
            .next()
            .expect("current byte index starts a character");
        stripped.push(ch);
        index += ch.len_utf8();
    }

    stripped
}

/// Inject OSC 8 escape sequences into frame-buffer cells covered by hyperlinks.
///
/// `links_by_line` is index-aligned with the rendered output lines; each entry
/// lists the hyperlink ranges of that line in display columns. The line at
/// output index `i` occupies screen row `area.y + i - scroll_rows` because the
/// chat list is pre-wrapped (one logical line per row) and rendered with a
/// `Paragraph::scroll` offset of `scroll_rows`. Cells outside the area, skip
/// cells (wide-character continuations), and blank cells are left untouched.
pub(crate) fn mark_buffer_hyperlinks(
    buf: &mut Buffer,
    area: Rect,
    links_by_line: &[Vec<HyperlinkRange>],
    scroll_rows: usize,
) {
    if area.width == 0 {
        return;
    }
    for (line_index, links) in links_by_line.iter().enumerate() {
        if links.is_empty() {
            continue;
        }
        let row = line_index as isize - scroll_rows as isize;
        if row < 0 || row as u16 >= area.height {
            continue;
        }
        let y = area.y + row as u16;
        for link in links {
            let Some(destination) = web_destination(&link.destination) else {
                continue;
            };
            for column in link.columns.clone() {
                if column as u16 >= area.width {
                    continue;
                }
                let x = area.x + column as u16;
                let cell = &mut buf[(x, y)];
                if cell.diff_option == CellDiffOption::Skip || cell.symbol().trim().is_empty() {
                    continue;
                }
                cell.set_symbol(&osc8_hyperlink(&destination, cell.symbol()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_destinations_receive_osc8() {
        assert!(osc8_hyperlink("https://example.com/a", "a").contains("\x1b]8;;"));
        assert_eq!(osc8_hyperlink("mailto:a@example.com", "a"), "a");
        assert_eq!(
            osc8_hyperlink("https://example.com/\u{7}safe", "a"),
            "\x1b]8;;https://example.com/safe\x07a\x1b]8;;\x07"
        );
        assert_eq!(
            strip_osc8(&osc8_hyperlink("https://example.com/a", "visible")),
            "visible"
        );
    }

    #[test]
    fn discovers_punctuated_web_url_columns() {
        assert_eq!(
            web_links_in_text("See (https://example.com/a)."),
            vec![HyperlinkRange::web(
                /*columns*/ 5..26,
                "https://example.com/a".to_string(),
            )]
        );
    }

    #[test]
    fn preserves_balanced_parentheses_in_bare_web_urls() {
        let destination = "https://en.wikipedia.org/wiki/Function_(mathematics)";
        assert_eq!(
            web_links_in_text(&format!("See ({destination}).")),
            vec![HyperlinkRange::web(
                /*columns*/ 5..5 + destination.width(),
                destination.to_string(),
            )]
        );
    }

    #[test]
    fn detects_urls_glued_to_cjk_text() {
        // No whitespace between CJK text and the URL — common in Chinese text.
        assert_eq!(
            web_links_in_text("链接：https://example.com/a"),
            vec![HyperlinkRange::web(
                /*columns*/ 6..27,
                "https://example.com/a".to_string(),
            )]
        );
        assert_eq!(
            web_links_in_text("见https://example.com/a"),
            vec![HyperlinkRange::web(
                /*columns*/ 2..23,
                "https://example.com/a".to_string(),
            )]
        );
        assert_eq!(
            web_links_in_text("（见 https://example.com/a）"),
            vec![HyperlinkRange::web(
                /*columns*/ 5..26,
                "https://example.com/a".to_string(),
            )]
        );
    }

    #[test]
    fn trims_cjk_trailing_punctuation() {
        assert_eq!(
            web_links_in_text("看 https://example.com/a。"),
            vec![HyperlinkRange::web(
                /*columns*/ 3..24,
                "https://example.com/a".to_string(),
            )]
        );
        // Balanced CJK parentheses inside the path are preserved.
        assert_eq!(
            web_links_in_text("https://example.com/（文档）"),
            vec![HyperlinkRange::web(
                /*columns*/ 0..28,
                "https://example.com/（文档）".to_string(),
            )]
        );
    }

    #[test]
    fn strip_osc8_removes_embedded_sequences() {
        let input = format!(
            "a{}b",
            osc8_hyperlink("https://example.com/", "link")
        );
        assert_eq!(strip_osc8(&input), "alinkb");
    }

    #[test]
    fn remap_wrapped_line_maps_ranges_across_rows() {
        let source = HyperlinkLine {
            line: Line::from("  alpha 😀here"),
            hyperlinks: vec![HyperlinkRange::web(
                /*columns*/ 10..14,
                "https://example.com/first".to_string(),
            )],
        };
        // Simulated wrap output: the link columns land on the second row.
        let wrapped = vec![Line::from("  alpha"), Line::from("😀here")];
        let remapped = remap_wrapped_line(&source, wrapped);
        assert_eq!(remapped.len(), 2);
        assert!(remapped[0].hyperlinks.is_empty());
        assert_eq!(remapped[1].hyperlinks.len(), 1);
        // "😀" occupies output columns 0..2, so "here" maps to 2..6.
        assert_eq!(remapped[1].hyperlinks[0].columns, 2..6);
        assert_eq!(
            remapped[1].hyperlinks[0].destination,
            "https://example.com/first"
        );
    }

    #[test]
    fn mark_buffer_hyperlinks_injects_osc8_into_cells() {
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        // Row 0 (output index 0): "hello"; row 1 (output index 1): "link".
        for (col, ch) in "hello".chars().enumerate() {
            buf[(col as u16, 0)].set_symbol(&ch.to_string());
        }
        for (col, ch) in "link".chars().enumerate() {
            buf[(col as u16, 1)].set_symbol(&ch.to_string());
        }
        let links_by_line = vec![
            vec![],
            vec![HyperlinkRange::web(
                0..4,
                "https://example.com".to_string(),
            )],
        ];
        mark_buffer_hyperlinks(&mut buf, area, &links_by_line, /*scroll_rows*/ 0);
        let cell = &buf[(0, 1)];
        assert!(cell.symbol().contains("\x1b]8;;https://example.com\x07"));
        assert!(cell.symbol().ends_with("\x1b]8;;\x07"));
        // The last link column (3) is marked; the unlinked cell after it (4)
        // and the cells of row 0 are untouched.
        assert!(buf[(3, 1)].symbol().contains("\x1b]8;;"));
        assert_eq!(buf[(4, 1)].symbol(), " ");
        assert_eq!(buf[(0, 0)].symbol(), "h");
    }

    #[test]
    fn mark_buffer_hyperlinks_respects_scroll_and_bounds() {
        let area = Rect::new(0, 0, 8, 2);
        let mut buf = Buffer::empty(area);
        for (col, ch) in "abcdefgh".chars().enumerate() {
            buf[(col as u16, 0)].set_symbol(&ch.to_string());
        }
        for (col, ch) in "12345678".chars().enumerate() {
            buf[(col as u16, 1)].set_symbol(&ch.to_string());
        }
        let links_by_line = vec![vec![HyperlinkRange::web(
            0..8,
            "https://example.com".to_string(),
        )]];
        // Line 0 is scrolled off-screen; nothing should be marked.
        mark_buffer_hyperlinks(&mut buf, area, &links_by_line, /*scroll_rows*/ 1);
        assert_eq!(buf[(0, 0)].symbol(), "a");
        // With no scroll the line maps to row 0 and its cells are marked.
        mark_buffer_hyperlinks(&mut buf, area, &links_by_line, /*scroll_rows*/ 0);
        assert!(buf[(0, 0)].symbol().contains("\x1b]8;;"));
        assert!(buf[(7, 0)].symbol().contains("\x1b]8;;"));
        assert_eq!(buf[(0, 1)].symbol(), "1");
    }
}
