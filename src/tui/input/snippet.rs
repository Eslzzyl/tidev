use std::path::Path;

const MAX_SUGGESTIONS: usize = 12;

#[derive(Clone, Debug)]
pub struct Snippet {
    pub text: String,
    pub matched_indices: Vec<usize>,
    pub score: i64,
}

#[derive(Clone, Debug, Default)]
pub struct SnippetState {
    pub visible: bool,
    pub query: String,
    pub selected_index: usize,
    pub snippets: Vec<Snippet>,
    snippets_loaded: bool,
    snippets_enabled: bool,
    snippets_cache: Vec<String>,
}

impl SnippetState {
    pub fn clear(&mut self) {
        self.visible = false;
        self.query.clear();
        self.selected_index = 0;
        self.snippets.clear();
    }

    /// Returns whether snippets are available and the feature should run.
    /// Note: This only returns true if snippets have been loaded AND found.
    /// For checking if snippets *should* be attempted to load, use needs_load() instead.
    pub fn is_enabled(&self) -> bool {
        // If not loaded yet, we can't determine - return false to let sync handle loading
        if !self.snippets_loaded {
            return false;
        }
        self.snippets_enabled
    }

    /// Returns whether snippets need to be loaded (hasn't been loaded yet)
    pub fn needs_load(&self) -> bool {
        !self.snippets_loaded
    }

