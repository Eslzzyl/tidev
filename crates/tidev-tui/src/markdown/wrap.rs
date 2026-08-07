use super::line::push_owned_lines;
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
        Self {
            break_words,
            ..self
        }
    }

    pub fn word_separator(self, word_separator: textwrap::WordSeparator) -> RtOptions<'a> {
        RtOptions {
            word_separator,
            ..self
        }
    }

    pub fn word_splitter(self, word_splitter: textwrap::WordSplitter) -> RtOptions<'a> {
        RtOptions {
            word_splitter,
            ..self
        }
    }
}

/// Wraps a single line, automatically switching to URL-aware behavior when
/// the line contains URL-like tokens.
///
/// Lines without URL-like tokens wrap identically to [`word_wrap_line`].
/// URL-only lines use URL-preserving options (ASCII-space word separation and
/// no hyphenation) so terminal link detection keeps seeing one intact token.
/// Mixed URL/prose lines use a token-aware wrapper so ordinary prose still
/// moves as whole words while an overlong token can still split when needed.
///
/// Unlike Codex, a URL that is itself wider than the row is hard-broken
/// instead of being emitted as an over-wide line: tidev's renderer pre-wraps
/// every line to the content width and clips anything wider, so an unbroken
/// over-wide URL would be cut off and become unreadable.
pub fn adaptive_wrap_line<'a>(line: &'a Line<'a>, base: RtOptions<'a>) -> Vec<Line<'a>> {
    let (flat, span_bounds) = flatten_line(line);
    let mut saw_url = false;
    let mut saw_non_url = false;

    for token in flat.split_ascii_whitespace() {
        if is_url_like_token(token) {
            saw_url = true;
        } else if is_substantive_non_url_token(token) {
            saw_non_url = true;
        }

        if saw_url && saw_non_url {
            break;
        }
    }

    if !saw_url {
        word_wrap_flattened_line(line, &flat, &span_bounds, base)
    } else if saw_non_url {
        mixed_url_wrap_line(line, &flat, &span_bounds, base)
    } else {
        word_wrap_flattened_line(line, &flat, &span_bounds, url_preserving_wrap_options(base))
    }
}

fn flatten_line(line: &Line<'_>) -> (String, Vec<(Range<usize>, ratatui::style::Style)>) {
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
    (flat, span_bounds)
}

// ---------------------------------------------------------------------------
// URL-like token detection
// ---------------------------------------------------------------------------

/// Decides whether a single whitespace-delimited token is URL-like.
///
/// Strips surrounding punctuation, then checks for an absolute URL
/// (with `://`) or a bare domain URL (recognized host + path/query/fragment).
fn is_url_like_token(raw_token: &str) -> bool {
    let token = trim_url_token(raw_token);
    !token.is_empty() && (is_absolute_url_like(token) || is_bare_url_like(token))
}

fn is_substantive_non_url_token(raw_token: &str) -> bool {
    let token = trim_url_token(raw_token);
    if token.is_empty() || is_decorative_marker_token(raw_token, token) {
        return false;
    }

    token.chars().any(char::is_alphanumeric)
}

fn is_decorative_marker_token(raw_token: &str, token: &str) -> bool {
    let raw = raw_token.trim();
    matches!(
        raw,
        "-" | "*"
            | "+"
            | "•"
            | "◦"
            | "▪"
            | ">"
            | "|"
            | "│"
            | "┆"
            | "└"
            | "├"
            | "┌"
            | "┐"
            | "┘"
            | "┼"
    ) || is_ordered_list_marker(raw, token)
}

fn is_ordered_list_marker(raw_token: &str, token: &str) -> bool {
    token.chars().all(|c| c.is_ascii_digit())
        && (raw_token.ends_with('.') || raw_token.ends_with(')'))
}

fn trim_url_token(token: &str) -> &str {
    token.trim_matches(|c: char| {
        matches!(
            c,
            '(' | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | ','
                | '.'
                | ';'
                | ':'
                | '!'
                | '\''
                | '"'
                | '（'
                | '）'
                | '【'
                | '】'
                | '《'
                | '》'
                | '「'
                | '」'
                | '『'
                | '』'
                | '。'
                | '，'
                | '、'
                | '；'
                | '？'
        )
    })
}

