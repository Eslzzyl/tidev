use crate::render::line_utils::push_owned_lines;
use ratatui::text::Line;
use ratatui::text::Span;
use std::borrow::Cow;
use std::ops::Range;
use textwrap::Options;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug)]
pub struct RtOptions<'a> {
    pub width: usize,
    pub line_ending: textwrap::LineEnding,
    pub initial_indent: Line<'a>,
    pub subsequent_indent: Line<'a>,
    pub break_words: bool,
    pub wrap_algorithm: textwrap::WrapAlgorithm,
    pub word_separator: textwrap::WordSeparator,
    pub word_splitter: textwrap::WordSplitter,
}

impl From<usize> for RtOptions<'_> {
    fn from(width: usize) -> Self {
        Self::new(width)
    }
}

impl<'a> RtOptions<'a> {
    pub fn new(width: usize) -> Self {
        Self {
            width,
            line_ending: textwrap::LineEnding::LF,
            initial_indent: Line::default(),
            subsequent_indent: Line::default(),
            break_words: true,
            wrap_algorithm: textwrap::WrapAlgorithm::FirstFit,
            word_separator: textwrap::WordSeparator::new(),
            word_splitter: textwrap::WordSplitter::HyphenSplitter,
        }
    }

    pub fn line_ending(self, line_ending: textwrap::LineEnding) -> Self {
        Self { line_ending, ..self }
    }

    pub fn width(self, width: usize) -> Self {
        Self { width, ..self }
    }

    pub fn initial_indent(self, initial_indent: Line<'a>) -> Self {
        Self {
            initial_indent,
            ..self
        }
    }

    pub fn subsequent_indent(self, subsequent_indent: Line<'a>) -> Self {
        Self {
            subsequent_indent,
            ..self
        }
    }

    pub fn break_words(self, break_words: bool) -> Self {
        Self { break_words, ..self }
    }

    pub fn word_separator(self, word_separator: textwrap::WordSeparator) -> Self {
        Self {
            word_separator,
            ..self
        }
    }

    pub fn wrap_algorithm(self, wrap_algorithm: textwrap::WrapAlgorithm) -> Self {
        Self {
            wrap_algorithm,
            ..self
        }
    }

    pub fn word_splitter(self, word_splitter: textwrap::WordSplitter) -> Self {
        Self {
            word_splitter,
            ..self
        }
    }
}

pub(crate) fn adaptive_wrap_line<'a>(line: &'a Line<'a>, base: RtOptions<'a>) -> Vec<Line<'a>> {
    let selected = if line_contains_url_like(line) {
        url_preserving_wrap_options(base)
    } else {
        base
    };
    word_wrap_line(line, selected)
}

#[allow(private_bounds)]
pub(crate) fn adaptive_wrap_lines<'a, I, L>(
    lines: I,
    width_or_options: RtOptions<'a>,
) -> Vec<Line<'static>>
where
    I: IntoIterator<Item = L>,
    L: IntoLineInput<'a>,
{
    let base_opts = width_or_options;
    let mut out: Vec<Line<'static>> = Vec::new();

    for (index, line) in lines.into_iter().enumerate() {
        let line_input = line.into_line_input();
        let opts = if index == 0 {
            base_opts.clone()
        } else {
            base_opts
                .clone()
                .initial_indent(base_opts.subsequent_indent.clone())
        };

        let wrapped = adaptive_wrap_line(line_input.as_ref(), opts);
        push_owned_lines(&wrapped, &mut out);
    }

    out
}

