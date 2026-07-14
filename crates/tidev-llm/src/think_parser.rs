/// Stream-aware `<think>…</think>` tag parser used to separate chain-of-thought
/// reasoning from visible assistant text in streaming LLM responses.
#[derive(Clone, Debug, Default)]
pub(crate) struct ThinkParser {
    in_think: bool,
    buffer: String,
}

impl ThinkParser {
    /// Push a chunk of streaming text and return `(visible, reasoning)`.
    pub(crate) fn push(&mut self, text: &str) -> (String, String) {
        self.buffer.push_str(text);

        let mut visible = String::new();
        let mut reasoning = String::new();

        loop {
            if self.in_think {
                if let Some(end) = self.buffer.find("</think>") {
                    reasoning.push_str(&self.buffer[..end]);
                    self.buffer.drain(..end + "</think>".len());
                    self.in_think = false;
                    continue;
                }

                let keep = think_tag_suffix_len(&self.buffer);
                let split = self.buffer.len().saturating_sub(keep);
                reasoning.push_str(&self.buffer[..split]);
                self.buffer.drain(..split);
                break;
            }

            if let Some(start) = self.buffer.find("<think>") {
                visible.push_str(&self.buffer[..start]);
                self.buffer.drain(..start + "<think>".len());
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
}

fn think_tag_suffix_len(text: &str) -> usize {
    const TAGS: [&str; 2] = ["</think>", "<think>"];

    for tag in TAGS {
        let max = tag.len().saturating_sub(1);
        for keep in (1..=max).rev() {
            if text.ends_with(&tag[..keep]) {
                return keep;
            }
        }
    }

    0
}
