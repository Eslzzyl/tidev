// ---------------------------------------------------------------------------
// Read tool metadata helpers
// ---------------------------------------------------------------------------

/// Parsed metadata from a read tool output's XML-style metadata block.
type ReadContentMetadata = Option<(
    (i64, i64),         // line_range (start, end)
    Option<(i64, i64)>, // requested_range (None if model didn't specify)
    i64,                // file_total
    Option<String>,     // truncated_by: None | "size" | "lines"
)>;

/// Parse range string "start-end" into (start, end).
fn parse_range(s: &str) -> Option<(i64, i64)> {
    let parts: Vec<_> = s.split('-').collect();
    if parts.len() == 2 {
        let start = parts[0].trim().parse().ok()?;
        let end = parts[1].trim().parse().ok()?;
        Some((start, end))
    } else {
        None
    }
}

/// Parse line range from read tool output (e.g., "Showing lines 10-50 of 100")
pub(super) fn parse_line_range_from_read_output(output: &str) -> Option<(i64, i64)> {
    // Match patterns like "Showing lines 10-50 of 100" or "Showing lines 10-50"
    if let Some(start) = output.find("Showing lines ") {
        let after_prefix = &output[start + 14..];
        // Parse start number
        let mut end_idx = 0;
        let mut start_num = 0i64;
        for (i, c) in after_prefix.chars().enumerate() {
            if c.is_ascii_digit() {
                start_num = start_num * 10 + (c as i64 - '0' as i64);
                end_idx = i;
            } else {
                break;
            }
        }
        // Look for "-{end}" after "Showing lines {start}-"
        let after_start = &after_prefix[end_idx + 1..];
        if let Some(stripped) = after_start.strip_prefix('-') {
            let mut end_num = 0i64;
            for c in stripped.chars() {
                if c.is_ascii_digit() {
                    end_num = end_num * 10 + (c as i64 - '0' as i64);
                } else {
                    break;
                }
            }
            if end_num > start_num {
                return Some((start_num, end_num));
            }
        }
    }
    None
}

/// Parse XML-style metadata from read tool output.
///
/// Reads `<line_range>`, `<requested_range>`, `<file_total>`, and
/// `<truncated_by>` tags from the output to build structured metadata.
pub(super) fn parse_read_content_metadata(content: &str) -> ReadContentMetadata {
    let mut line_range = None;
    let mut requested_range = None;
    let mut file_total = None;
    let mut truncated_by = None;

    for line in content.lines() {
        if let Some(val) = line.strip_prefix("<line_range>") {
            if let Some(end) = val.strip_suffix("</line_range>") {
                line_range = parse_range(end);
            }
        } else if let Some(val) = line.strip_prefix("<requested_range>") {
            if let Some(end) = val.strip_suffix("</requested_range>") {
                requested_range = parse_range(end);
            }
        } else if let Some(val) = line.strip_prefix("<file_total>") {
            if let Some(end) = val.strip_suffix("</file_total>") {
                file_total = end.trim().parse().ok();
            }
        } else if let Some(val) = line.strip_prefix("<truncated_by>")
            && let Some(end) = val.strip_suffix("</truncated_by>")
            && matches!(end.trim(), "size" | "lines")
        {
            truncated_by = Some(end.trim().to_string());
        }
    }

    match (line_range, file_total) {
        (Some(lr), Some(ft)) => Some((lr, requested_range, ft, truncated_by)),
        _ => None,
    }
}

/// Format the label for a read tool result (file content).
///
/// Handles three cases:
/// - 50KB size truncation (`is_size_truncated`)
/// - 2000-line cap truncation (`truncated_by == "lines"`)
/// - No truncation
///
/// When a specific range was requested **and** all requested lines were returned,
/// the truncation label is omitted even if the 50KB cap was triggered internally,
/// because the cap only affected content beyond what the user asked for.
pub(super) fn format_read_result_label(
    start: i64,
    end: i64,
    total: i64,
    requested_range: Option<(i64, i64)>,
    truncated_by: Option<&str>,
    is_size_truncated: bool,
) -> String {
    let is_full_file = start == 1 && end == total;

    // Determine the truncation suffix (if any) — the user may have gotten
    // everything they asked for even when the tool hit the 50KB cap.
    let trunc_suffix = if is_size_truncated {
        if let Some((_req_start, req_end)) = requested_range {
            if end >= req_end {
                None // All requested lines returned; cap only affected lines beyond
            } else {
                Some("truncated due to 50KB cap")
            }
        } else {
            Some("truncated due to 50KB cap")
        }
    } else if truncated_by == Some("lines") {
        Some("truncated due to 2000 lines cap")
    } else {
        None
    };

    match (is_full_file, requested_range, trunc_suffix) {
        // Full file — no truncation possible when all lines are returned
        (true, _, _) => format!(" → All {} lines", total),
        // Partial read without truncation
        (false, None, None) => {
            format!(" → Line {start}-{end} of {total}")
        }
        (false, Some((req_start, req_end)), None) => {
            format!(" → Line {start}-{end} of {total} (requested {req_start}-{req_end})")
        }
        // Partial read with truncation
        (false, None, Some(suffix)) => {
            format!(" → Line {start}-{end} of {total} (requested all lines, {suffix})")
        }
        (false, Some((req_start, req_end)), Some(suffix)) => {
            format!(" → Line {start}-{end} of {total} (requested {req_start}-{req_end}, {suffix})")
        }
    }
}

/// Format a file size in human-readable form.
pub(crate) fn format_file_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[unit_idx])
    } else {
        format!("{:.1} {}", size, UNITS[unit_idx])
    }
}
