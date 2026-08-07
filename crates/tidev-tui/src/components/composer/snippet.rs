//! SnippetState — text snippet insertion for the Composer.
//!
//! Loads snippets from `.tidev/snippets.txt` and `~/.config/tidev/snippets.txt`
//! and provides fuzzy-matched insertion.  When the current word (at cursor)
//! matches a snippet prefix, a popup shows the available completions.

use std::path::Path;

const MAX_SUGGESTIONS: usize = 12;

/// A single snippet with its match metadata.
#[derive(Clone, Debug)]
pub(crate) struct Snippet {
    pub text: String,
    pub score: i64,
}

/// State of the snippet popup.
#[derive(Clone, Debug)]
pub(crate) struct SnippetState {
    pub visible: bool,
    pub query: String,
    pub selected_index: usize,
    pub snippets: Vec<Snippet>,
    snippets_loaded: bool,
    snippets_enabled: bool,
    snippets_cache: Vec<String>,
}

impl SnippetState {
    pub fn new() -> Self {
        Self {
            visible: false,
            query: String::new(),
            selected_index: 0,
            snippets: Vec::new(),
            snippets_loaded: false,
            snippets_enabled: false,
            snippets_cache: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.visible = false;
        self.query.clear();
        self.selected_index = 0;
        self.snippets.clear();
    }

    /// Load snippets from global and workspace config directories.
    pub fn load_snippets(&mut self, workspace_root: &Path, config_dir: &Path) {
        if self.snippets_loaded {
            return;
        }

        let mut snippets = Vec::new();

        // Global snippets: ~/.config/tidev/snippets.txt
        let global_path = config_dir.join("snippets.txt");
        if let Ok(content) = read_text_file(&global_path) {
            Self::parse_snippets_from_content(&content, &mut snippets);
        }

        // Workspace snippets: <workspace_root>/.tidev/snippets.txt
        let workspace_path = workspace_root.join(".tidev").join("snippets.txt");
        if let Ok(content) = read_text_file(&workspace_path) {
            Self::parse_snippets_from_content(&content, &mut snippets);
        }

        self.snippets_cache = snippets;
        self.snippets_loaded = true;
        self.snippets_enabled = !self.snippets_cache.is_empty();
    }

    fn parse_snippets_from_content(content: &str, snippets: &mut Vec<String>) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            snippets.push(trimmed.to_string());
        }
    }

    /// Sync state with the current input and cursor position.
    pub fn sync(&mut self, workspace_root: &Path, config_dir: &Path, input: &str, cursor: usize) {
        self.load_snippets(workspace_root, config_dir);

        if !self.snippets_enabled {
            self.clear();
            return;
        }

        let query = Self::current_word(input, cursor);

        // Minimum 2 characters required to trigger snippets (matches old TUI).
        if query.len() < 2 {
            self.clear();
            return;
        }

        // Try to find the longest suffix of the current word that matches a
        // snippet prefix.  This allows triggering snippets like "你好世界"
        // when typing "请你输出你好" without spacing.
        let mut best_query = String::new();
        let full_word_chars: Vec<char> = query.chars().collect();

        for start_idx in 0..full_word_chars.len() {
            let possible_query: String = full_word_chars[start_idx..].iter().collect();
            if possible_query.is_empty() {
                continue;
            }
            if possible_query.len() < 2 {
                break;
            }

            // Only trigger if the query is a prefix of at least one snippet.
            let query_lower = possible_query.to_lowercase();
            let is_prefix = self
                .snippets_cache
                .iter()
                .any(|snippet| snippet.to_lowercase().starts_with(&query_lower));

            if is_prefix && possible_query.len() > best_query.len() {
                let matched = self.candidates(&possible_query);
                if !matched.is_empty() {
                    self.query = possible_query.clone();
                    best_query = possible_query;
                    self.snippets = matched;
                }
            }
        }

        if best_query.is_empty() {
            self.clear();
            return;
        }

        self.visible = !self.snippets.is_empty();
        self.selected_index = self
            .selected_index
            .min(self.snippets.len().saturating_sub(1));
    }

    /// Apply the currently selected completion.
    /// Returns the replacement text or `None`.
    pub fn apply_completion(&self) -> Option<String> {
        self.snippets
            .get(self.selected_index)
            .map(|s| s.text.clone())
    }

    /// Move the selection by `delta`.
    pub fn move_selection(&mut self, delta: isize) {
        if self.snippets.is_empty() {
            return;
        }
        let len = self.snippets.len() as isize;
        let current = self.selected_index as isize;
        let next = (current + delta).rem_euclid(len);
        self.selected_index = next as usize;
    }

    /// Height of the popup in terminal rows (0 if hidden).
    pub fn popup_height(&self) -> u16 {
        if !self.visible || self.snippets.is_empty() {
            return 0;
        }
        (self.snippets.len() as u16).min(6).saturating_add(2)
    }

    // ── Internal helpers ──────────────────────────────────────────────

    /// Extract the current word at cursor position (going backwards).
    ///
    /// Mirrors the old `tidev_tui::input::snippet::SnippetState::current_word`
    /// behaviour: walks backwards from cursor, collecting characters that are
    /// alphanumeric or `_`, stopping at whitespace or any other non-word char.
    fn current_word(input: &str, cursor: usize) -> String {
        if cursor == 0 {
            return String::new();
        }
        let cursor = cursor.min(input.len());

        // Count complete characters before the cursor byte offset.
        let mut char_count_before = 0;
        for (byte_pos, _) in input.char_indices() {
            if byte_pos >= cursor {
                break;
            }
            char_count_before += 1;
        }

        // Walk backwards from cursor to find word start.
        let chars: Vec<char> = input.chars().collect();
        let mut word_start = char_count_before;

        for i in (0..char_count_before).rev() {
            let c = chars[i];
            if c.is_whitespace() || (!c.is_alphanumeric() && c != '_') {
                word_start = i + 1;
                break;
            }
            word_start = i;
        }

        chars[word_start..char_count_before].iter().collect()
    }

    /// Score and filter snippets that match the query.
    fn candidates(&self, query: &str) -> Vec<Snippet> {
        let mut results: Vec<Snippet> = self
            .snippets_cache
            .iter()
            .filter_map(|snippet| {
                let (score, _) = Self::calculate_score(snippet, query)?;
                Some(Snippet {
                    text: snippet.clone(),
                    score,
                })
            })
            .collect();

        // Sort by score descending, then alphabetically.
        results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.text.cmp(&b.text)));

        results.truncate(MAX_SUGGESTIONS);
        results
    }

    /// Score a snippet against a query.
    /// Returns `None` if there's no match.
    fn calculate_score(snippet: &str, query: &str) -> Option<(i64, Vec<usize>)> {
        let query_lower = query.to_lowercase();
        let snippet_lower = snippet.to_lowercase();

        // Exact match (case-insensitive) — highest score.
        if snippet_lower == query_lower {
            return Some((10_000, (0..query.len()).collect()));
        }

        // Prefix match.
        if snippet_lower.starts_with(&query_lower) {
            let score = 5_000 - (snippet_lower.len() as i64 - query_lower.len() as i64) * 10;
            return Some((score, (0..query.len()).collect()));
        }

        // Fuzzy match: each matching char adds to score.
        let mut si = 0usize;
        let mut qi = 0usize;
        let mut matched_indices = Vec::new();
        let snippet_chars: Vec<char> = snippet_lower.chars().collect();
        let query_chars: Vec<char> = query_lower.chars().collect();

        while si < snippet_chars.len() && qi < query_chars.len() {
            if snippet_chars[si] == query_chars[qi] {
                matched_indices.push(si);
                qi += 1;
            }
            si += 1;
        }

        if qi == query_chars.len() {
            // All query chars matched — score by match density.
            let first = matched_indices.first().copied().unwrap_or(0) as i64;
            let total = matched_indices.last().copied().unwrap_or(0) as i64;
            let score = 1_000 + (matched_indices.len() as i64 * 100) - first * 2 - total;
            Some((score.max(1), matched_indices))
        } else {
            None
        }
    }
}

