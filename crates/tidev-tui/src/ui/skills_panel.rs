use std::path::PathBuf;

/// Represents a discovered skill in the panel.
#[derive(Clone, Debug)]
pub struct SkillItem {
    pub name: String,
    /// Description of the skill (used in preview)
    #[allow(dead_code)]
    pub description: String,
    /// Location where the skill was found
    #[allow(dead_code)]
    pub location: PathBuf,
}

/// Panel state for the /skills command.
///
/// Features:
/// - Searchable skill list (filters by name)
/// - Two-pane layout: list on left, preview on right
/// - Keyboard navigation with vim-style bindings
#[derive(Clone, Debug)]
pub struct SkillsPanelState {
    /// All discovered skills (unfiltered)
    pub all_skills: Vec<SkillItem>,
    /// Currently visible skills after filtering
    pub filtered_indices: Vec<usize>,
    /// Selected index into filtered_indices
    pub selected_index: usize,
    /// Search query composer
    pub query: String,
    /// Scroll offset for the skill list
    pub list_scroll: usize,
    /// Scroll offset for the preview pane
    pub preview_scroll: usize,
    /// Whether the query input is active
    pub query_active: bool,
}

impl SkillsPanelState {
    /// Create a new panel state with the given skills.
    pub fn new(skills: Vec<SkillItem>) -> Self {
        let filtered_indices: Vec<usize> = (0..skills.len()).collect();
        Self {
            all_skills: skills,
            filtered_indices,
            selected_index: 0,
            query: String::new(),
            list_scroll: 0,
            preview_scroll: 0,
            query_active: false,
        }
    }

    /// Returns true if there are no skills at all.
    pub fn is_empty(&self) -> bool {
        self.all_skills.is_empty()
    }

    /// Returns the number of filtered skills.
    pub fn filtered_count(&self) -> usize {
        self.filtered_indices.len()
    }

    /// Returns the currently selected skill, if any.
    pub fn selected_skill(&self) -> Option<&SkillItem> {
        self.filtered_indices
            .get(self.selected_index)
            .and_then(|&idx| self.all_skills.get(idx))
    }

    /// Returns the name of the currently selected skill, if any.
    pub fn selected_skill_name(&self) -> Option<&str> {
        self.selected_skill().map(|s| s.name.as_str())
    }

    /// Append a character to the query (for incremental search).
    pub fn append_to_query(&mut self, ch: char) {
        self.query.push(ch);
        self.refilter();
    }

    /// Remove the last character from the query.
    pub fn backspace_query(&mut self) {
        self.query.pop();
        self.refilter();
    }

    /// Refilter skills based on the current query.
    pub(crate) fn refilter(&mut self) {
        if self.query.is_empty() {
            self.filtered_indices = (0..self.all_skills.len()).collect();
        } else {
            self.filtered_indices = self
                .all_skills
                .iter()
                .enumerate()
                .filter_map(|(idx, skill)| {
                    if skill.name.to_ascii_lowercase().contains(&self.query) {
                        Some(idx)
                    } else {
                        None
                    }
                })
                .collect();
        }
        // Reset selection to safe position
        self.selected_index = 0;
        self.list_scroll = 0;
    }

    /// Move selection up by one item.
    pub fn move_up(&mut self, page_size: usize) {
        if self.filtered_count() == 0 {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = self.filtered_count() - 1;
        } else {
            self.selected_index -= 1;
        }
        self.adjust_scroll(page_size);
    }

