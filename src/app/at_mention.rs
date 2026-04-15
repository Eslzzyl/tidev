use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

const MAX_SUGGESTIONS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtMentionKind {
    File,
    Directory,
    Image,
}

#[derive(Clone, Debug)]
pub struct AtMentionSuggestion {
    pub path: String,
    pub display: String,
    pub kind: AtMentionKind,
}

#[derive(Clone, Debug, Default)]
pub struct AtMentionState {
    pub visible: bool,
    pub query: String,
    pub selected_index: usize,
    pub suggestions: Vec<AtMentionSuggestion>,
    indexed_root: Option<PathBuf>,
    indexed_entries: Vec<IndexedEntry>,
}

impl AtMentionState {
    pub fn clear(&mut self) {
        self.visible = false;
        self.query.clear();
        self.selected_index = 0;
        self.suggestions.clear();
    }

    pub fn sync(&mut self, workspace_root: &Path, input: &str, cursor: usize) {
        let Some((_, query)) = current_at_fragment(input, cursor) else {
            self.clear();
            return;
        };

        self.visible = true;
        self.query = query.to_string();
        self.ensure_index(workspace_root);
        self.suggestions = search_entries(&self.indexed_entries, &self.query);
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

    fn ensure_index(&mut self, workspace_root: &Path) {
        if self.indexed_root.as_deref() == Some(workspace_root) {
            return;
        }

        self.indexed_entries = index_workspace_entries(workspace_root);
        self.indexed_root = Some(workspace_root.to_path_buf());
    }
}

pub fn current_at_fragment(input: &str, cursor: usize) -> Option<(usize, String)> {
    let cursor = cursor.min(input.len());
    let prefix = input.get(..cursor)?;
    let at_index = prefix.rfind('@')?;
    if at_index > 0 {
        let previous = prefix[..at_index].chars().last()?;
        if !previous.is_whitespace() && !matches!(previous, '(' | '[' | '{' | '"' | '/' | '\\') {
            return None;
        }
    }

    let query = &prefix[at_index + 1..];
    if query.chars().any(char::is_whitespace) {
        return None;
    }

    Some((at_index, query.to_string()))
}

#[derive(Clone, Debug)]
struct IndexedEntry {
    path: String,
    display: String,
    lowercase_display: String,
    lowercase_name: String,
    kind: AtMentionKind,
}

impl IndexedEntry {
    fn suggestion(&self) -> AtMentionSuggestion {
        AtMentionSuggestion {
            path: self.path.clone(),
            display: self.display.clone(),
            kind: self.kind,
        }
    }
}

fn index_workspace_entries(workspace_root: &Path) -> Vec<IndexedEntry> {
    let mut entries = Vec::new();
    let walker = WalkBuilder::new(workspace_root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(true)
        .require_git(true)
        .build();

    for result in walker {
        let Ok(entry) = result else {
            continue;
        };

        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() && !file_type.is_file() {
            continue;
        }

        let path = entry.path();
        let Ok(rel) = path.strip_prefix(workspace_root) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }

        let path_text = rel.to_string_lossy().into_owned();
        let kind = if file_type.is_dir() {
            AtMentionKind::Directory
        } else if is_image_path(path) {
            AtMentionKind::Image
        } else {
            AtMentionKind::File
        };
        let display = match kind {
            AtMentionKind::Directory => format!("{}/", path_text),
            _ => path_text.clone(),
        };

        entries.push(IndexedEntry {
            path: path_text,
            lowercase_display: display.to_ascii_lowercase(),
            lowercase_name: rel
                .file_name()
                .map(|value| value.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default(),
            display,
            kind,
        });
    }

    entries
}

fn search_entries(indexed_entries: &[IndexedEntry], query: &str) -> Vec<AtMentionSuggestion> {
    let normalized = query.trim().to_ascii_lowercase();

    if normalized.is_empty() {
        let mut suggestions = indexed_entries
            .iter()
            .map(IndexedEntry::suggestion)
            .collect::<Vec<_>>();
        suggestions.sort_by(|left, right| {
            kind_rank(left.kind)
                .cmp(&kind_rank(right.kind))
                .then_with(|| left.display.cmp(&right.display))
        });
        suggestions.truncate(MAX_SUGGESTIONS);
        return suggestions;
    }

    let mut ranked = indexed_entries
        .iter()
        .filter_map(|entry| score_entry(entry, &normalized).map(|score| (score, entry)))
        .collect::<Vec<_>>();

    ranked.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| kind_rank(left.kind).cmp(&kind_rank(right.kind)))
            .then_with(|| left.display.cmp(&right.display))
    });
    ranked.truncate(MAX_SUGGESTIONS);

    ranked
        .into_iter()
        .map(|(_, entry)| entry.suggestion())
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct SubsequenceMatch {
    start: usize,
    span: usize,
    gaps: usize,
}