fn read_text_file(path: &Path) -> anyhow::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(tidev_utils::encoding::decode_text(&bytes)?.into_text())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_word_basic() {
        assert_eq!(SnippetState::current_word("hello world", 0), "");
        assert_eq!(SnippetState::current_word("hello world", 1), "h");
        assert_eq!(SnippetState::current_word("hello world", 2), "he");
        assert_eq!(SnippetState::current_word("hello world", 3), "hel");
        assert_eq!(SnippetState::current_word("hello world", 4), "hell");
        assert_eq!(SnippetState::current_word("hello world", 5), "hello");
        assert_eq!(SnippetState::current_word("hello world", 6), ""); // at space
        assert_eq!(SnippetState::current_word("hello world", 7), "w");
        assert_eq!(SnippetState::current_word("hello world", 8), "wo");
        assert_eq!(SnippetState::current_word("hello world", 9), "wor");
        assert_eq!(SnippetState::current_word("hello world", 10), "worl");
        assert_eq!(SnippetState::current_word("hello world", 11), "world");
    }

    #[test]
    fn test_current_word_special_chars() {
        // "fn hello(" — cursor at '(' (position 8), word before is "hello"
        assert_eq!(SnippetState::current_word("fn hello(", 8), "hello");
        assert_eq!(SnippetState::current_word("test[abc]", 8), "abc");
    }

    #[test]
    fn test_current_word_cjk() {
        // "你好世界" -- 你(0-2), 好(3-5), 世(6-8), 界(9-11)
        assert_eq!(SnippetState::current_word("你好世界", 3), "你");
        assert_eq!(SnippetState::current_word("你好世界", 6), "你好");
        assert_eq!(SnippetState::current_word("你好世界", 9), "你好世");
        assert_eq!(SnippetState::current_word("你好世界", 12), "你好世界");
    }
    #[test]
    fn test_calculate_score_exact() {
        let (score, _) = SnippetState::calculate_score("你好世界", "你好世界").unwrap();
        assert_eq!(score, 10_000);
    }

    #[test]
    fn test_calculate_score_prefix() {
        let (score, _) = SnippetState::calculate_score("你好世界", "你好").unwrap();
        assert!(score > 0);
    }

    #[test]
    fn test_calculate_score_fuzzy() {
        let result = SnippetState::calculate_score("hello world", "hwd");
        assert!(result.is_some());
        let (score, indices) = result.unwrap();
        assert!(score > 0);
        assert!(!indices.is_empty());
    }

    #[test]
    fn test_calculate_score_no_match() {
        let result = SnippetState::calculate_score("hello", "xyz");
        assert!(result.is_none());
    }

    #[test]
    fn test_snippet_loading() {
        // Create temp snippets.txt
        let dir = std::env::temp_dir().join("tidev-test-snippets");
        let _ = std::fs::create_dir_all(&dir);
        let snip_path = dir.join("snippets.txt");
        std::fs::write(&snip_path, "# comments\nhello\nworld\n").ok();

        let mut state = SnippetState::new();
        state.load_snippets(&dir, &dir);
        assert!(!state.snippets_cache.is_empty());
        assert_eq!(state.snippets_cache.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_snippets() {
        let mut snippets = Vec::new();
        SnippetState::parse_snippets_from_content("# comment\nhello\n\nworld\n", &mut snippets);
        assert_eq!(snippets, vec!["hello", "world"]);
    }
}