/// Checks for `scheme://host` patterns. Uses `url::Url::parse` for
/// well-known schemes; falls back to `has_valid_scheme_prefix` for
/// custom schemes that the `url` crate rejects.
fn is_absolute_url_like(token: &str) -> bool {
    if !token.contains("://") {
        return false;
    }

    if let Ok(url) = url::Url::parse(token) {
        let scheme = url.scheme().to_ascii_lowercase();
        if matches!(
            scheme.as_str(),
            "http" | "https" | "ftp" | "ftps" | "ws" | "wss"
        ) {
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

/// Checks for bare-domain URLs without a scheme: `host[:port]/path`,
/// `host[:port]?query`, or `host[:port]#fragment`.
///
/// Requires that the host is `localhost`, an IPv4 address, or a valid
/// domain name. Bare `host.tld` without a path/query/fragment is only
/// accepted when the host starts with `www.`.
///
/// IPv6 bracket notation (`[::1]:8080`) is intentionally not handled.
fn is_bare_url_like(token: &str) -> bool {
    let (host_port, has_trailer) = split_host_port_and_trailer(token);
    if host_port.is_empty() {
        return false;
    }

    // Require URL-ish trailer for bare hosts unless token starts with www.
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
    // We intentionally do not treat bracketed IPv6 as URL-like in this first pass.
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

    parts
        .iter()
        .all(|part| !part.is_empty() && part.parse::<u8>().is_ok())
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

/// Reconfigures wrapping options so that URL-like tokens are never split at
/// `/`, `-`, or other punctuation: ASCII-space word separation and no
/// per-word hyphenation. `break_words` stays enabled so a token wider than
/// the row is hard-broken rather than emitted as an over-wide line.
fn url_preserving_wrap_options<'a>(opts: RtOptions<'a>) -> RtOptions<'a> {
    opts.word_separator(textwrap::WordSeparator::AsciiSpace)
        .word_splitter(textwrap::WordSplitter::NoHyphenation)
        .break_words(true)
}

// ---------------------------------------------------------------------------
// Mixed URL/prose wrapping
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct MixedUrlWord {
    range: Range<usize>,
}

impl MixedUrlWord {
    fn width(&self, text: &str) -> usize {
        UnicodeWidthStr::width(&text[self.range.clone()])
    }
}

fn mixed_url_wrap_line<'a>(
    line: &'a Line<'a>,
    flat: &str,
    span_bounds: &[(Range<usize>, ratatui::style::Style)],
    rt_opts: RtOptions<'a>,
) -> Vec<Line<'a>> {
    let initial_width_available = rt_opts
        .width
        .saturating_sub(line_display_width(&rt_opts.initial_indent))
        .max(1);
    let subsequent_width_available = rt_opts
        .width
        .saturating_sub(line_display_width(&rt_opts.subsequent_indent))
        .max(1);
    let ranges = mixed_url_wrap_ranges(flat, initial_width_available, subsequent_width_available);

    let mut out = Vec::new();
    for (idx, range) in ranges.iter().enumerate() {
        let mut wrapped_line = if idx == 0 {
            rt_opts.initial_indent.clone()
        } else {
            rt_opts.subsequent_indent.clone()
        }
        .style(line.style);
        let sliced = slice_line_spans(line, span_bounds, range);
        let mut spans = wrapped_line.spans;
        spans.extend(
            sliced
                .spans
                .into_iter()
                .map(|span| span.patch_style(line.style)),
        );
        wrapped_line.spans = spans;
        out.push(wrapped_line);
    }

    if out.is_empty() {
        vec![rt_opts.initial_indent.clone()]
    } else {
        out
    }
}