    pub fn load_snippets(&mut self, workspace_root: &Path, config_dir: &Path) {
        if self.snippets_loaded {
            return;
        }

        let mut snippets = Vec::new();

        // Load global snippets: ~/.config/tidev/snippets.txt
        let global_path = config_dir.join("snippets.txt");
        if let Ok(content) = std::fs::read_to_string(&global_path) {
            Self::parse_snippets_from_content(&content, &mut snippets);
        }

        // Load workspace snippets: <workspace_root>/.tidev/snippets.txt
        let workspace_path = workspace_root.join(".tidev").join("snippets.txt");
        if let Ok(content) = std::fs::read_to_string(&workspace_path) {
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

    pub fn sync(&mut self, workspace_root: &Path, config_dir: &Path, input: &str, cursor: usize) {
        // Ensure snippets are loaded
        self.load_snippets(workspace_root, config_dir);

        // If no snippets available, disable the feature entirely
        if !self.snippets_enabled {
            self.clear();
            return;
        }

        // Extract current word (from cursor position going backwards)
        let mut query = Self::current_word(input, cursor);

        // Try to find the longest suffix of the current word that matches a snippet prefix.
        // This allows triggering snippets like "你好世界" when typing "请你输出你好" without spacing.
        let mut best_query = String::new();
        let full_word_chars: Vec<char> = query.chars().collect();

        for start_idx in 0..full_word_chars.len() {
            let possible_query: String = full_word_chars[start_idx..].iter().collect();
            if possible_query.len() < 2 {
                break;
            }

            let query_lower = possible_query.to_lowercase();
            let query_chars: Vec<char> = query_lower.chars().collect();

            let has_match = self.snippets_cache.iter().any(|snippet| {
                snippet
                    .to_lowercase()
                    .starts_with(&String::from_iter(&query_chars))
            });

            if has_match {
                best_query = possible_query;
                break;
            }
        }

        if !best_query.is_empty() {
            query = best_query;
        }

        // If query byte length is too short or empty, hide menu
        if query.len() < 2 {
            self.clear();
            return;
        }

        // If query hasn't changed and menu is visible, no need to re-search
        if self.visible && self.query == query {
            return;
        }

        self.query = query.to_string();
        self.search_snippets();
        self.visible = !self.snippets.is_empty();
    }

    fn current_word(input: &str, cursor: usize) -> String {
        if cursor == 0 {
            return String::new();
        }

        // Clamp cursor to string length (cursor is byte offset)
        let cursor = cursor.min(input.len());

        // Find the character index where cursor falls (how many complete chars before cursor)
        let mut char_count_before_cursor = 0;
        for (byte_pos, _c) in input.char_indices() {
            if byte_pos >= cursor {
                break;
            }
            char_count_before_cursor += 1;
        }

        // Now find word start by going backwards from cursor position
        let mut word_char_start = char_count_before_cursor;

        // Get all chars for easy indexing
        let chars: Vec<char> = input.chars().collect();

        for i in (0..char_count_before_cursor).rev() {
            let c = chars[i];
            if c.is_whitespace() || (!c.is_alphanumeric() && c != '_') {
                word_char_start = i + 1;
                break;
            }
            word_char_start = i;
        }

        // Extract the word
        chars[word_char_start..char_count_before_cursor]
            .iter()
            .collect()
    }

    fn search_snippets(&mut self) {
        let query_lower = self.query.to_lowercase();
        let query_chars: Vec<char> = query_lower.chars().collect();

        let mut results: Vec<Snippet> = self
            .snippets_cache
            .iter()
            .filter_map(|snippet| {
                let snippet_lower = snippet.to_lowercase();
                let (score, matched_indices) = Self::calculate_score(&snippet_lower, &query_chars);

                if score > 0 {
                    Some(Snippet {
                        text: snippet.clone(),
                        matched_indices,
                        score,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending
        results.sort_by_key(|b| std::cmp::Reverse(b.score));

        // Limit to max suggestions
        results.truncate(MAX_SUGGESTIONS);

        self.snippets = results;
        self.selected_index = 0;
    }

    fn calculate_score(snippet: &str, query_chars: &[char]) -> (i64, Vec<usize>) {
        // Only use simple prefix matching
        if snippet.starts_with(&String::from_iter(query_chars)) {
            let matched: Vec<usize> = (0..query_chars.len()).collect();
            let score = 1000 + query_chars.len() as i64;
            return (score, matched);
        }

        (0, vec![])
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.snippets.is_empty() {
            return;
        }

        let len = self.snippets.len() as isize;
        let current = self.selected_index as isize;
        self.selected_index = (current + delta).rem_euclid(len) as usize;
    }

    pub fn apply_completion(&self) -> Option<String> {
        let selected = self.snippets.get(self.selected_index)?;
        Some(selected.text.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_snippets() {
        let content = r#"
# This is a comment
hello world

another snippet
"#;
        let mut snippets = Vec::new();
        SnippetState::parse_snippets_from_content(content, &mut snippets);
        assert_eq!(snippets.len(), 2);
        assert!(snippets.contains(&"hello world".to_string()));
        assert!(snippets.contains(&"another snippet".to_string()));
    }

    #[test]
    fn test_current_word() {
        // Debug: check "  hello" at cursor 3
        let input = "  hello";
        let cursor = 3;
        let input_before_cursor = &input[..cursor.min(input.len())];
        eprintln!(
            "input_before_cursor: '{}' (len={})",
            input_before_cursor,
            input_before_cursor.len()
        );
        for (i, c) in input_before_cursor.char_indices().rev() {
            eprintln!("  i={}, c='{}'", i, c);
        }

        // The function returns the word fragment from cursor position backwards to word start
        // cursor positions in "hello world" (indices 0-11):
        // 0=h,1=e,2=l,3=l,4=o,5=space,6=w,7=o,8=r,9=l,10=d,11=end
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
    fn test_exact_prefix_match() {
        let (score, indices) = SnippetState::calculate_score("hello world", &['h', 'e', 'l']);
        assert!(score > 1000);
        assert_eq!(indices, vec![0, 1, 2]);
    }

    #[test]
    fn test_no_match() {
        let (score, _) = SnippetState::calculate_score("abc", &['x', 'y', 'z']);
        assert_eq!(score, 0);
    }

    #[test]
    fn test_non_prefix_no_match() {
        // Non-prefix queries should not match anymore (no fuzzy/subsequence matching)
        let (score, _) = SnippetState::calculate_score("hello world", &['e', 'l', 'o']);
        assert_eq!(score, 0);

        let (score, _) = SnippetState::calculate_score("hello", &['h', 'a', 'l', 'o']);
        assert_eq!(score, 0);
    }
}

#[test]
fn test_cjk_support() {
    // Test pure CJK prefix matching
    let (score, _) = SnippetState::calculate_score("你好世界", &['你', '好']);
    eprintln!("CJK prefix match score: {}", score);
    assert!(score > 0, "CJK prefix match should work");

    // Non-prefix CJK queries should not match
    let (score, _) = SnippetState::calculate_score("你好世界", &['好', '世']);
    eprintln!("CJK non-prefix match score: {}", score);
    assert_eq!(score, 0, "CJK non-prefix should not match");

    // Test current_word with CJK - use valid character boundaries
    // "你好世界" = 你(0-2), 好(3-5), 世(6-8), 界(9-11)
    // cursor=3 is at start of "好", should return "你"
    assert_eq!(SnippetState::current_word("你好世界", 3), "你");
    // cursor=6 is at start of "世", should return "你好"
    assert_eq!(SnippetState::current_word("你好世界", 6), "你好");
    // cursor=9 is at start of "界", should return "你好世"
    assert_eq!(SnippetState::current_word("你好世界", 9), "你好世");
    // cursor=12 is at end, should return "你好世界"
    assert_eq!(SnippetState::current_word("你好世界", 12), "你好世界");
}
