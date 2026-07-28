/// Stream-aware `<think>…</think>` / `<thinking>…</thinking>` tag parser used to
/// separate chain-of-thought reasoning from visible assistant text in streaming
/// LLM responses.
#[derive(Clone, Debug, Default)]
pub(crate) struct ThinkParser {
    in_think: bool,
    buffer: String,
}

// Tags to match (longer first so suffix detection works correctly).
const OPEN_TAGS: &[&str] = &["<thinking>", "<think>"];
const CLOSE_TAGS: &[&str] = &["</thinking>", "</think>"];

/// Strip all `<think>`, `</think>`, `<thinking>`, `</thinking>` tags from `text`.
///
/// This is used for `reasoning_content` that bypasses the streaming parser but
/// may still contain tag-delimited sections (e.g. interleaved thinking).
pub(crate) fn strip_think_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut pos = 0;
    let bytes = text.as_bytes();

    while pos < bytes.len() {
        // Look for any opening or closing tag at current position
        let mut found = false;
        for tag in OPEN_TAGS.iter().chain(CLOSE_TAGS) {
            let tag_bytes = tag.as_bytes();
            if bytes[pos..].starts_with(tag_bytes) {
                pos += tag_bytes.len();
                found = true;
                break;
            }
        }
        if !found {
            // SAFETY: `text` is a valid `&str`, so it is guaranteed to contain
            // a valid UTF-8 sequence starting at `pos`.
            let ch = text[pos..].chars().next().unwrap();
            result.push(ch);
            pos += ch.len_utf8();
        }
    }

    result
}

impl ThinkParser {
    /// Push a chunk of streaming text and return `(visible, reasoning)`.
    pub(crate) fn push(&mut self, text: &str) -> (String, String) {
        self.buffer.push_str(text);

        let mut visible = String::new();
        let mut reasoning = String::new();

        loop {
            if self.in_think {
                if let Some(end) = find_any_tag(&self.buffer, CLOSE_TAGS) {
                    reasoning.push_str(&self.buffer[..end]);
                    // Drain past the matched close tag
                    let tag = close_tag_at(&self.buffer[end..]);
                    self.buffer.drain(..end + tag.len());
                    self.in_think = false;
                    continue;
                }

                let keep = think_tag_suffix_len(&self.buffer);
                let split = self.buffer.len().saturating_sub(keep);
                reasoning.push_str(&self.buffer[..split]);
                self.buffer.drain(..split);
                break;
            }

            if let Some(start) = find_any_tag(&self.buffer, OPEN_TAGS) {
                visible.push_str(&self.buffer[..start]);
                // Drain past the matched open tag
                let tag = open_tag_at(&self.buffer[start..]);
                self.buffer.drain(..start + tag.len());
                self.in_think = true;
                continue;
            }

            let keep = think_tag_suffix_len(&self.buffer);
            let split = self.buffer.len().saturating_sub(keep);
            visible.push_str(&self.buffer[..split]);
            self.buffer.drain(..split);
            break;
        }

        (visible, reasoning)
    }

    /// Drain any remaining buffered text. Call this after the stream ends.
    pub(crate) fn finish(&mut self) -> (String, String) {
        let mut visible = String::new();
        let mut reasoning = String::new();

        if self.in_think {
            reasoning.push_str(&self.buffer);
        } else {
            visible.push_str(&self.buffer);
        }

        self.buffer.clear();
        (visible, reasoning)
    }
}

