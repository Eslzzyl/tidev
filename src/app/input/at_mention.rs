use std::path::Path;
use std::sync::Arc;

use crate::shared::file_search::{
    current_at_fragment, FileEntryKind, FileSearchIndex, FileSuggestion,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtMentionKind {
    File,
    Directory,
    Image,
}

impl From<FileEntryKind> for AtMentionKind {
    fn from(kind: FileEntryKind) -> Self {
        match kind {
            FileEntryKind::File => AtMentionKind::File,
            FileEntryKind::Directory => AtMentionKind::Directory,
            FileEntryKind::Image => AtMentionKind::Image,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AtMentionSuggestion {
    pub path: String,
    pub display: String,
    pub kind: AtMentionKind,
    pub matched_indices: Vec<usize>,
}

impl From<FileSuggestion> for AtMentionSuggestion {
    fn from(suggestion: FileSuggestion) -> Self {
        AtMentionSuggestion {
            path: suggestion.path,
            display: suggestion.display,
            kind: suggestion.kind.into(),
            matched_indices: suggestion.matched_indices,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AtMentionState {
    pub visible: bool,
    pub query: String,
    pub selected_index: usize,
    pub suggestions: Vec<AtMentionSuggestion>,
    last_index_revision: u64,
    index: Arc<FileSearchIndex>,
}

impl AtMentionState {
    pub fn clear(&mut self) {
        self.visible = false;
        self.query.clear();
        self.selected_index = 0;
        self.suggestions.clear();
        self.last_index_revision = 0;
    }

    pub fn start_background_indexing(&self, workspace_root: &Path) {
        FileSearchIndex::ensure_background_indexing(&self.index, workspace_root);
    }

    pub fn sync(&mut self, workspace_root: &Path, input: &str, cursor: usize) {
        let Some((_, query)) = current_at_fragment(input, cursor) else {
            self.clear();
            return;
        };

        self.start_background_indexing(workspace_root);

        let current_revision = self.index.revision();
        if self.visible && self.query == query && self.last_index_revision == current_revision {
            return;
        }

        self.visible = true;
        self.query = query.to_string();
        let file_suggestions = self.index.search(&self.query);
        self.suggestions = file_suggestions
            .into_iter()
            .map(AtMentionSuggestion::from)
            .collect();
        self.last_index_revision = current_revision;
        if self.suggestions.is_empty() {
            self.selected_index = 0;
            return;
        }

        self.selected_index = self
            .selected_index
            .min(self.suggestions.len().saturating_sub(1));
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.suggestions.is_empty() {
            return;
        }

        let len = self.suggestions.len() as isize;
        let current = self.selected_index as isize;
        self.selected_index = (current + delta).rem_euclid(len) as usize;
    }

    pub fn selected(&self) -> Option<&AtMentionSuggestion> {
        self.suggestions.get(self.selected_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    fn wait_for_index_ready(index: &FileSearchIndex, timeout: Duration) {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            // Check if indexing is complete by comparing generations
            // Index is ready when completed_generation matches current_generation
            let current = std::sync::atomic::Ordering::Acquire;
            let current_gen = index.current_generation.load(current);
            let completed_gen = index.completed_generation.load(current);
            if current_gen == completed_gen && current_gen > 0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn test_current_at_fragment_basic() {
        assert_eq!(
            current_at_fragment("Hello @world", 12),
            Some((6, "world".to_string()))
        );
        assert_eq!(
            current_at_fragment("Hello @world test", 12),
            Some((6, "world".to_string()))
        );
        assert_eq!(current_at_fragment("Hello @world test", 17), None);
    }

    #[test]
    fn test_current_at_fragment_no_whitespace_after_at() {
        assert_eq!(current_at_fragment("Hello @ world", 13), None);
    }

    #[test]
    fn test_current_at_fragment_at_start() {
        assert_eq!(
            current_at_fragment("@file", 5),
            Some((0, "file".to_string()))
        );
    }

    #[test]
    fn test_current_at_fragment_empty_query() {
        assert_eq!(current_at_fragment("Hello @", 7), Some((6, "".to_string())));
    }

    #[test]
    fn test_current_at_fragment_no_at() {
        assert_eq!(current_at_fragment("Hello world", 11), None);
    }

    #[test]
    fn test_current_at_fragment_after_bracket() {
        assert_eq!(
            current_at_fragment("(@file)", 6),
            Some((1, "file".to_string()))
        );
    }

    #[test]
    fn test_mention_state_sync_and_select() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();

        fs::write(workspace.join("alpha.txt"), "alpha").expect("failed to write file");
        fs::create_dir(workspace.join("beta")).expect("failed to create dir");
        fs::write(workspace.join("beta").join("gamma.txt"), "gamma")
            .expect("failed to write file");

        let mut state = AtMentionState::default();
        state.sync(workspace, "@alp", 4);
        wait_for_index_ready(&state.index, Duration::from_secs(5));
        state.sync(workspace, "@alp", 4);

        assert!(
            state
                .suggestions
                .iter()
                .any(|suggestion| suggestion.path == "alpha.txt")
        );

        // Test selection navigation (circular)
        let initial_index = state.selected_index;
        state.move_selection(1);
        let after_forward = state.selected_index;
        state.move_selection(-1);
        let after_backward = state.selected_index;

        // Should return to initial position after forward and backward
        assert_eq!(after_backward, initial_index);
        // If there are multiple suggestions, forward should be different
        if state.suggestions.len() > 1 {
            assert_ne!(after_forward, initial_index);
        }
    }

    #[test]
    fn test_mention_state_clears_when_no_at() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();

        fs::write(workspace.join("file.txt"), "content").expect("failed to write file");

        let mut state = AtMentionState::default();
        state.sync(workspace, "@file", 5);
        wait_for_index_ready(&state.index, Duration::from_secs(5));
        state.sync(workspace, "@file", 5);

        assert!(!state.suggestions.is_empty());

        state.sync(workspace, "no at here", 10);
        assert!(!state.visible);
        assert!(state.suggestions.is_empty());
    }

    #[test]
    fn test_mention_state_file_watcher_refresh() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();

        fs::write(workspace.join("alpha.txt"), "alpha").expect("failed to write file");

        let mut state = AtMentionState::default();
        state.sync(workspace, "@", 1);
        wait_for_index_ready(&state.index, Duration::from_secs(5));
        state.sync(workspace, "@", 1);

        let initial_count = state.suggestions.len();

        fs::write(workspace.join("beta.txt"), "beta").expect("failed to write file");
        FileSearchIndex::invalidate_and_refresh(&state.index, workspace);
        wait_for_index_ready(&state.index, Duration::from_secs(5));

        state.sync(workspace, "@", 1);
        assert_eq!(state.suggestions.len(), initial_count + 1);
    }

    #[test]
    fn test_mention_kind_conversion() {
        assert!(matches!(
            AtMentionKind::from(FileEntryKind::File),
            AtMentionKind::File
        ));
        assert!(matches!(
            AtMentionKind::from(FileEntryKind::Directory),
            AtMentionKind::Directory
        ));
        assert!(matches!(
            AtMentionKind::from(FileEntryKind::Image),
            AtMentionKind::Image
        ));
    }
}
