//! Rendering and parsing helpers for collapsed tool diffs and streamed patch arguments.

use diffy::{Line as DiffLine, Patch};
use ratatui::prelude::{Modifier, Style};
use ratatui::text::{Line, Span};
use tidev_llm::message::Message;

use super::utils::tool_output_is_error;
use super::{hyper_lines, pluralize, wrap_tool_title};
use crate::diff_render::split_diff_sections;
use crate::hyperlink::HyperlinkLine;
use crate::theme::ThemePalette;

// ---------------------------------------------------------------------------
// Collapsed diff summary (write/edit/apply_patch)
// ---------------------------------------------------------------------------

/// Whether a tool result carries renderable diff data (structured file
/// changes or a unified diff in metadata). Used to decide whether a diff
/// card is foldable at all.
pub(crate) fn tool_result_has_diff(result_msg: &Message) -> bool {
    !result_msg.metadata.file_changes.is_empty() || result_msg.metadata.diff.is_some()
}

/// Build collapsed per-file summary lines for edit/write/apply_patch results.
///
/// Each file becomes a single line: operation label, path, and +N/-M counts
/// (zero sides omitted). Returns None when no diff is available or the result
/// is an error, so callers fall back to the normal rendering.
pub(super) fn render_diff_summary_lines(
    message: &Message,
    content_width: usize,
    palette: ThemePalette,
    canonical_name: &str,
) -> Option<Vec<HyperlinkLine>> {
    let output = crate::utils::strip_system_reminder_tags(&message.content);
    if tool_output_is_error(&output) {
        return None;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut first = true;

    // Structured per-file changes (apply_patch): exact paths and operations.
    if canonical_name == "apply_patch" && !message.metadata.file_changes.is_empty() {
        lines.extend(build_apply_patch_summary_header(
            message.metadata.file_changes.len(),
            palette,
            content_width,
        ));
        first = false;
        for change in &message.metadata.file_changes {
            let label = match change.operation.as_str() {
                "A" => "Write",
                "M" => "Edit",
                "D" => "Delete",
                _ => "Edit",
            };
            let (adds, dels) = change
                .diff
                .as_deref()
                .and_then(count_diff_section_lines)
                .unwrap_or((0, 0));
            push_diff_summary_line(
                &mut lines,
                &mut first,
                label,
                change.path.clone(),
                adds,
                dels,
                palette,
                content_width,
            );
        }
        return Some(hyper_lines(lines));
    }

    // Unified diff in metadata (edit/write; apply_patch fallback).
    if let Some(diff) = message.metadata.diff.as_ref() {
        for section in split_diff_sections(diff) {
            if let Some((adds, dels)) = count_diff_section_lines(&section) {
                let path = diff_section_file_path(&section)
                    .or_else(|| message.metadata.filepath.clone())
                    .unwrap_or_else(|| "(unknown)".to_string());
                push_diff_summary_line(
                    &mut lines,
                    &mut first,
                    diff_section_operation(&section),
                    path,
                    adds,
                    dels,
                    palette,
                    content_width,
                );
            }
        }
        return if lines.is_empty() {
            None
        } else {
            Some(hyper_lines(lines))
        };
    }

    // Output fallback: only used when the text actually parses as a diff.
    let mut any = false;
    for section in split_diff_sections(&output) {
        if let Some((adds, dels)) = count_diff_section_lines(&section) {
            any = true;
            let path = diff_section_file_path(&section)
                .or_else(|| message.metadata.filepath.clone())
                .unwrap_or_else(|| "(unknown)".to_string());
            push_diff_summary_line(
                &mut lines,
                &mut first,
                diff_section_operation(&section),
                path,
                adds,
                dels,
                palette,
                content_width,
            );
        }
    }
    any.then(|| hyper_lines(lines))
}

/// Append one per-file summary line, carrying the ▶ fold indicator on the
/// first line of the card.
#[allow(clippy::too_many_arguments)]
fn push_diff_summary_line(
    lines: &mut Vec<Line<'static>>,
    is_first: &mut bool,
    label: &str,
    path: String,
    adds: usize,
    dels: usize,
    palette: ThemePalette,
    content_width: usize,
) {
    lines.extend(build_diff_summary_line(
        label,
        &path,
        adds,
        dels,
        palette,
        if *is_first { Some("▶") } else { None },
        content_width,
    ));
    *is_first = false;
}

/// Build the collapsed apply_patch header. The tool identity is shown once
/// here; individual rows below only describe the file operation.
fn build_apply_patch_summary_header(
    file_count: usize,
    palette: ThemePalette,
    content_width: usize,
) -> Vec<Line<'static>> {
    wrap_tool_title(
        Line::from(vec![
            Span::styled("Apply patch ", Style::default().fg(palette.accent_soft)),
            Span::styled(
                format!("· {}", pluralize(file_count, "file", "files")),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ▶", Style::default().fg(palette.muted)),
        ]),
        content_width,
        "  ",
    )
}

/// Build a single wrapped summary line for one file:
/// `Edit  src/main.rs  +12 -3  ▶` (zero count sides omitted).
fn build_diff_summary_line(
    label: &str,
    path: &str,
    adds: usize,
    dels: usize,
    palette: ThemePalette,
    fold_indicator: Option<&str>,
    content_width: usize,
) -> Vec<Line<'static>> {
    let mut spans = vec![
        Span::styled(
            format!("{} ", label),
            Style::default().fg(palette.accent_soft),
        ),
        Span::styled(
            path.to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if adds > 0 {
        spans.push(Span::styled(
            format!("  +{}", adds),
            Style::default().fg(palette.diff_add),
        ));
    }
    if dels > 0 {
        spans.push(Span::styled(
            format!("  -{}", dels),
            Style::default().fg(palette.diff_delete),
        ));
    }
    if let Some(indicator) = fold_indicator {
        spans.push(Span::styled(
            format!("  {}", indicator),
            Style::default().fg(palette.muted),
        ));
    }
    wrap_tool_title(Line::from(spans), content_width, "  ")
}

/// Count added/deleted lines in a unified diff section.
///
/// Parses with diffy for complete diffs; falls back to a prefix scan for
/// truncated output, but only when the text actually looks like a diff so
/// arbitrary tool output never produces bogus counts.
pub(super) fn count_diff_section_lines(section: &str) -> Option<(usize, usize)> {
    if let Ok(patch) = Patch::from_str(section) {
        // diffy parses arbitrary text as an empty patch; without hunks the
        // text is not a real diff.
        if patch.hunks().is_empty() {
            return None;
        }
        let mut adds = 0usize;
        let mut dels = 0usize;
        for hunk in patch.hunks() {
            for line in hunk.lines() {
                match line {
                    DiffLine::Insert(_) => adds += 1,
                    DiffLine::Delete(_) => dels += 1,
                    DiffLine::Context(_) => {}
                }
            }
        }
        return Some((adds, dels));
    }
    let looks_like_diff = section.lines().any(|l| {
        l.starts_with("diff --git")
            || l.starts_with("--- ")
            || l.starts_with("+++ ")
            || l.starts_with("@@")
    });
    if !looks_like_diff {
        return None;
    }
    let mut adds = 0usize;
    let mut dels = 0usize;
    for line in section.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            adds += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            dels += 1;
        }
    }
    Some((adds, dels))
}

