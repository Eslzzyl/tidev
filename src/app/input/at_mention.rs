use ignore::WalkBuilder;
use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

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
    pub matched_indices: Vec<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct AtMentionState {
    pub visible: bool,
    pub query: String,
    pub selected_index: usize,
    pub suggestions: Vec<AtMentionSuggestion>,
    last_index_revision: u64,
    index: Arc<AtMentionIndex>,
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
        AtMentionIndex::ensure_background_indexing(&self.index, workspace_root);
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
        let search_result = self.index.search_entries(&self.query);
        self.suggestions = search_result.suggestions;
        self.last_index_revision = search_result.revision;
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

#[derive(Clone, Debug)]
struct IndexSnapshot {
    entries: Arc<Vec<IndexedEntry>>,
    revision: u64,
}

impl Default for IndexSnapshot {
    fn default() -> Self {
        Self {
            entries: Arc::new(Vec::new()),
            revision: 0,
        }
    }
}

#[derive(Debug)]
struct WatcherHandle {
    id: u64,
    root: PathBuf,
    stop_tx: mpsc::Sender<()>,
}

#[derive(Debug, Default)]
struct AtMentionIndex {
    root: Mutex<Option<PathBuf>>,
    snapshot: Mutex<IndexSnapshot>,
    watcher: Mutex<Option<WatcherHandle>>,
    current_generation: AtomicU64,
    completed_generation: AtomicU64,
    worker_generation: AtomicU64,
    watcher_id_seed: AtomicU64,
}

#[derive(Clone, Debug)]
struct IndexedEntry {
    path: String,
    display: String,
    lowercase_path: String,
    lowercase_name: String,
    basename_char_offset: usize,
    kind: AtMentionKind,
}

#[derive(Clone, Debug)]
struct MatchCandidate {
    score: u32,
    matched_indices: Vec<usize>,
}

#[derive(Clone, Debug)]
struct SearchResult {
    revision: u64,
    suggestions: Vec<AtMentionSuggestion>,
}

impl IndexedEntry {
    fn suggestion(&self, matched_indices: Vec<usize>) -> AtMentionSuggestion {
        AtMentionSuggestion {
            path: self.path.clone(),
            display: self.display.clone(),
            kind: self.kind,
            matched_indices,
        }
    }
}

impl AtMentionIndex {
    fn ensure_background_indexing(index: &Arc<Self>, workspace_root: &Path) {
        let generation = index.ensure_root(workspace_root);
        index.ensure_workspace_watcher(workspace_root);
        if index.completed_generation.load(Ordering::Acquire) == generation {
            return;
        }

        if index.worker_generation.load(Ordering::Acquire) == generation {
            return;
        }

        index.worker_generation.store(generation, Ordering::Release);
        let index = Arc::clone(index);
        let workspace_root = workspace_root.to_path_buf();
        thread::spawn(move || index.build_index(workspace_root, generation));
    }

    fn ensure_root(&self, workspace_root: &Path) -> u64 {
        let mut root = self.root.lock().unwrap();
        if root.as_deref() != Some(workspace_root) {
            *root = Some(workspace_root.to_path_buf());
            self.current_generation.fetch_add(1, Ordering::AcqRel);
            self.completed_generation.store(0, Ordering::Release);

            let mut snapshot = self.snapshot.lock().unwrap();
            snapshot.entries = Arc::new(Vec::new());
            snapshot.revision = snapshot.revision.wrapping_add(1);
        }

        self.current_generation.load(Ordering::Acquire)
    }

    fn ensure_workspace_watcher(self: &Arc<Self>, workspace_root: &Path) {
        {
            let watcher_slot = self.watcher.lock().unwrap();
            if watcher_slot
                .as_ref()
                .is_some_and(|handle| handle.root == workspace_root)
            {
                return;
            }
        }

        let watcher_id = self
            .watcher_id_seed
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        let (stop_tx, stop_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        let mut watcher = match RecommendedWatcher::new(
            move |result| {
                let _ = event_tx.send(result);
            },
            NotifyConfig::default(),
        ) {
            Ok(watcher) => watcher,
            Err(error) => {
                crate::log_warn!("failed to initialize @mention file watcher: {}", error);
                return;
            }
        };

        if let Err(error) = watcher.watch(workspace_root, RecursiveMode::Recursive) {
            crate::log_warn!(
                "failed to watch workspace for @mention refreshes: {}",
                error
            );
            return;
        }

        let old_handle = {
            let mut watcher_slot = self.watcher.lock().unwrap();
            if watcher_slot
                .as_ref()
                .is_some_and(|handle| handle.root == workspace_root)
            {
                return;
            }

            let old_handle = watcher_slot.take();
            *watcher_slot = Some(WatcherHandle {
                id: watcher_id,
                root: workspace_root.to_path_buf(),
                stop_tx,
            });
            old_handle
        };

        let index = Arc::clone(self);
        let watch_root = workspace_root.to_path_buf();
        thread::spawn(move || {
            index.run_workspace_watcher(watcher_id, watch_root, watcher, event_rx, stop_rx)
        });

        if let Some(handle) = old_handle {
            let _ = handle.stop_tx.send(());
        }
    }

    fn run_workspace_watcher(
        self: Arc<Self>,
        watcher_id: u64,
        workspace_root: PathBuf,
        _watcher: RecommendedWatcher,
        event_rx: mpsc::Receiver<notify::Result<Event>>,
        stop_rx: mpsc::Receiver<()>,
    ) {
        let debounce = Duration::from_millis(150);
        let mut pending_refresh = false;

        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }

            match event_rx.recv_timeout(debounce) {
                Ok(Ok(event)) => {
                    if Self::event_should_refresh(&event) {
                        pending_refresh = true;
                    }
                }
                Ok(Err(error)) => {
                    crate::log_warn!("@mention watcher event error: {}", error);
                    pending_refresh = true;
                }
                Err(RecvTimeoutError::Timeout) => {
                    if pending_refresh {
                        pending_refresh = false;
                        Self::request_refresh(&self, watcher_id, &workspace_root);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        self.clear_workspace_watcher(&workspace_root, watcher_id);
    }

    fn event_should_refresh(event: &Event) -> bool {
        !matches!(event.kind, EventKind::Access(_))
    }

    fn clear_workspace_watcher(&self, workspace_root: &Path, watcher_id: u64) {
        let mut watcher_slot = self.watcher.lock().unwrap();
        if watcher_slot
            .as_ref()
            .is_some_and(|handle| handle.id == watcher_id && handle.root == workspace_root)
        {
            watcher_slot.take();
        }
    }

    fn request_refresh(index: &Arc<Self>, watcher_id: u64, workspace_root: &Path) {
        if !index.is_current_watcher(workspace_root, watcher_id) {
            return;
        }

        Self::invalidate_and_refresh(index, workspace_root);
    }

    fn is_current_watcher(&self, workspace_root: &Path, watcher_id: u64) -> bool {
        let watcher_slot = self.watcher.lock().unwrap();
        watcher_slot
            .as_ref()
            .is_some_and(|handle| handle.id == watcher_id && handle.root == workspace_root)
    }

    fn is_current_root(&self, workspace_root: &Path) -> bool {
        let root = self.root.lock().unwrap();
        root.as_deref() == Some(workspace_root)
    }

    fn invalidate_and_refresh(index: &Arc<Self>, workspace_root: &Path) {
        if !index.is_current_root(workspace_root) {
            return;
        }

        index.current_generation.fetch_add(1, Ordering::AcqRel);
        index.completed_generation.store(0, Ordering::Release);
        Self::ensure_background_indexing(index, workspace_root);
    }

    fn build_index(self: Arc<Self>, workspace_root: PathBuf, generation: u64) {
        let mut entries = Vec::new();
        walk_workspace_entries(&workspace_root, |entry| {
            if self.current_generation.load(Ordering::Acquire) != generation {
                return false;
            }

            entries.push(entry);
            true
        });

        if self.current_generation.load(Ordering::Acquire) == generation {
            let mut snapshot = self.snapshot.lock().unwrap();
            if self.current_generation.load(Ordering::Acquire) == generation {
                snapshot.entries = Arc::new(entries);
                snapshot.revision = snapshot.revision.wrapping_add(1);
                self.completed_generation
                    .store(generation, Ordering::Release);
            }
        }

        let _ = self.worker_generation.compare_exchange(
            generation,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    fn revision(&self) -> u64 {
        self.snapshot.lock().unwrap().revision
    }

    fn search_entries(&self, query: &str) -> SearchResult {
        let normalized = query.trim().to_ascii_lowercase();
        let (entries, revision) = {
            let snapshot = self.snapshot.lock().unwrap();
            (Arc::clone(&snapshot.entries), snapshot.revision)
        };

        rank_indexed_entries(entries.as_slice(), &normalized, revision)
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

fn walk_workspace_entries<F>(workspace_root: &Path, mut visit: F)
where
    F: FnMut(IndexedEntry) -> bool,
{
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

        let Some(indexed_entry) = build_indexed_entry(rel, path, file_type.is_dir()) else {
            continue;
        };

        if !visit(indexed_entry) {
            break;
        }
    }
}

#[cfg(test)]
fn index_workspace_entries(workspace_root: &Path) -> Vec<IndexedEntry> {
    let mut entries = Vec::new();
    walk_workspace_entries(workspace_root, |entry| {
        entries.push(entry);
        true
    });
    entries
}

fn build_indexed_entry(rel: &Path, path: &Path, is_dir: bool) -> Option<IndexedEntry> {
    let path_text = rel.to_string_lossy().into_owned();
    if path_text.is_empty() {
        return None;
    }

    let kind = if is_dir {
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

    Some(IndexedEntry {
        lowercase_path: path_text.to_ascii_lowercase(),
        lowercase_name: rel
            .file_name()
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default(),
        basename_char_offset: basename_char_offset(&path_text),
        path: path_text,
        display,
        kind,
    })
}

fn score_entry(entry: &IndexedEntry, query: &str) -> Option<MatchCandidate> {
    let query_chars = query.chars().count();
    let mut best: Option<MatchCandidate> = None;

    consider_candidate(
        &mut best,
        exact_match(&entry.lowercase_path, query, 1_000, 0),
    );
    consider_candidate(
        &mut best,
        exact_match(
            &entry.lowercase_name,
            query,
            1_000,
            entry.basename_char_offset,
        ),
    );

    consider_candidate(
        &mut best,
        prefix_match(&entry.lowercase_path, query, 980, 3, 160, 0),
    );
    consider_candidate(
        &mut best,
        prefix_match(
            &entry.lowercase_name,
            query,
            995,
            2,
            120,
            entry.basename_char_offset,
        ),
    );

    consider_candidate(
        &mut best,
        contains_match(&entry.lowercase_path, query, 920, 4, 0),
    );
    consider_candidate(
        &mut best,
        contains_match(
            &entry.lowercase_name,
            query,
            950,
            5,
            entry.basename_char_offset,
        ),
    );

    consider_candidate(
        &mut best,
        subsequence_match(&entry.lowercase_path, query, 850, 4, 7, query_chars, 0),
    );
    consider_candidate(
        &mut best,
        subsequence_match(
            &entry.lowercase_name,
            query,
            890,
            3,
            6,
            query_chars,
            entry.basename_char_offset,
        ),
    );

    let kind_bonus = match entry.kind {
        AtMentionKind::Directory => 20,
        AtMentionKind::Image => 8,
        AtMentionKind::File => 0,
    };
    let depth = entry.path.bytes().filter(|byte| *byte == b'/').count() as u32;
    let depth_penalty = depth * 6;
    let root_bonus = if depth == 0 { 50 } else { 0 };

    best.map(|mut candidate| {
        candidate.score = candidate
            .score
            .saturating_sub(depth_penalty)
            .saturating_add(kind_bonus)
            .saturating_add(root_bonus);
        candidate
    })
}

fn rank_indexed_entries(
    indexed_entries: &[IndexedEntry],
    query: &str,
    revision: u64,
) -> SearchResult {
    let suggestions = if query.is_empty() {
        let mut suggestions = indexed_entries
            .iter()
            .map(|entry| entry.suggestion(Vec::new()))
            .collect::<Vec<_>>();
        suggestions.sort_by(|left, right| {
            kind_rank(left.kind)
                .cmp(&kind_rank(right.kind))
                .then_with(|| left.display.cmp(&right.display))
        });
        suggestions.truncate(MAX_SUGGESTIONS);
        suggestions
    } else {
        let mut ranked: Vec<_> = indexed_entries
            .par_iter()
            .filter_map(|entry| {
                score_entry(entry, query)
                    .map(|candidate| (candidate.score, entry, candidate.matched_indices))
            })
            .collect();

        ranked.sort_by(|(left_score, left, _), (right_score, right, _)| {
            right_score
                .cmp(left_score)
                .then_with(|| kind_rank(left.kind).cmp(&kind_rank(right.kind)))
                .then_with(|| left.display.cmp(&right.display))
        });
        ranked.truncate(MAX_SUGGESTIONS);

        ranked
            .into_iter()
            .map(|(_, entry, matched_indices)| entry.suggestion(matched_indices))
            .collect()
    };

    SearchResult {
        revision,
        suggestions,
    }
}

#[cfg(test)]
fn search_entries(indexed_entries: &[IndexedEntry], query: &str) -> Vec<AtMentionSuggestion> {
    rank_indexed_entries(indexed_entries, &query.trim().to_ascii_lowercase(), 0).suggestions
}

fn consider_candidate(best: &mut Option<MatchCandidate>, candidate: Option<MatchCandidate>) {
    let Some(candidate) = candidate else {
        return;
    };

    if best
        .as_ref()
        .is_none_or(|current| candidate.score > current.score)
    {
        *best = Some(candidate);
    }
}

fn exact_match(haystack: &str, query: &str, score: u32, offset: usize) -> Option<MatchCandidate> {
    if haystack != query {
        return None;
    }

    Some(MatchCandidate {
        score,
        matched_indices: range_indices(offset, haystack.chars().count()),
    })
}

fn prefix_match(
    haystack: &str,
    query: &str,
    base_score: u32,
    length_penalty: u32,
    max_length_penalty: u32,
    offset: usize,
) -> Option<MatchCandidate> {
    if !haystack.starts_with(query) {
        return None;
    }

    let penalty = haystack
        .chars()
        .count()
        .saturating_sub(query.chars().count())
        .min(max_length_penalty as usize) as u32;

    Some(MatchCandidate {
        score: base_score.saturating_sub(penalty.saturating_mul(length_penalty)),
        matched_indices: range_indices(offset, query.chars().count()),
    })
}

fn contains_match(
    haystack: &str,
    query: &str,
    base_score: u32,
    position_penalty: u32,
    offset: usize,
) -> Option<MatchCandidate> {
    let start_byte = haystack.find(query)?;
    let start = offset + byte_offset_to_char_index(haystack, start_byte);
    let query_chars = query.chars().count();
    Some(MatchCandidate {
        score: base_score.saturating_sub((start as u32).saturating_mul(position_penalty)),
        matched_indices: range_indices(start, query_chars),
    })
}

fn subsequence_match(
    haystack: &str,
    query: &str,
    base_score: u32,
    start_penalty: u32,
    gap_penalty: u32,
    query_chars: usize,
    offset: usize,
) -> Option<MatchCandidate> {
    let indices = find_subsequence_indices(haystack, query)?;
    let start = offset + indices[0];
    let span = indices.last().copied().unwrap_or(indices[0]) - indices[0] + 1;
    let gaps = span.saturating_sub(indices.len());

    let score = base_score
        .saturating_sub((start as u32).saturating_mul(start_penalty))
        .saturating_sub((gaps as u32).saturating_mul(gap_penalty))
        .saturating_sub((span.saturating_sub(query_chars) as u32).saturating_mul(4));

    Some(MatchCandidate {
        score,
        matched_indices: indices.into_iter().map(|index| index + offset).collect(),
    })
}

fn find_subsequence_indices(haystack: &str, needle: &str) -> Option<Vec<usize>> {
    let haystack_chars: Vec<char> = haystack.chars().collect();
    let needle_chars: Vec<char> = needle.chars().collect();
    if needle_chars.is_empty() {
        return None;
    }

    let mut haystack_index = 0usize;
    let mut matched_indices = Vec::with_capacity(needle_chars.len());

    for needle_char in needle_chars {
        while haystack_index < haystack_chars.len() && haystack_chars[haystack_index] != needle_char
        {
            haystack_index += 1;
        }

        if haystack_index == haystack_chars.len() {
            return None;
        }

        matched_indices.push(haystack_index);
        haystack_index += 1;
    }

    Some(matched_indices)
}

fn range_indices(start: usize, len: usize) -> Vec<usize> {
    (start..start.saturating_add(len)).collect()
}

fn byte_offset_to_char_index(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset].chars().count()
}

fn basename_char_offset(path: &str) -> usize {
    let Some(byte_offset) = path.bytes().rposition(|byte| byte == b'/' || byte == b'\\') else {
        return 0;
    };

    path[..=byte_offset].chars().count()
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
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::Ordering,
        thread,
        time::{Duration, Instant},
    };
    use uuid::Uuid;

    fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tidev-at-mention-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("failed to create temp dir");
        dir
    }

    fn wait_for_index_ready(index: &AtMentionIndex, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let current_generation = index.current_generation.load(Ordering::Acquire);
            let completed_generation = index.completed_generation.load(Ordering::Acquire);
            let worker_generation = index.worker_generation.load(Ordering::Acquire);
            if completed_generation == current_generation && worker_generation == 0 {
                return;
            }

            if Instant::now() >= deadline {
                panic!("index did not become ready before timeout");
            }

            thread::sleep(Duration::from_millis(10));
        }
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

    #[test]
    fn search_entries_returns_highlight_indices_for_fuzzy_matches() {
        let workspace = make_temp_dir();
        fs::write(workspace.join("at_mention.rs"), "mod").expect("failed to write file");

        let indexed_entries = index_workspace_entries(&workspace);
        let suggestions = search_entries(&indexed_entries, "atmnr");
        let suggestion = suggestions
            .iter()
            .find(|suggestion| suggestion.path == "at_mention.rs")
            .expect("expected suggestion");

        assert_eq!(suggestion.matched_indices, vec![0, 1, 3, 5, 11]);
    }

    #[test]
    fn at_mention_refreshes_when_new_files_are_added() {
        let workspace = make_temp_dir();
        fs::write(workspace.join("alpha.txt"), "alpha").expect("failed to write file");

        let mut state = AtMentionState::default();
        state.start_background_indexing(&workspace);
        wait_for_index_ready(&state.index, Duration::from_secs(5));

        state.sync(&workspace, "@al", 3);
        assert!(
            state
                .suggestions
                .iter()
                .any(|suggestion| suggestion.path == "alpha.txt")
        );

        state.sync(&workspace, "@be", 3);
        assert!(
            state
                .suggestions
                .iter()
                .all(|suggestion| suggestion.path != "beta.txt")
        );

        fs::write(workspace.join("beta.txt"), "beta").expect("failed to write file");
        AtMentionIndex::invalidate_and_refresh(&state.index, &workspace);
        wait_for_index_ready(&state.index, Duration::from_secs(5));

        state.sync(&workspace, "@be", 3);
        assert!(
            state
                .suggestions
                .iter()
                .any(|suggestion| suggestion.path == "beta.txt")
        );
    }
}
