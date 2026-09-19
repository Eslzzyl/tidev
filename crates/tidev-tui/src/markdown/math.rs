use std::borrow::Cow;
use std::ops::Range;

use memchr::{memchr, memchr2};

/// Normalize TeX delimiters and line structure for pulldown-cmark.
///
/// The returned string is only used as the input to the Markdown parser. The
/// original message remains unchanged in the session history and in requests
/// sent to the model. Display-math line breaks become spaces so Markdown block
/// constructs such as setext headings cannot split a formula before parsing.
pub(super) fn normalize_math_delimiters(source: &str) -> Cow<'_, str> {
    if memchr2(b'\\', b'$', source.as_bytes()).is_none() {
        return Cow::Borrowed(source);
    }

    let code = code_ranges(source);
    let dollar_math = dollar_math_ranges(source, &code);
    let mut protected = code.clone();
    protected.extend(dollar_math.iter().cloned());
    protected.sort_unstable_by_key(|range| range.start);
    let mut replacements = display_dollar_line_break_replacements(source, &dollar_math);
    let mut protected_index = 0;
    let mut cursor = 0;
    let mut escaped = false;

    while cursor < source.len() {
        if let Some(range) = protected.get(protected_index) {
            if cursor >= range.end {
                protected_index += 1;
                continue;
            }
            if range.contains(&cursor) {
                cursor = range.end;
                protected_index += 1;
                escaped = is_escaped(source, cursor);
                continue;
            }
        }

        let segment_end = protected
            .get(protected_index)
            .map_or(source.len(), |range| range.start);
        let Some(relative_slash) = memchr(b'\\', &source.as_bytes()[cursor..segment_end]) else {
            cursor = segment_end;
            escaped = false;
            continue;
        };
        let slash = cursor + relative_slash;
        if slash > cursor {
            escaped = false;
        }
        if escaped {
            cursor = slash + 1;
            escaped = !escaped;
            continue;
        }

        cursor = slash;

        if let Some((environment, opener_end)) = parse_environment(source, cursor)
            && is_display_environment(environment)
        {
            let closing = format!(r"\end{{{environment}}}");
            if let Some(end_start) = find_unescaped(source, opener_end, &closing, &protected) {
                let end = end_start + closing.len();
                replacements.push(Replacement {
                    range: cursor..cursor,
                    text: "$$",
                });
                replacements.push(Replacement {
                    range: end..end,
                    text: "$$",
                });
                replacements.extend(normalize_display_line_breaks(source, cursor, end));
                cursor = end;
                escaped = is_escaped(source, cursor);
                continue;
            }
        }

        let Some((open, close, replacement_open, replacement_close)) = math_opener(source, cursor)
        else {
            cursor += 1;
            escaped = true;
            continue;
        };

        let body_start = cursor + open.len();
        let Some(body_end) = find_unescaped(source, body_start, close, &protected) else {
            cursor = body_start;
            escaped = is_escaped(source, cursor);
            continue;
        };
        if source[body_start..body_end].trim().is_empty() {
            cursor = body_end + close.len();
            escaped = is_escaped(source, cursor);
            continue;
        }

        replacements.push(Replacement {
            range: cursor..body_start,
            text: replacement_open,
        });
        replacements.push(Replacement {
            range: body_end..body_end + close.len(),
            text: replacement_close,
        });
        if replacement_open == "$$" {
            replacements.extend(normalize_display_line_breaks(source, body_start, body_end));
        }
        cursor = body_end + close.len();
        escaped = is_escaped(source, cursor);
    }

    if replacements.is_empty() {
        return Cow::Borrowed(source);
    }

    replacements
        .sort_unstable_by_key(|replacement| (replacement.range.start, replacement.range.end));
    let mut normalized = String::with_capacity(source.len());
    let mut cursor = 0;
    for replacement in replacements {
        normalized.push_str(&source[cursor..replacement.range.start]);
        normalized.push_str(replacement.text);
        cursor = replacement.range.end;
    }
    normalized.push_str(&source[cursor..]);
    Cow::Owned(normalized)
}

struct Replacement {
    range: Range<usize>,
    text: &'static str,
}

fn math_opener(
    source: &str,
    cursor: usize,
) -> Option<(&'static str, &'static str, &'static str, &'static str)> {
    if source[cursor..].starts_with(r"\[") {
        return Some((r"\[", r"\]", "$$", "$$"));
    }
    if source[cursor..].starts_with(r"\(") {
        return Some((r"\(", r"\)", "$", "$"));
    }
    None
}

fn parse_environment(source: &str, start: usize) -> Option<(&str, usize)> {
    if !source.get(start..)?.starts_with(r"\begin{") {
        return None;
    }
    let name_start = start.checked_add(r"\begin{".len())?;
    let close =
        memchr(b'}', source.as_bytes().get(name_start..)?).map(|offset| offset + name_start)?;
    let name = &source[name_start..close];
    (!name.is_empty()).then_some((name, close + 1))
}