fn mixed_url_wrap_ranges(
    text: &str,
    initial_width: usize,
    subsequent_width: usize,
) -> Vec<Range<usize>> {
    let leading_space_width = text.chars().take_while(|ch| *ch == ' ').count();
    let mut words = Vec::new();
    let mut cursor = 0usize;
    for word in textwrap::WordSeparator::AsciiSpace.find_words(text) {
        let word_start = cursor;
        let word_end = word_start + word.word.len();
        let trailing_space_end = word_end + word.whitespace.len();
        if !word.word.is_empty() {
            words.push(MixedUrlWord {
                range: word_start..word_end,
            });
        }
        cursor = trailing_space_end;
    }

    let mut lines = Vec::new();
    let mut line_start = None;
    let mut line_end = 0usize;
    let mut line_width = 0usize;
    let mut line_limit = initial_width.max(1);

    for word in words {
        let mut pending = split_mixed_url_word(text, word, line_limit);
        let mut pending_idx = 0usize;

        while let Some(piece) = pending.get(pending_idx).cloned() {
            let empty_line_prefix_width = if line_start.is_none() && lines.is_empty() {
                leading_space_width
            } else {
                0
            };
            let empty_line_piece_limit = line_limit.saturating_sub(empty_line_prefix_width).max(1);
            if line_start.is_none() && piece.width(text) > empty_line_piece_limit {
                pending.splice(
                    pending_idx..=pending_idx,
                    split_mixed_url_word(text, piece, empty_line_piece_limit),
                );
                continue;
            }

            let piece_width = piece.width(text);
            let inter_word_space = line_start
                .map(|_| text[line_end..piece.range.start].len())
                .unwrap_or(0);
            let fits = if line_start.is_none() {
                empty_line_prefix_width + piece_width <= line_limit
                    || empty_line_prefix_width >= line_limit
            } else {
                line_width + inter_word_space + piece_width <= line_limit
            };

            if fits {
                if line_start.is_none() {
                    let is_first_output_line = lines.is_empty();
                    let start = if is_first_output_line {
                        0
                    } else {
                        piece.range.start
                    };
                    line_start = Some(start);
                    line_width = if is_first_output_line {
                        leading_space_width + piece_width
                    } else {
                        piece_width
                    };
                } else {
                    line_width += inter_word_space + piece_width;
                }
                line_end = piece.range.end;
                pending_idx += 1;
                continue;
            }

            if let Some(start) = line_start.take() {
                lines.push(start..line_end);
            }
            line_end = 0;
            line_width = 0;
            line_limit = subsequent_width.max(1);
        }
    }

    if let Some(start) = line_start {
        lines.push(start..line_end);
    }

    lines
}

fn split_mixed_url_word(text: &str, word: MixedUrlWord, line_limit: usize) -> Vec<MixedUrlWord> {
    if word.width(text) <= line_limit {
        return vec![word];
    }

    let source = textwrap::core::Word::from(&text[word.range.clone()]);
    let mut offset = word.range.start;
    let mut pieces = Vec::new();
    for piece in source.break_apart(line_limit.max(1)) {
        let end = offset + piece.word.len();
        pieces.push(MixedUrlWord { range: offset..end });
        offset = end;
    }
    pieces
}

#[allow(private_bounds)]
pub fn adaptive_wrap_lines<'a, I, L>(
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

pub fn word_wrap_line<'a, O>(line: &'a Line<'a>, width_or_options: O) -> Vec<Line<'a>>
where
    O: Into<RtOptions<'a>>,
{
    let (flat, span_bounds) = flatten_line(line);
    word_wrap_flattened_line(line, &flat, &span_bounds, width_or_options.into())
}