pub(crate) fn word_wrap_line<'a, O>(line: &'a Line<'a>, width_or_options: O) -> Vec<Line<'a>>
where
    O: Into<RtOptions<'a>>,
{
    let mut flat = String::new();
    let mut span_bounds = Vec::new();
    let mut acc = 0usize;
    for span in &line.spans {
        let text = span.content.as_ref();
        let start = acc;
        flat.push_str(text);
        acc += text.len();
        span_bounds.push((start..acc, span.style));
    }

    let rt_opts: RtOptions<'a> = width_or_options.into();
    let opts = Options::new(rt_opts.width)
        .line_ending(rt_opts.line_ending)
        .break_words(rt_opts.break_words)
        .wrap_algorithm(rt_opts.wrap_algorithm)
        .word_separator(rt_opts.word_separator)
        .word_splitter(rt_opts.word_splitter);

    let mut out: Vec<Line<'a>> = Vec::new();

    let initial_width_available = opts
        .width
        .saturating_sub(line_display_width(&rt_opts.initial_indent))
        .max(1);
    let initial_wrapped = wrap_ranges_trim(&flat, opts.clone().width(initial_width_available));
    let Some(first_line_range) = initial_wrapped.first() else {
        return vec![rt_opts.initial_indent.clone()];
    };

    let mut first_line = rt_opts.initial_indent.clone().style(line.style);
    {
        let sliced = slice_line_spans(line, &span_bounds, first_line_range);
        let mut spans = first_line.spans;
        spans.extend(sliced.spans.into_iter().map(|span| Span {
            style: span.style.patch(line.style),
            content: span.content,
        }));
        first_line.spans = spans;
        out.push(first_line);
    }

    let base = first_line_range.end;
    let skip_leading_spaces = flat[base..].chars().take_while(|c| *c == ' ').count();
    let base = base + skip_leading_spaces;
    let subsequent_width_available = opts
        .width
        .saturating_sub(line_display_width(&rt_opts.subsequent_indent))
        .max(1);
    let remaining_wrapped = wrap_ranges_trim(&flat[base..], opts.width(subsequent_width_available));
    for r in &remaining_wrapped {
        if r.is_empty() {
            continue;
        }
        let mut subsequent_line = rt_opts.subsequent_indent.clone().style(line.style);
        let offset_range = (r.start + base)..(r.end + base);
        let sliced = slice_line_spans(line, &span_bounds, &offset_range);
        let mut spans = subsequent_line.spans;
        spans.extend(sliced.spans.into_iter().map(|span| Span {
            style: span.style.patch(line.style),
            content: span.content,
        }));
        subsequent_line.spans = spans;
        out.push(subsequent_line);
    }

    out
}

#[allow(dead_code)]
#[allow(private_bounds)]
pub(crate) fn word_wrap_lines<'a, I, O, L>(lines: I, width_or_options: O) -> Vec<Line<'static>>
where
    I: IntoIterator<Item = L>,
    L: IntoLineInput<'a>,
    O: Into<RtOptions<'a>>,
{
    let base_opts: RtOptions<'a> = width_or_options.into();
    let mut out: Vec<Line<'static>> = Vec::new();

    for (index, line) in lines.into_iter().enumerate() {
        let line_input = line.into_line_input();
        let opts = if index == 0 {
            base_opts.clone()
        } else {
            base_opts
                .clone()
                .initial_indent(base_opts.subsequent_indent.clone())
        };
        let wrapped = word_wrap_line(line_input.as_ref(), opts);
        push_owned_lines(&wrapped, &mut out);
    }

    out
}

#[allow(dead_code)]
pub(crate) fn word_wrap_lines_borrowed<'a, I, O>(
    lines: I,
    width_or_options: O,
) -> Vec<Line<'a>>
where
    I: IntoIterator<Item = &'a Line<'a>>,
    O: Into<RtOptions<'a>>,
{
    let base_opts: RtOptions<'a> = width_or_options.into();
    let mut out: Vec<Line<'a>> = Vec::new();
    let mut first = true;
    for line in lines.into_iter() {
        let opts = if first {
            base_opts.clone()
        } else {
            base_opts
                .clone()
                .initial_indent(base_opts.subsequent_indent.clone())
        };
        out.extend(word_wrap_line(line, opts));
        first = false;
    }
    out
}

fn line_display_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn line_contains_url_like(line: &Line<'_>) -> bool {
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    text_contains_url_like(&text)
}

fn text_contains_url_like(text: &str) -> bool {
    text.split_ascii_whitespace().any(is_url_like_token)
}

fn is_url_like_token(raw_token: &str) -> bool {
    let token = trim_url_token(raw_token);
    !token.is_empty() && (is_absolute_url_like(token) || is_bare_url_like(token))
}

fn trim_url_token(raw_token: &str) -> &str {
    raw_token.trim_matches(|ch: char| {
        matches!(
            ch,
            '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | '.' | ';' | ':' | '!' | '?' | '\'' | '"'
        )
    })
}

fn is_absolute_url_like(token: &str) -> bool {
    if !token.contains("://") {
        return false;
    }

    if let Ok(url) = url::Url::parse(token) {
        let scheme = url.scheme().to_ascii_lowercase();
        if matches!(scheme.as_str(), "http" | "https" | "ftp" | "ftps" | "ws" | "wss") {
            return url.host_str().is_some();
        }
        return true;
    }

    has_valid_scheme_prefix(token)
}