fn is_display_environment(environment: &str) -> bool {
    matches!(
        environment,
        "align"
            | "align*"
            | "alignat"
            | "alignat*"
            | "aligned"
            | "cases"
            | "displaymath"
            | "equation"
            | "equation*"
            | "flalign"
            | "flalign*"
            | "gather"
            | "gather*"
            | "gathered"
            | "matrix"
            | "pmatrix"
            | "bmatrix"
            | "Bmatrix"
            | "vmatrix"
            | "Vmatrix"
            | "multline"
            | "multline*"
            | "split"
    )
}

fn find_unescaped(
    source: &str,
    mut cursor: usize,
    needle: &str,
    protected: &[Range<usize>],
) -> Option<usize> {
    let bytes = source.as_bytes();
    let first = *needle.as_bytes().first()?;
    let mut protected_index = protected.partition_point(|range| range.end <= cursor);
    let mut escaped = is_escaped(source, cursor);
    while cursor <= source.len().saturating_sub(needle.len()) {
        if let Some(range) = protected.get(protected_index) {
            if cursor >= range.end {
                protected_index += 1;
                continue;
            }
            if range.contains(&cursor) {
                cursor = range.end;
                protected_index += 1;
                escaped = is_escaped(source, cursor);
                continue;
            }
        }

        let segment_end = protected
            .get(protected_index)
            .map_or(source.len(), |range| range.start)
            .min(source.len().saturating_sub(needle.len()) + 1);
        if cursor >= segment_end {
            cursor = segment_end;
            escaped = false;
            continue;
        }

        let Some(relative) = (if first == b'\\' {
            memchr(b'\\', &bytes[cursor..segment_end])
        } else {
            memchr2(b'\\', first, &bytes[cursor..segment_end])
        }) else {
            cursor = segment_end;
            escaped = false;
            continue;
        };
        let candidate = cursor + relative;
        if candidate > cursor {
            escaped = false;
        }

        if bytes[candidate] == b'\\' {
            if first == b'\\' && !escaped && source[candidate..].starts_with(needle) {
                return Some(candidate);
            }
            escaped = !escaped;
        } else {
            if !escaped && source[candidate..].starts_with(needle) {
                return Some(candidate);
            }
            escaped = false;
        }
        cursor = candidate + 1;
    }
    None
}

fn is_escaped(source: &str, index: usize) -> bool {
    source[..index]
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'\\')
        .count()
        % 2
        == 1
}

fn code_ranges(source: &str) -> Vec<Range<usize>> {
    if memchr2(b'`', b'~', source.as_bytes()).is_none() {
        return Vec::new();
    }

    let mut ranges = fenced_code_ranges(source);
    ranges.extend(inline_code_ranges(source, &ranges));
    ranges.sort_unstable_by_key(|range| range.start);
    ranges
}

fn dollar_math_ranges(source: &str, protected: &[Range<usize>]) -> Vec<Range<usize>> {
    if memchr(b'$', source.as_bytes()).is_none() {
        return Vec::new();
    }

    let mut ranges = Vec::new();
    let mut protected_index = 0;
    let mut cursor = 0;
    let mut escaped = false;

    while cursor < source.len() {
        if let Some(range) = protected.get(protected_index) {
            if cursor >= range.end {
                protected_index += 1;
                continue;
            }
            if range.contains(&cursor) {
                cursor = range.end;
                protected_index += 1;
                escaped = is_escaped(source, cursor);
                continue;
            }
        }

        let segment_end = protected
            .get(protected_index)
            .map_or(source.len(), |range| range.start);
        let Some(relative) = memchr2(b'\\', b'$', &source.as_bytes()[cursor..segment_end]) else {
            cursor = segment_end;
            escaped = false;
            continue;
        };
        let candidate = cursor + relative;
        if candidate > cursor {
            escaped = false;
        }
        if source.as_bytes()[candidate] == b'\\' {
            cursor = candidate + 1;
            escaped = !escaped;
            continue;
        }

        let is_display = source[candidate..].starts_with("$$");
        let is_inline = !escaped && !is_display;
        if !is_display && !is_inline {
            cursor = candidate + 1;
            escaped = false;
            continue;
        }

        let (open, close) = if is_display { ("$$", "$$") } else { ("$", "$") };

        let body_start = candidate + open.len();
        if let Some(body_end) = find_unescaped(source, body_start, close, protected) {
            ranges.push(candidate..body_end + close.len());
            cursor = body_end + close.len();
            escaped = is_escaped(source, cursor);
        } else {
            cursor = body_start;
            escaped = is_escaped(source, cursor);
        }
    }
    ranges
}