fn score_entry(entry: &IndexedEntry, query: &str) -> Option<u32> {
    let mut best_score = None;

    if entry.lowercase_name == query {
        best_score = Some(1_000);
    }

    if entry.lowercase_name.starts_with(query) {
        let penalty = entry
            .lowercase_name
            .len()
            .saturating_sub(query.len())
            .min(160) as u32;
        let score = 980u32.saturating_sub(penalty);
        best_score = Some(best_score.unwrap_or(0).max(score));
    }

    if let Some(position) = entry.lowercase_name.find(query) {
        let score = 930u32.saturating_sub((position as u32).saturating_mul(8));
        best_score = Some(best_score.unwrap_or(0).max(score));
    }

    if entry.lowercase_display.starts_with(query) {
        let penalty = entry
            .lowercase_display
            .len()
            .saturating_sub(query.len())
            .min(180) as u32;
        let score = 900u32.saturating_sub(penalty);
        best_score = Some(best_score.unwrap_or(0).max(score));
    }

    if let Some(position) = entry.lowercase_display.find(query) {
        let score = 860u32.saturating_sub((position as u32).saturating_mul(4));
        best_score = Some(best_score.unwrap_or(0).max(score));
    }

    if let Some(subsequence) = find_subsequence_match(&entry.lowercase_name, query) {
        let score = 800u32
            .saturating_sub((subsequence.start as u32).saturating_mul(3))
            .saturating_sub((subsequence.gaps as u32).saturating_mul(6))
            .saturating_sub(
                (subsequence.span as u32)
                    .saturating_sub(query.len() as u32)
                    .saturating_mul(4),
            );
        best_score = Some(best_score.unwrap_or(0).max(score));
    }

    if let Some(subsequence) = find_subsequence_match(&entry.lowercase_display, query) {
        let score = 720u32
            .saturating_sub((subsequence.start as u32).saturating_mul(2))
            .saturating_sub((subsequence.gaps as u32).saturating_mul(3))
            .saturating_sub(
                (subsequence.span as u32)
                    .saturating_sub(query.len() as u32)
                    .saturating_mul(2),
            );
        best_score = Some(best_score.unwrap_or(0).max(score));
    }

    let kind_bonus = match entry.kind {
        AtMentionKind::Directory => 20,
        AtMentionKind::Image => 8,
        AtMentionKind::File => 0,
    };
    let depth_penalty = entry.path.bytes().filter(|byte| *byte == b'/').count() as u32 * 6;

    best_score.map(|score| {
        score
            .saturating_sub(depth_penalty)
            .saturating_add(kind_bonus)
    })
}

fn find_subsequence_match(haystack: &str, needle: &str) -> Option<SubsequenceMatch> {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() {
        return None;
    }

    let mut haystack_index = 0usize;
    let mut start = None;
    let mut previous_index = None;
    let mut gaps = 0usize;

    for needle_byte in needle {
        while haystack_index < haystack.len() && haystack[haystack_index] != *needle_byte {
            haystack_index += 1;
        }
        if haystack_index == haystack.len() {
            return None;
        }

        if let Some(previous) = previous_index {
            gaps += haystack_index.saturating_sub(previous + 1);
        } else {
            start = Some(haystack_index);
        }

        previous_index = Some(haystack_index);
        haystack_index += 1;
    }

    let start = start.unwrap_or(0);
    let end = previous_index.unwrap_or(start);
    Some(SubsequenceMatch {
        start,
        span: end.saturating_sub(start).saturating_add(1),
        gaps,
    })
}

fn kind_rank(kind: AtMentionKind) -> usize {
    match kind {
        AtMentionKind::Directory => 0,
        AtMentionKind::Image => 1,
        AtMentionKind::File => 2,
    }
}

fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif"
            )
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};
    use uuid::Uuid;

    fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tidev-at-mention-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("failed to create temp dir");
        dir
    }

    #[test]
    fn search_entries_includes_root_level_files() {
        let workspace = make_temp_dir();
        let nested = workspace.join("nested");
        fs::create_dir_all(&nested).expect("failed to create nested dir");

        for index in 0..12 {
            fs::write(nested.join(format!("match-{index:02}.txt")), "nested")
                .expect("failed to write nested file");
        }
        fs::write(workspace.join("match-root.txt"), "root").expect("failed to write root file");

        let indexed_entries = index_workspace_entries(&workspace);
        let suggestions = search_entries(&indexed_entries, "match");

        assert!(
            suggestions
                .iter()
                .any(|suggestion| suggestion.path == "match-root.txt")
        );
        assert!(suggestions.len() <= 12);
    }

    #[test]
    fn search_entries_skips_workspace_root_directory() {
        let workspace = make_temp_dir();
        fs::write(workspace.join("match-root.txt"), "root").expect("failed to write root file");

        let indexed_entries = index_workspace_entries(&workspace);
        let suggestions = search_entries(&indexed_entries, "");

        assert!(
            suggestions
                .iter()
                .all(|suggestion| !suggestion.path.is_empty())
        );
    }

    #[test]
    fn search_entries_prefers_basename_prefix_matches() {
        let workspace = make_temp_dir();
        let nested = workspace.join("nested").join("docs");
        fs::create_dir_all(&nested).expect("failed to create nested dirs");
        fs::write(workspace.join("target-note.txt"), "root").expect("failed to write root file");
        fs::write(nested.join("alpha-target.md"), "nested").expect("failed to write nested file");

        let indexed_entries = index_workspace_entries(&workspace);
        let suggestions = search_entries(&indexed_entries, "target");

        assert_eq!(
            suggestions
                .first()
                .map(|suggestion| suggestion.path.as_str()),
            Some("target-note.txt")
        );
    }

    #[test]
    fn search_entries_supports_fuzzy_subsequence_matches() {
        let workspace = make_temp_dir();
        fs::write(workspace.join("at_mention.rs"), "mod").expect("failed to write file");

        let indexed_entries = index_workspace_entries(&workspace);
        let suggestions = search_entries(&indexed_entries, "atmnr");

        assert!(
            suggestions
                .iter()
                .any(|suggestion| suggestion.path == "at_mention.rs")
        );
    }
}