fn word_wrap_flattened_line<'a>(
    line: &'a Line<'a>,
    flat: &str,
    span_bounds: &[(Range<usize>, ratatui::style::Style)],
    rt_opts: RtOptions<'a>,
) -> Vec<Line<'a>> {
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
    let initial_wrapped = wrap_ranges_trim(flat, opts.clone().width(initial_width_available));
    let Some(first_line_range) = initial_wrapped.first() else {
        return vec![rt_opts.initial_indent.clone()];
    };

    let mut first_line = rt_opts.initial_indent.clone().style(line.style);
    {
        let sliced = slice_line_spans(line, span_bounds, first_line_range);
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
        let sliced = slice_line_spans(line, span_bounds, &offset_range);
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
pub(crate) fn word_wrap_lines_borrowed<'a, I, O>(lines: I, width_or_options: O) -> Vec<Line<'a>>
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
                // SAFETY: textwrap::wrap() returns Cow::Borrowed only when the
                // wrapped line is a subslice of the input `text`.  offset_from
                // is valid only when both pointers point into the same
                // allocation, which holds here.
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
    fn url_short_enough_stays_intact() {
        let line = Line::from("https://example.com");
        let out = adaptive_wrap_line(&line, RtOptions::new(40));
        assert_eq!(out.len(), 1);
        assert_eq!(concat_line(&out[0]), "https://example.com");
    }

    #[test]
    fn url_longer_than_width_hard_breaks_without_truncation() {
        let line = Line::from("https://example.com/long-path-wider-than-terminal");
        let out = adaptive_wrap_line(&line, RtOptions::new(30));
        // URL-only lines use URL-preserving options: no hyphenation, hard
        // break at the width boundary so the URL is never truncated.
        assert!(out.len() >= 2);
        let full: String = out.iter().map(concat_line).collect();
        assert_eq!(full, "https://example.com/long-path-wider-than-terminal");
        assert_eq!(concat_line(&out[0]), "https://example.com/long-path-");
        assert_eq!(concat_line(&out[1]), "wider-than-terminal");
    }

    #[test]
    fn url_with_query_hard_breaks_at_width() {
        let line =
            Line::from("https://example.com/search?q=very+long+query+string&page=1&limit=50");
        let out = adaptive_wrap_line(&line, RtOptions::new(45));
        assert_eq!(out.len(), 2);
        assert_eq!(
            concat_line(&out[0]),
            "https://example.com/search?q=very+long+query+"
        );
        assert_eq!(concat_line(&out[1]), "string&page=1&limit=50");
    }

    #[test]
    fn url_in_mixed_text_wraps_naturally() {
        let line = Line::from("Check this: https://example.com/very/long/path more text");
        let out = adaptive_wrap_line(&line, RtOptions::new(35));
        // The URL token is kept intact and moves as a whole word; prose wraps
        // around it.
        assert_eq!(out.len(), 3);
        assert_eq!(concat_line(&out[0]), "Check this:");
        assert_eq!(concat_line(&out[1]), "https://example.com/very/long/path");
        assert_eq!(concat_line(&out[2]), "more text");
    }

    #[test]
    fn url_in_mixed_text_hard_breaks_overwide_url() {
        let line = Line::from("See https://example.com/very/long/path now");
        let out = adaptive_wrap_line(&line, RtOptions::new(20));
        // "See" + URL does not fit; the URL itself exceeds the row and is
        // hard-broken (content preserved) instead of spanning an over-wide line.
        let full: String = out.iter().map(concat_line).collect();
        assert!(full.contains("https://example.com/very/long/path"));
        assert!(
            out.iter()
                .all(|l| UnicodeWidthStr::width(concat_line(l).as_str()) <= 20)
        );
    }

    #[test]
    fn url_like_token_matches_expected_tokens() {
        for token in [
            "https://example.com",
            "http://localhost:8080/path",
            "www.example.com/a",
            "ftp://files.example.com/x",
            "git://host/repo",
            "https://example.com:8443/x",
        ] {
            assert!(is_url_like_token(token), "expected URL-like: {token}");
        }
    }

    #[test]
    fn url_like_token_rejects_non_urls() {
        for token in [
            "src/main.rs",
            "hello",
            "a/b/c",
            "example.com",
            "localhost:8080",
            "v1.2.3",
            "foo_bar",
        ] {
            assert!(!is_url_like_token(token), "expected non-URL: {token}");
        }
    }

    #[test]
    fn long_url_never_truncated_even_at_narrow_width() {
        let line = Line::from("https://github.com/very-long-project-name/issues/12345");
        // With a very narrow width that would have caused truncation before,
        // the URL should still be fully visible via wrapping.
        let out = adaptive_wrap_line(&line, RtOptions::new(25));
        let full: String = out.iter().map(concat_line).collect();
        assert_eq!(
            full, "https://github.com/very-long-project-name/issues/12345",
            "URL must never be truncated — all content must be present"
        );
        assert!(out.len() >= 2, "should wrap across multiple lines");
    }
}