fn display_dollar_line_break_replacements(
    source: &str,
    ranges: &[Range<usize>],
) -> Vec<Replacement> {
    let mut replacements = Vec::new();
    for range in ranges {
        if range.end.saturating_sub(range.start) < 4 || !source[range.start..].starts_with("$$") {
            continue;
        }

        let body_start = range.start + 2;
        let body_end = range.end - 2;
        replacements.extend(normalize_display_line_breaks(source, body_start, body_end));
    }
    replacements
}

fn normalize_display_line_breaks(
    source: &str,
    body_start: usize,
    body_end: usize,
) -> Vec<Replacement> {
    let mut replacements = Vec::new();
    let mut cursor = body_start;

    while cursor < body_end {
        let Some(relative_newline) = memchr(b'\n', &source.as_bytes()[cursor..body_end]) else {
            break;
        };
        let newline = cursor + relative_newline;
        let newline_start = if newline > body_start && source.as_bytes()[newline - 1] == b'\r' {
            newline - 1
        } else {
            newline
        };
        replacements.push(Replacement {
            range: newline_start..newline + 1,
            text: " ",
        });
        cursor = newline + 1;
    }

    replacements
}

fn fenced_code_ranges(source: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut open: Option<(u8, usize, usize)> = None;
    let mut offset = 0;

    for line_with_ending in source.split_inclusive('\n') {
        let line = line_with_ending.trim_end_matches(['\r', '\n']);
        let indentation = line.bytes().take_while(|byte| *byte == b' ').count();
        let candidate = if indentation <= 3 {
            fence_run(&line[indentation..])
        } else {
            None
        };

        match (open, candidate) {
            (None, Some((marker, length))) if length >= 3 => {
                open = Some((marker, length, offset));
            }
            (Some((marker, length, start)), Some((closing, closing_length)))
                if marker == closing && closing_length >= length =>
            {
                ranges.push(start..offset + line_with_ending.len());
                open = None;
            }
            _ => {}
        }
        offset += line_with_ending.len();
    }

    if let Some((_, _, start)) = open {
        ranges.push(start..source.len());
    }
    ranges
}

fn fence_run(source: &str) -> Option<(u8, usize)> {
    let marker = *source.as_bytes().first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    Some((
        marker,
        source.bytes().take_while(|byte| *byte == marker).count(),
    ))
}

fn inline_code_ranges(source: &str, fenced: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut cursor = 0;
    let bytes = source.as_bytes();

    while cursor < source.len() {
        let fenced_index = fenced.partition_point(|range| range.end <= cursor);
        if let Some(range) = fenced.get(fenced_index)
            && range.contains(&cursor)
        {
            cursor = range.end;
            continue;
        }

        let Some(relative_tick) = memchr(b'`', &bytes[cursor..]) else {
            break;
        };
        cursor += relative_tick;

        let length = bytes[cursor..]
            .iter()
            .take_while(|byte| **byte == b'`')
            .count();
        let delimiter = "`".repeat(length);
        let content_start = cursor + length;
        let line_end = memchr(b'\n', &bytes[content_start..])
            .map_or(source.len(), |relative| content_start + relative);

        if let Some(relative) = source[content_start..line_end].find(&delimiter) {
            let end = content_start + relative + length;
            ranges.push(cursor..end);
            cursor = end;
        } else {
            cursor = content_start;
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::normalize_math_delimiters;

    #[test]
    fn normalizes_bracket_and_parenthesis_math() {
        let source = r#"text \(x^2\)

\[
\begin{aligned}
 a &= b \\
 c &= d
\end{aligned}
\]"#;
        let normalized = normalize_math_delimiters(source);
        assert_eq!(
            normalized,
            r#"text $x^2$

$$ \begin{aligned}  a &= b \\  c &= d \end{aligned} $$"#
        );
    }

    #[test]
    fn leaves_code_and_incomplete_math_untouched() {
        let source = "`\\(code\\)`\n\n```latex\n\\[code\\]\n```\n\n\\[unfinished";
        assert_eq!(normalize_math_delimiters(source), source);
    }

    #[test]
    fn wraps_standalone_display_environments() {
        let source = r"\begin{aligned} a &= b \\ c &= d \end{aligned}";
        assert_eq!(
            normalize_math_delimiters(source),
            r"$$\begin{aligned} a &= b \\ c &= d \end{aligned}$$"
        );
    }

    #[test]
    fn flattens_markdown_line_breaks_inside_display_math() {
        let source = "$$\nfirst\n\n  \nsecond\n$$";
        assert_eq!(normalize_math_delimiters(source), "$$ first     second $$");
    }

    #[test]
    fn preserves_backslash_escape_parity_while_scanning() {
        assert_eq!(
            normalize_math_delimiters(r"\\[not math\\]"),
            r"\\[not math\\]"
        );
        assert_eq!(normalize_math_delimiters(r"\\\[x\]"), r"\\$$x$$");
        assert_eq!(
            normalize_math_delimiters("前置文字 \\[x\\] 后置文字"),
            "前置文字 $$x$$ 后置文字"
        );
    }
}
