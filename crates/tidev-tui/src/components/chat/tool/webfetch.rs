//! Web fetch result display helpers.
//!
//! Strips line-number prefixes and metadata footer lines from webfetch output
//! for clean TUI rendering, while preserving the annotated format in the
//! model-visible message.

// ---------------------------------------------------------------------------
// Content sanitisation
// ---------------------------------------------------------------------------

/// Strip line-number prefixes (e.g. `"1: content"`) and metadata footer lines
/// from a webfetch tool result for TUI display.
///
/// The raw output from [`tidev_tools::builtin::web::fetch`] looks like:
///
/// ```text
/// 1: first line
/// 2: second line
///
/// (Showing lines 1-2 of 100. Use offset=3 to continue.)
/// ```
///
/// After stripping:
///
/// ```text
/// first line
/// second line
/// ```
pub fn strip_webfetch_content(output: &str) -> String {
    let mut result = String::new();
    let mut first = true;

    for line in output.lines() {
        let trimmed = line.trim_start();

        // Skip metadata footer lines
        if trimmed.starts_with("(Showing lines ")
            || trimmed.starts_with("(End of page - total ")
        {
            continue;
        }

        // Strip line number prefix: "N: content" → "content"
        let content = if let Some(digit_end) = line.find(|c: char| !c.is_ascii_digit()) {
            if digit_end > 0 && line.as_bytes().get(digit_end) == Some(&b':') {
                let after_colon = digit_end + 1;
                if after_colon < line.len() && line.as_bytes().get(after_colon) == Some(&b' ') {
                    &line[after_colon + 1..]
                } else {
                    &line[after_colon..]
                }
            } else {
                line
            }
        } else {
            line
        };

        if !first {
            result.push('\n');
        }
        result.push_str(content);
        first = false;
    }

    // Trim trailing newlines that may come from blank lines preceding the
    // metadata footer (the footer itself is already skipped above).
    while result.ends_with('\n') {
        result.pop();
    }

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_basic_pagination() {
        let input = "1: hello\n2: world\n\n(Showing lines 1-2 of 100. Use offset=3 to continue.)";
        assert_eq!(strip_webfetch_content(input), "hello\nworld");
    }

    #[test]
    fn test_strip_full_page() {
        let input = "1: only line\n\n(End of page - total 1 lines)";
        assert_eq!(strip_webfetch_content(input), "only line");
    }

    #[test]
    fn test_no_line_numbers() {
        let input = "plain text\nmore text";
        assert_eq!(strip_webfetch_content(input), "plain text\nmore text");
    }

    #[test]
    fn test_leading_whitespace_content() {
        let input = "1:   indented content\n2:normal";
        assert_eq!(strip_webfetch_content(input), "  indented content\nnormal");
    }

    #[test]
    fn test_multiline_numbers() {
        let input = "10: line ten\n11: line eleven\n\n(End of page - total 2 lines)";
        assert_eq!(strip_webfetch_content(input), "line ten\nline eleven");
    }

    #[test]
    fn test_empty_output() {
        assert_eq!(strip_webfetch_content(""), "");
    }

    #[test]
    fn test_only_metadata() {
        let input = "(End of page - total 0 lines)";
        assert_eq!(strip_webfetch_content(input), "");
    }
}