/// Returns the maximum suffix of `text` that could be a partial `<think>` or
/// `</think>` tag.  This prevents splitting a multi-chunk tag boundary.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passthrough() {
        let mut p = ThinkParser::default();
        let (v, r) = p.push("Hello, world!");
        assert_eq!(v, "Hello, world!");
        assert!(r.is_empty());
        let (v, r) = p.finish();
        assert!(v.is_empty());
        assert!(r.is_empty());
    }

    #[test]
    fn full_think_tag_in_one_chunk() {
        let mut p = ThinkParser::default();
        let (v, r) = p.push("before <think>thinking text</think> after");
        assert_eq!(v, "before  after");
        assert_eq!(r, "thinking text");
    }

    #[test]
    fn think_tag_split_across_chunks() {
        let mut p = ThinkParser::default();
        // "<thin" is kept as pending prefix of "<think>", "before " flushes
        let (v, r) = p.push("before <thin");
        assert_eq!(v, "before ");
        assert!(r.is_empty());

        // completes "<think>", enters think mode, then "</thin" kept as pending
        let (v, r) = p.push("k>thinking text</thin");
        assert!(v.is_empty());
        assert_eq!(r, "thinking text");

        // completes "</think>", exits think mode, " after" flushes
        let (v, r) = p.push("k> after");
        assert_eq!(v, " after");
        assert!(r.is_empty());
    }

    #[test]
    fn finish_drains_remaining_think() {
        let mut p = ThinkParser::default();
        let (v, r) = p.push("hello <think>unclosed");
        // "hello " flushed before <think>, "unclosed" flushed as reasoning
        // (in think mode, residual text is reasoning even without close tag)
        assert_eq!(v, "hello ");
        assert_eq!(r, "unclosed");
        let (v, r) = p.finish();
        assert!(v.is_empty());
        assert!(r.is_empty()); // already drained by push
    }

    #[test]
    fn finish_drains_remaining_visible_no_think() {
        let mut p = ThinkParser::default();
        let (v, r) = p.push("just visible text");
        assert_eq!(v, "just visible text");
        assert!(r.is_empty());
        let (v, r) = p.finish();
        assert!(v.is_empty());
        assert!(r.is_empty());
    }

    #[test]
    fn empty_content() {
        let mut p = ThinkParser::default();
        let (v, r) = p.push("");
        assert!(v.is_empty());
        assert!(r.is_empty());
    }

    #[test]
    fn multiple_think_tags() {
        let mut p = ThinkParser::default();
        let (v, r) = p.push("<think>first</think> visible <think>second</think> end");
        assert_eq!(v, " visible  end");
        assert_eq!(r, "firstsecond");
    }

    #[test]
    fn close_without_open_is_visible() {
        let mut p = ThinkParser::default();
        let (v, r) = p.push("</think>");
        assert_eq!(v, "</think>");
        assert!(r.is_empty());
    }

    #[test]
    fn thinking_tag_in_one_chunk() {
        let mut p = ThinkParser::default();
        let (v, r) = p.push("before <thinking>reasoning text</thinking> after");
        assert_eq!(v, "before  after");
        assert_eq!(r, "reasoning text");
    }

    #[test]
    fn thinking_tag_split_across_chunks() {
        let mut p = ThinkParser::default();
        let (v, r) = p.push("before <thin");
        assert_eq!(v, "before ");
        assert!(r.is_empty());

        let (v, r) = p.push("king>thinking text</thin");
        assert!(v.is_empty());
        assert_eq!(r, "thinking text");

        let (v, r) = p.push("king> after");
        assert_eq!(v, " after");
        assert!(r.is_empty());
    }

    #[test]
    fn multiple_mixed_think_thinking_tags() {
        let mut p = ThinkParser::default();
        let (v, r) = p.push("<think>first</think> visible <thinking>second</thinking> end");
        assert_eq!(v, " visible  end");
        assert_eq!(r, "firstsecond");
    }

    #[test]
    fn strip_think_tags_removes_think_and_thinking() {
        assert_eq!(strip_think_tags("<think>some text</think>"), "some text");
        assert_eq!(
            strip_think_tags("<thinking>some text</thinking>"),
            "some text"
        );
        assert_eq!(
            strip_think_tags("before <think>a</think> after <thinking>b</thinking> end"),
            "before a after b end"
        );
    }

    #[test]
    fn strip_think_tags_preserves_plain_text() {
        assert_eq!(strip_think_tags("hello world"), "hello world");
        assert_eq!(strip_think_tags(""), "");
        assert_eq!(strip_think_tags("no tags here"), "no tags here");
    }

    #[test]
    fn strip_think_tags_preserves_non_ascii() {
        // Non-ASCII characters (Chinese) must survive intact.
        let input = "错误处理：prompt 提交失败时返回 EndTurn";
        assert_eq!(strip_think_tags(input), input);

        // Mixed non-ASCII with think tags.
        assert_eq!(
            strip_think_tags("<think>错误处理</think>完成"),
            "错误处理完成"
        );
    }
}

fn think_tag_suffix_len(text: &str) -> usize {
    for tag in OPEN_TAGS.iter().chain(CLOSE_TAGS) {
        let max = tag.len().saturating_sub(1);
        for keep in (1..=max).rev() {
            if text.ends_with(&tag[..keep]) {
                return keep;
            }
        }
    }

    0
}

/// Find the earliest occurrence of any of the given `tags` in `text`.
/// Returns the byte offset of the match, or `None`.
fn find_any_tag(text: &str, tags: &[&str]) -> Option<usize> {
    let mut earliest: Option<usize> = None;
    for tag in tags {
        if let Some(pos) = text.find(tag) {
            match earliest {
                None => earliest = Some(pos),
                Some(prev) if pos < prev => earliest = Some(pos),
                _ => {}
            }
        }
    }
    earliest
}

/// Return which close tag is at the start of `text` (assumes one matches).
fn close_tag_at(text: &str) -> &'static str {
    for tag in CLOSE_TAGS {
        if text.starts_with(tag) {
            return tag;
        }
    }
    CLOSE_TAGS[0] // fallback, should never reach here
}

/// Return which open tag is at the start of `text` (assumes one matches).
fn open_tag_at(text: &str) -> &'static str {
    for tag in OPEN_TAGS {
        if text.starts_with(tag) {
            return tag;
        }
    }
    OPEN_TAGS[0] // fallback, should never reach here
}
