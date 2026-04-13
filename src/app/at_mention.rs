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

        if matches.len() >= 12 {
            break;
        }
    }

    matches.sort_by(|left, right| {
        kind_rank(left.kind)
            .cmp(&kind_rank(right.kind))
            .then_with(|| left.display.cmp(&right.display))
    });

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