/// Extract the file path from a unified diff section, preferring the new
/// (b/) side and skipping `/dev/null` placeholders.
pub(super) fn diff_section_file_path(section: &str) -> Option<String> {
    // `diff --git a/foo b/foo` header (paths may be quoted when spaced).
    for line in section.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            for token in rest.split_whitespace() {
                let token = token.trim_matches('"');
                if let Some(path) = token.strip_prefix("b/") {
                    return Some(path.to_string());
                }
            }
            for token in rest.split_whitespace() {
                let token = token.trim_matches('"');
                if let Some(path) = token.strip_prefix("a/") {
                    return Some(path.to_string());
                }
            }
        }
    }
    // `+++ b/foo` / `--- a/foo` headers.
    for line in section.lines() {
        if let Some(path) = line.strip_prefix("+++ ").map(str::trim)
            && path != "/dev/null"
            && !path.is_empty()
        {
            return Some(path.strip_prefix("b/").unwrap_or(path).to_string());
        }
    }
    for line in section.lines() {
        if let Some(path) = line.strip_prefix("--- ").map(str::trim)
            && path != "/dev/null"
            && !path.is_empty()
        {
            return Some(path.strip_prefix("a/").unwrap_or(path).to_string());
        }
    }
    None
}

/// Infer the operation label for a diff section from its `---`/`+++`
/// headers: Write for new files, Delete for removed files, Edit otherwise.
pub(super) fn diff_section_operation(section: &str) -> &'static str {
    let mut has_old = false;
    let mut has_new = false;
    for line in section.lines() {
        if let Some(path) = line.strip_prefix("--- ").map(str::trim)
            && path != "/dev/null"
            && !path.is_empty()
        {
            has_old = true;
        } else if let Some(path) = line.strip_prefix("+++ ").map(str::trim)
            && path != "/dev/null"
            && !path.is_empty()
        {
            has_new = true;
        }
    }
    match (has_old, has_new) {
        (false, true) => "Write",
        (true, false) => "Delete",
        _ => "Edit",
    }
}

