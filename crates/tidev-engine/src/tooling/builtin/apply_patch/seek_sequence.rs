//! Port of codex's seek_sequence.rs — fuzzy line matching.
//!
//! Attempt to find the sequence of `pattern` lines within `lines` beginning at
//! or after `start`. Returns the starting index of the match or `None` if not
//! found. Matches are attempted with decreasing strictness: exact match, then
//! ignoring trailing whitespace, then ignoring leading and trailing whitespace.
//!
//! When `eof` is true, we first try starting at the end-of-file (so that
//! patterns intended to match file endings are applied at the end), and fall
//! back to searching from `start` if needed.

/// Special cases handled defensively:
///  • Empty `pattern` → returns `Some(start)` (no-op match)
///  • `pattern.len() > lines.len()` → returns `None` (cannot match, avoids
///    out‑of‑bounds panic)
pub(crate) fn seek_sequence(
    lines: &[String],
    pattern: &[String],
    start: usize,
    eof: bool,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start);
    }

    if pattern.len() > lines.len() {
        return None;
    }

    let search_start = if eof && lines.len() >= pattern.len() {
        lines.len() - pattern.len()
    } else {
        start
    };

    // 1. Exact match.
    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        if lines[i..i + pattern.len()] == *pattern {
            return Some(i);
        }
    }

    // 2. Rstrip match (ignore trailing whitespace).
    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        let mut ok = true;
        for (p_idx, pat) in pattern.iter().enumerate() {
            if lines[i + p_idx].trim_end() != pat.trim_end() {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(i);
        }
    }

    // 3. Trim match (ignore leading and trailing whitespace).
    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        let mut ok = true;
        for (p_idx, pat) in pattern.iter().enumerate() {
            if lines[i + p_idx].trim() != pat.trim() {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(i);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ls(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_empty_pattern() {
        let lines = ls(&["a", "b", "c"]);
        assert_eq!(seek_sequence(&lines, &[], 0, false), Some(0));
        assert_eq!(seek_sequence(&lines, &[], 2, false), Some(2));
    }

    #[test]
    fn test_exact_match() {
        let lines = ls(&["foo", "bar", "baz"]);
        assert_eq!(seek_sequence(&lines, &["bar".to_string()], 0, false), Some(1));
    }

    #[test]
    fn test_trim_end_match() {
        let lines = ls(&["foo  ", "bar  "]);
        assert_eq!(seek_sequence(&lines, &["foo".to_string()], 0, false), Some(0));
    }

    #[test]
    fn test_trim_both_match() {
        let lines = ls(&["  foo", "  bar"]);
        assert_eq!(seek_sequence(&lines, &["foo".to_string()], 0, false), Some(0));
    }

    #[test]
    fn test_pattern_longer_than_lines() {
        let lines = ls(&["a", "b"]);
        assert_eq!(seek_sequence(&lines, &["a".to_string(), "b".to_string(), "c".to_string()], 0, false), None);
    }

    #[test]
    fn test_eof_mode() {
        let lines = ls(&["a", "b", "c", "d", "e"]);
        // Pattern "d", "e" appears at end, eof=true should find it at index 3.
        assert_eq!(seek_sequence(&lines, &["d".to_string(), "e".to_string()], 0, true), Some(3));
    }

    #[test]
    fn test_not_found() {
        let lines = ls(&["a", "b", "c"]);
        assert_eq!(seek_sequence(&lines, &["x".to_string()], 0, false), None);
    }
}