    /// Move selection down by one item.
    pub fn move_down(&mut self, page_size: usize) {
        if self.filtered_count() == 0 {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.filtered_count();
        self.adjust_scroll(page_size);
    }

    /// Move selection up by a page.
    pub fn page_up(&mut self, page_size: usize) {
        if self.filtered_count() == 0 {
            return;
        }
        self.selected_index = self.selected_index.saturating_sub(page_size);
        self.adjust_scroll(page_size);
    }

    /// Move selection down by a page.
    pub fn page_down(&mut self, page_size: usize) {
        if self.filtered_count() == 0 {
            return;
        }
        self.selected_index = (self.selected_index + page_size).min(self.filtered_count() - 1);
        self.adjust_scroll(page_size);
    }

    /// Adjust the list scroll to keep the selected item visible.
    fn adjust_scroll(&mut self, page_size: usize) {
        if self.selected_index < self.list_scroll {
            self.list_scroll = self.selected_index;
        } else if self.selected_index >= self.list_scroll + page_size {
            self.list_scroll = self.selected_index.saturating_sub(page_size - 1);
        }
    }

    /// Scroll the preview pane up.
    pub fn scroll_preview_up(&mut self, lines: usize) {
        self.preview_scroll = self.preview_scroll.saturating_sub(lines);
    }

    /// Scroll the preview pane down.
    pub fn scroll_preview_down(&mut self, lines: usize) {
        self.preview_scroll = self.preview_scroll.saturating_add(lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_skills() -> Vec<SkillItem> {
        vec![
            SkillItem {
                name: "rust-debug".to_string(),
                description: "Debug Rust code".to_string(),
                location: PathBuf::from(".opencode/skills/rust-debug"),
            },
            SkillItem {
                name: "python-test".to_string(),
                description: "Test Python code".to_string(),
                location: PathBuf::from(".opencode/skills/python-test"),
            },
            SkillItem {
                name: "js-lint".to_string(),
                description: "Lint JavaScript".to_string(),
                location: PathBuf::from(".opencode/skills/js-lint"),
            },
        ]
    }

    #[test]
    fn test_new_panel() {
        let skills = create_test_skills();
        let panel = SkillsPanelState::new(skills.clone());

        assert_eq!(panel.all_skills.len(), 3);
        assert_eq!(panel.filtered_count(), 3);
        assert_eq!(panel.selected_index, 0);
        assert!(panel.selected_skill().is_some());
    }

    #[test]
    fn test_filter_by_name() {
        let skills = create_test_skills();
        let mut panel = SkillsPanelState::new(skills);

        // Type "rust" character by character
        for ch in "rust".chars() {
            panel.append_to_query(ch);
        }
        assert_eq!(panel.filtered_count(), 1);
        assert_eq!(panel.selected_skill().unwrap().name, "rust-debug");

        // Clear and type "python"
        panel.query.clear();
        panel.refilter();
        for ch in "python".chars() {
            panel.append_to_query(ch);
        }
        assert_eq!(panel.filtered_count(), 1);
        assert_eq!(panel.selected_skill().unwrap().name, "python-test");

        // Clear and type "test"
        panel.query.clear();
        panel.refilter();
        for ch in "test".chars() {
            panel.append_to_query(ch);
        }
        assert_eq!(panel.filtered_count(), 1); // python-test only
    }

    #[test]
    fn test_empty_filter() {
        let skills = create_test_skills();
        let mut panel = SkillsPanelState::new(skills);

        for ch in "xyz".chars() {
            panel.append_to_query(ch);
        }
        assert_eq!(panel.filtered_count(), 0);
        assert!(panel.selected_skill().is_none());
    }

    #[test]
    fn test_navigation() {
        let skills = create_test_skills();
        let mut panel = SkillsPanelState::new(skills);

        panel.move_down(10);
        assert_eq!(panel.selected_index, 1);

        panel.move_down(10);
        assert_eq!(panel.selected_index, 2);

        panel.move_down(10);
        assert_eq!(panel.selected_index, 0); // wrap around

        panel.move_up(10);
        assert_eq!(panel.selected_index, 2); // wrap around
    }

    #[test]
    fn test_navigation_with_filter() {
        let skills = create_test_skills();
        let mut panel = SkillsPanelState::new(skills);

        // Type "test" - matches python-test only
        for ch in "test".chars() {
            panel.append_to_query(ch);
        }
        assert_eq!(panel.filtered_count(), 1);

        // With only one item, selection stays at 0
        panel.move_down(10);
        assert_eq!(panel.selected_index, 0);

        panel.move_down(10);
        assert_eq!(panel.selected_index, 0); // still 0 with wrap
    }

    #[test]
    fn test_backspace_query() {
        let skills = create_test_skills();
        let mut panel = SkillsPanelState::new(skills);

        // Type "rust"
        for ch in "rust".chars() {
            panel.append_to_query(ch);
        }
        assert_eq!(panel.filtered_count(), 1);

        // Backspace to clear
        for _ in 0..4 {
            panel.backspace_query();
        }
        assert_eq!(panel.filtered_count(), 3);
        assert_eq!(panel.selected_index, 0);
    }

    #[test]
    fn test_preview_scroll() {
        let skills = create_test_skills();
        let mut panel = SkillsPanelState::new(skills);

        assert_eq!(panel.preview_scroll, 0);

        panel.scroll_preview_down(10);
        assert_eq!(panel.preview_scroll, 10);

        panel.scroll_preview_up(5);
        assert_eq!(panel.preview_scroll, 5);

        // Should not go below 0
        panel.scroll_preview_up(100);
        assert_eq!(panel.preview_scroll, 0);
    }
}