fn has_valid_scheme_prefix(token: &str) -> bool {
    let Some((scheme, rest)) = token.split_once("://") else {
        return false;
    };
    if scheme.is_empty() || rest.is_empty() {
        return false;
    }

    let mut chars = scheme.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

fn is_bare_url_like(token: &str) -> bool {
    let (host_port, has_trailer) = split_host_port_and_trailer(token);
    if host_port.is_empty() {
        return false;
    }

    if !has_trailer && !host_port.to_ascii_lowercase().starts_with("www.") {
        return false;
    }

    let (host, port) = split_host_and_port(host_port);
    if host.is_empty() {
        return false;
    }
    if let Some(port) = port
        && !is_valid_port(port)
    {
        return false;
    }

    host.eq_ignore_ascii_case("localhost") || is_ipv4(host) || is_domain_name(host)
}

fn split_host_port_and_trailer(token: &str) -> (&str, bool) {
    if let Some(idx) = token.find(['/', '?', '#']) {
        (&token[..idx], true)
    } else {
        (token, false)
    }
}

fn split_host_and_port(host_port: &str) -> (&str, Option<&str>) {
    if host_port.starts_with('[') {
        return (host_port, None);
    }

    if let Some((host, port)) = host_port.rsplit_once(':')
        && !host.is_empty()
        && !port.is_empty()
        && port.chars().all(|c| c.is_ascii_digit())
    {
        return (host, Some(port));
    }

    (host_port, None)
}

fn is_valid_port(port: &str) -> bool {
    if port.is_empty() || port.len() > 5 || !port.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }

    port.parse::<u16>().is_ok()
}

fn is_ipv4(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return false;
    }

    parts.iter().all(|part| !part.is_empty() && part.parse::<u8>().is_ok())
}

fn is_domain_name(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if !host.contains('.') {
        return false;
    }

    let mut labels = host.split('.');
    let Some(tld) = labels.next_back() else {
        return false;
    };
    if !is_tld(tld) {
        return false;
    }

    labels.all(is_domain_label)
}

fn is_tld(label: &str) -> bool {
    (2..=63).contains(&label.len()) && label.chars().all(|c| c.is_ascii_alphabetic())
}

fn is_domain_label(label: &str) -> bool {
    if label.is_empty() || label.len() > 63 {
        return false;
    }

    let mut chars = label.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let Some(last) = label.chars().next_back() else {
        return false;
    };

    first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub(crate) fn url_preserving_wrap_options<'a>(opts: RtOptions<'a>) -> RtOptions<'a> {
    opts.word_separator(textwrap::WordSeparator::AsciiSpace)
        .word_splitter(textwrap::WordSplitter::Custom(split_non_url_word))
        .break_words(false)
}

fn split_non_url_word(word: &str) -> Vec<usize> {
    if is_url_like_token(word) {
        return Vec::new();
    }

    word.char_indices().skip(1).map(|(idx, _)| idx).collect()
}

fn wrap_ranges_trim<'a, O>(text: &str, width_or_options: O) -> Vec<Range<usize>>
where
    O: Into<Options<'a>>,
{
    let opts = width_or_options.into();
    let mut lines: Vec<Range<usize>> = Vec::new();
    let mut cursor = 0usize;
    for (line_index, line) in textwrap::wrap(text, &opts).iter().enumerate() {
        match line {
            Cow::Borrowed(slice) => {
                let start = unsafe { slice.as_ptr().offset_from(text.as_ptr()) as usize };
                let end = start + slice.len();
                lines.push(start..end);
                cursor = end;
            }
            Cow::Owned(slice) => {
                let synthetic_prefix = if line_index == 0 {
                    opts.initial_indent
                } else {
                    opts.subsequent_indent
                };
                let mapped = map_owned_wrapped_line_to_range(text, cursor, slice, synthetic_prefix);
                lines.push(mapped.clone());
                cursor = mapped.end;
            }
        }
    }
    lines
}

fn map_owned_wrapped_line_to_range(
    text: &str,
    cursor: usize,
    wrapped: &str,
    synthetic_prefix: &str,
) -> Range<usize> {
    let wrapped = if synthetic_prefix.is_empty() {
        wrapped
    } else {
        wrapped.strip_prefix(synthetic_prefix).unwrap_or(wrapped)
    };

    let mut start = cursor;
    while start < text.len() && !wrapped.starts_with(' ') {
        let Some(ch) = text[start..].chars().next() else {
            break;
        };
        if ch != ' ' {
            break;
        }
        start += ch.len_utf8();
    }

    let mut end = start;
    let mut saw_source_char = false;
    let mut chars = wrapped.chars().peekable();
    while let Some(ch) = chars.next() {
        if end < text.len() {
            let Some(src) = text[end..].chars().next() else {
                unreachable!("checked end < text.len()");
            };
            if ch == src {
                end += src.len_utf8();
                saw_source_char = true;
                continue;
            }
        }

        if ch == '-' && chars.peek().is_none() {
            continue;
        }

        if !saw_source_char {
            continue;
        }

        break;
    }

    start..end
}

