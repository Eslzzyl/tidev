use ignore::WalkBuilder;
use std::path::Path;

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
        self.suggestions = search_entries(workspace_root, &self.query);
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

fn search_entries(workspace_root: &Path, query: &str) -> Vec<AtMentionSuggestion> {
    let normalized = query.trim().to_ascii_lowercase();
    let mut matches = Vec::new();
    let walker = WalkBuilder::new(workspace_root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .build();

    for result in walker {
        let Ok(entry) = result else {
            continue;
        };

        let Some(file_type) = entry.file_type() else {
            continue;
        };

        let path = entry.path();
        let Ok(rel) = path.strip_prefix(workspace_root) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let kind = if file_type.is_dir() {
            AtMentionKind::Directory
        } else if is_image_path(path) {
            AtMentionKind::Image
        } else {
            AtMentionKind::File
        };

        let path_text = rel.display().to_string();
        let display = match kind {
            AtMentionKind::Directory => format!("{}/", path_text),
            _ => path_text.clone(),
        };
        let lowercase = display.to_ascii_lowercase();

        if !normalized.is_empty()
            && !lowercase.contains(&normalized)
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().contains(&normalized))
        {
            continue;
        }

        matches.push(AtMentionSuggestion {
            path: path_text,
            display,
            kind,
        });
    }

    matches.sort_by(|left, right| {
        kind_rank(left.kind)
            .cmp(&kind_rank(right.kind))
            .then_with(|| left.display.cmp(&right.display))
    });

    matches.truncate(12);

    matches
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

        let suggestions = search_entries(&workspace, "match");

        assert!(suggestions.iter().any(|suggestion| suggestion.path == "match-root.txt"));
        assert!(suggestions.len() <= 12);
    }

    #[test]
    fn search_entries_skips_workspace_root_directory() {
        let workspace = make_temp_dir();
        fs::write(workspace.join("match-root.txt"), "root").expect("failed to write root file");

        let suggestions = search_entries(&workspace, "");

        assert!(suggestions.iter().all(|suggestion| !suggestion.path.is_empty()));
    }
}
