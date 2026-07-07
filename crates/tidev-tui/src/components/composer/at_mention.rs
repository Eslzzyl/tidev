//! AtMentionState — @file path autocomplete for the Composer.
//!
//! When the user types `@path_fragment`, this module queries the
//! [`FileSearchIndex`](tidev_search::FileSearchIndex) and displays a
//! filtered suggestion list.  Tab/Enter accepts the selected suggestion
//! and inserts the path as an atomic inline span.

use std::path::Path;
use std::sync::Arc;

use tidev_search::{FileSearchIndex, FileSuggestion, current_at_fragment};

/// Kind of @-mention result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AtMentionKind {
    File,
    Directory,
    Image,
}

impl From<tidev_search::FileEntryKind> for AtMentionKind {
    fn from(kind: tidev_search::FileEntryKind) -> Self {
        match kind {
            tidev_search::FileEntryKind::File => AtMentionKind::File,
            tidev_search::FileEntryKind::Directory => AtMentionKind::Directory,
            tidev_search::FileEntryKind::Image => AtMentionKind::Image,
        }
    }
}

/// A single suggestion item from the @-mention index.
#[derive(Clone, Debug)]
pub(crate) struct AtMentionSuggestion {
    pub path: String,
    pub display: String,
    pub kind: AtMentionKind,
    pub matched_indices: Vec<usize>,
}

impl From<FileSuggestion> for AtMentionSuggestion {
    fn from(s: FileSuggestion) -> Self {
        AtMentionSuggestion {
            path: s.path,
            display: s.display,
            kind: s.kind.into(),
            matched_indices: s.matched_indices,
        }
    }
}

/// State of the @-mention popup.
#[derive(Clone, Debug)]
pub(crate) struct AtMentionState {
    pub visible: bool,
    pub query: String,
    pub selected_index: usize,
    pub suggestions: Vec<AtMentionSuggestion>,
    last_index_revision: u64,
    index: Option<Arc<FileSearchIndex>>,
}

impl AtMentionState {
    pub fn new() -> Self {
        Self {
            visible: false,
            query: String::new(),
            selected_index: 0,
            suggestions: Vec::new(),
            last_index_revision: 0,
            index: None,
        }
    }

    /// Set the file search index (called once after Runtime is available).
    pub fn set_index(&mut self, index: Arc<FileSearchIndex>) {
        self.index = Some(index);
    }

    pub fn clear(&mut self) {
        self.visible = false;
        self.query.clear();
        self.selected_index = 0;
        self.suggestions.clear();
        self.last_index_revision = 0;
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.suggestions.is_empty() {
            return;
        }
        let len = self.suggestions.len() as isize;
        let current = self.selected_index as isize;
        let next = (current + delta).rem_euclid(len);
        self.selected_index = next as usize;
    }

    pub fn selected(&self) -> Option<&AtMentionSuggestion> {
        self.suggestions.get(self.selected_index)
    }

    /// Sync state with the current input text and cursor position.
    pub fn sync(&mut self, workspace_root: &Path, input: &str, cursor: usize) {
        let Some((_, query)) = current_at_fragment(input, cursor) else {
            self.clear();
            return;
        };

        let Some(ref index) = self.index else {
            self.clear();
            return;
        };

        index.ensure_background_indexing(workspace_root);

        let current_revision = index.revision();
        if self.visible
            && self.query == query
            && self.last_index_revision == current_revision
            && !self.suggestions.is_empty()
        {
            return;
        }

        self.visible = true;
        self.query = query.clone();
        let file_suggestions = index.search(&query);
        self.suggestions = file_suggestions
            .into_iter()
            .map(AtMentionSuggestion::from)
            .collect();
        self.last_index_revision = current_revision;
        self.selected_index = self.selected_index.min(self.suggestions.len().saturating_sub(1));
    }

    /// Height of the popup in terminal rows (0 if hidden).
    pub fn popup_height(&self) -> u16 {
        if !self.visible || self.suggestions.is_empty() {
            return 0;
        }
        (self.suggestions.len() as u16).min(6).saturating_add(2)
    }
}