#[derive(Debug)]
enum LineInput<'a> {
    Borrowed(&'a Line<'a>),
    Owned(Line<'a>),
}

impl<'a> LineInput<'a> {
    fn as_ref(&self) -> &Line<'a> {
        match self {
            LineInput::Borrowed(line) => line,
            LineInput::Owned(line) => line,
        }
    }
}

trait IntoLineInput<'a> {
    fn into_line_input(self) -> LineInput<'a>;
}

impl<'a> IntoLineInput<'a> for &'a Line<'a> {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Borrowed(self)
    }
}

impl<'a> IntoLineInput<'a> for &'a mut Line<'a> {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Borrowed(self)
    }
}

impl<'a> IntoLineInput<'a> for Line<'a> {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Owned(self)
    }
}

impl<'a> IntoLineInput<'a> for String {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Owned(Line::from(self))
    }
}

impl<'a> IntoLineInput<'a> for &'a str {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Owned(Line::from(self))
    }
}

impl<'a> IntoLineInput<'a> for Cow<'a, str> {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Owned(Line::from(self))
    }
}

impl<'a> IntoLineInput<'a> for Span<'a> {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Owned(Line::from(self))
    }
}

impl<'a> IntoLineInput<'a> for Vec<Span<'a>> {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Owned(Line::from(self))
    }
}

fn slice_line_spans<'a>(
    original: &'a Line<'a>,
    span_bounds: &[(Range<usize>, ratatui::style::Style)],
    range: &Range<usize>,
) -> Line<'a> {
    let start_byte = range.start;
    let end_byte = range.end;
    let mut acc: Vec<Span<'a>> = Vec::new();
    for (index, (range, style)) in span_bounds.iter().enumerate() {
        let s = range.start;
        let e = range.end;
        if e <= start_byte {
            continue;
        }
        if s >= end_byte {
            break;
        }
        let seg_start = start_byte.max(s);
        let seg_end = end_byte.min(e);
        if seg_end > seg_start {
            let local_start = seg_start - s;
            let local_end = seg_end - s;
            let content = original.spans[index].content.as_ref();
            let slice = &content[local_start..local_end];
            acc.push(Span {
                style: *style,
                content: std::borrow::Cow::Borrowed(slice),
            });
        }
        if e >= end_byte {
            break;
        }
    }
    Line {
        style: original.style,
        alignment: original.alignment,
        spans: acc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;
    use ratatui::style::Stylize;

    fn concat_line(line: &Line) -> String {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn wraps_plain_text() {
        let line = Line::from("hello world");
        let out = word_wrap_line(&line, 5);
        assert_eq!(out.len(), 2);
        assert_eq!(concat_line(&out[0]), "hello");
        assert_eq!(concat_line(&out[1]), "world");
    }

    #[test]
    fn preserves_styles() {
        let line = Line::from(vec!["hello ".red(), "world".into()]);
        let out = word_wrap_line(&line, 6);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].spans.len(), 1);
        assert_eq!(out[0].spans[0].style.fg, Some(Color::Red));
        assert_eq!(concat_line(&out[0]), "hello");
        assert_eq!(concat_line(&out[1]), "world");
    }

    #[test]
    fn wrap_lines_accepts_str_slices() {
        let lines = ["hello world", "goodnight moon"];
        let out = word_wrap_lines(lines, 12);
        let rendered: Vec<String> = out.iter().map(concat_line).collect();
        assert_eq!(rendered, vec!["hello world", "goodnight", "moon"]);
    }

    #[test]
    fn line_contains_url_like_matches_expected_tokens() {
        let line = Line::from("see https://example.com/path for details");
        assert!(line_contains_url_like(&line));
    }

    #[test]
    fn url_preserving_wrap_keeps_url_intact() {
        let line = Line::from("https://example.com/long-url-with-dashes-wider-than-terminal");
        let out = adaptive_wrap_line(&line, RtOptions::new(20));
        assert_eq!(out.len(), 1);
        assert_eq!(concat_line(&out[0]), "https://example.com/long-url-with-dashes-wider-than-terminal");
    }
}