/// Count lines in a partial JSON string field (works on incomplete JSON during streaming).
///
/// Searches for `"field":` and reads the string value, counting both JSON `\n` escapes
/// and literal newlines. Handles escaped quotes correctly.
pub(super) fn count_lines_in_partial_json(args: &str, field: &str) -> usize {
    let key = format!("\"{}\":", field);
    if let Some(start) = args.find(&key) {
        let after_colon = &args[start + key.len()..];
        let value_start = after_colon.trim_start();
        if !value_start.starts_with('"') {
            return 0;
        }
        let rest = &value_start[1..]; // skip opening quote

        // Single pass: find closing quote while counting newlines
        let mut i = 0usize;
        let mut newlines = 0usize;
        let bytes = rest.as_bytes();
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                // JSON escape: check for \n, then skip 2 bytes
                if i + 1 < bytes.len() && bytes[i + 1] == b'n' {
                    newlines += 1;
                }
                i += 2;
            } else if bytes[i] == b'"' {
                break; // unescaped quote = end of value
            } else {
                if bytes[i] == b'\n' {
                    newlines += 1;
                }
                i += 1;
            }
        }
        if i == 0 {
            return 0;
        }
        newlines + 1
    } else {
        0
    }
}

/// Count patch changes (additions, deletions, file operations) from a partial
/// `patch_text` JSON field. Returns `(additions, deletions, file_ops)`.
///
/// This works on incomplete JSON during LLM streaming — it finds the string
/// value for `"patch_text":` and counts `+` lines, `-` lines, and `***` file
/// operation markers inside it.
pub(super) fn count_patch_changes(args: &str) -> (usize, usize, usize) {
    let key = "\"patch_text\":";
    let start = match args.find(key) {
        Some(s) => s,
        None => return (0, 0, 0),
    };
    let after_colon = &args[start + key.len()..];
    let value_start = after_colon.trim_start();
    if !value_start.starts_with('"') {
        return (0, 0, 0);
    }
    let rest = &value_start[1..]; // skip opening quote

    let mut i = 0usize;
    let mut adds = 0usize;
    let mut dels = 0usize;
    let mut ops = 0usize;
    let bytes = rest.as_bytes();
    let mut line_start = true;
    let mut was_cr = false; // track \r for \r\n sequences

    while i < bytes.len() {
        if bytes[i] == b'\\' {
            // JSON escape: check for \n (which represents a literal newline
            // in JSON), then process the decoded line.
            if i + 1 < bytes.len() && bytes[i + 1] == b'n' {
                line_start = true;
            }
            // Skip other escapes like \\, \", \t, etc.
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            // Unescaped quote = end of value (or nested JSON — skip)
            break;
        }
        if bytes[i] == b'\r' {
            was_cr = true;
            line_start = true;
            i += 1;
            continue;
        }
        if bytes[i] == b'\n' {
            was_cr = false;
            line_start = true;
            i += 1;
            continue;
        }
        if line_start && !was_cr {
            match bytes[i] {
                b'+' => adds += 1,
                b'-' => dels += 1,
                b'*'
                    // Check if this starts a *** marker like *** Update File:
                    if bytes[i..].starts_with(b"*** ") => {
                        ops += 1;
                    }
                _ => {}
            }
        }
        if bytes[i] == b'\n' || bytes[i] == b'\r' {
            // handled above
        } else {
            line_start = false;
        }
        was_cr = false;
        i += 1;
    }

    (adds, dels, ops)
}

/// Extract file paths from completed or partially streamed `patch_text` input.
///
/// The tool arguments are JSON, so `patch_text` commonly arrives with newline
/// escapes while the overall JSON object is still incomplete. Decode only the
/// string value that has arrived so the patch title can update immediately.
pub(super) fn partial_patch_file_paths(args: &str) -> Vec<String> {
    const KEY: &str = "\"patch_text\":";

    let Some(start) = args.find(KEY) else {
        return Vec::new();
    };
    let value_start = args[start + KEY.len()..].trim_start();
    let Some(value_start) = value_start.strip_prefix('"') else {
        return Vec::new();
    };

    let mut patch_text = String::new();
    let mut chars = value_start.chars();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            break;
        }
        if ch != '\\' {
            patch_text.push(ch);
            continue;
        }

        match chars.next() {
            Some('n') => patch_text.push('\n'),
            Some('r') => patch_text.push('\r'),
            Some('t') => patch_text.push('\t'),
            Some('"') => patch_text.push('"'),
            Some('\\') => patch_text.push('\\'),
            Some('b') => patch_text.push('\u{0008}'),
            Some('f') => patch_text.push('\u{000C}'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                if let Ok(codepoint) = u32::from_str_radix(&hex, 16)
                    && let Some(decoded) = char::from_u32(codepoint)
                {
                    patch_text.push(decoded);
                }
            }
            Some(other) => patch_text.push(other),
            None => break,
        }
    }

    patch_file_paths(&patch_text)
}

pub(super) fn patch_file_paths(patch_text: &str) -> Vec<String> {
    patch_text
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("*** Add File: ")
                .or_else(|| trimmed.strip_prefix("*** Update File: "))
                .or_else(|| trimmed.strip_prefix("*** Delete File: "))
        })
        .map(str::to_string)
        .collect()
}

pub(crate) fn tool_call_arguments_are_complete(arguments: &str) -> bool {
    if arguments.trim().is_empty() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(arguments).is_ok()
}
