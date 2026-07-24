//! File search indexing for tidev `@`-mention completion.
//!
//! Provides a background-indexed file search system used by the TUI's
//! `@`-mention autocomplete.  The index is built in a worker thread and
//! kept up-to-date via `notify` file-system watchers.

use ignore::WalkBuilder;

use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::ModifyKind,
};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const MAX_SUGGESTIONS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileEntryKind {
    File,
    Directory,
    Image,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct FileSuggestion {
    pub path: String,
    pub display: String,
    pub kind: FileEntryKind,
    pub matched_indices: Vec<usize>,
}

#[derive(Debug)]
pub struct FileSearchIndex {
    root: Mutex<Option<PathBuf>>,
    snapshot: Mutex<IndexSnapshot>,
    watcher: Mutex<Option<WatcherHandle>>,
    pub current_generation: AtomicU64,
    pub completed_generation: AtomicU64,
    worker_generation: AtomicU64,
    watcher_id_seed: AtomicU64,
    revision: AtomicU64,
    empty_cache: Mutex<(u64, Vec<FileSuggestion>)>,
}

impl Clone for FileSearchIndex {
    fn clone(&self) -> Self {
        let snapshot = self.snapshot.lock().unwrap();
        Self {
            root: Mutex::new(self.root.lock().unwrap().clone()),
            snapshot: Mutex::new(snapshot.clone()),
            watcher: Mutex::new(None),
            current_generation: AtomicU64::new(self.current_generation.load(Ordering::Acquire)),
            completed_generation: AtomicU64::new(self.completed_generation.load(Ordering::Acquire)),
            worker_generation: AtomicU64::new(self.worker_generation.load(Ordering::Acquire)),
            watcher_id_seed: AtomicU64::new(self.watcher_id_seed.load(Ordering::Acquire)),
            revision: AtomicU64::new(self.revision.load(Ordering::Acquire)),
            empty_cache: Mutex::new(self.empty_cache.lock().unwrap().clone()),
        }
    }
}

#[derive(Clone, Debug)]
struct IndexSnapshot {
    segments: Arc<HashMap<String, Vec<IndexedEntry>>>,
    flat_cache: Arc<Vec<IndexedEntry>>,
    revision: u64,
}

impl Default for IndexSnapshot {
    fn default() -> Self {
        Self {
            segments: Arc::new(HashMap::new()),
            flat_cache: Arc::new(Vec::new()),
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

#[derive(Clone, Debug)]
struct IndexedEntry {
    path: String,
    display: String,
    lowercase_path: String,
    lowercase_name: String,
    basename_char_offset: usize,
    kind: FileEntryKind,
    depth: u32,
    is_dotfile: bool,
}

impl IndexedEntry {
    fn suggestion(&self, matched_indices: Vec<usize>) -> FileSuggestion {
        FileSuggestion {
            path: self.path.clone(),
            display: self.display.clone(),
            kind: self.kind,
            matched_indices,
        }
    }
}

#[derive(Clone, Debug)]
struct MatchCandidate {
    score: u32,
    matched_indices: Vec<usize>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

impl FileSearchIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the current revision number, incremented on each index rebuild.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub fn ensure_background_indexing(self: &Arc<Self>, workspace_root: &Path) {
        let generation = self.ensure_root(workspace_root);
        self.ensure_workspace_watcher(workspace_root);
        if self.completed_generation.load(Ordering::Acquire) == generation {
            return;
        }

        if self.worker_generation.load(Ordering::Acquire) == generation {
            return;
        }

        self.worker_generation.store(generation, Ordering::Release);
        let index = Arc::clone(self);
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
            snapshot.segments = Arc::new(HashMap::new());
            snapshot.flat_cache = Arc::new(Vec::new());
            snapshot.revision = snapshot.revision.wrapping_add(1);
            self.revision.store(snapshot.revision, Ordering::Release);
            self.empty_cache.lock().unwrap().0 = 0;
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

        let watcher = match RecommendedWatcher::new(
            move |result| {
                let _ = event_tx.send(result);
            },
            NotifyConfig::default(),
        ) {
            Ok(watcher) => watcher,
            Err(error) => {
                log::warn!("failed to initialize file search watcher: {}", error);
                return;
            }
        };

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
        mut watcher: RecommendedWatcher,
        event_rx: mpsc::Receiver<notify::Result<Event>>,
        stop_rx: mpsc::Receiver<()>,
    ) {
        if let Err(error) = watcher.watch(&workspace_root, RecursiveMode::Recursive) {
            log::warn!(
                "failed to watch workspace for file search refreshes: {}",
                error,
            );
            return;
        }

        let debounce = Duration::from_millis(150);
        let mut pending_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut last_event = std::time::Instant::now() - debounce - Duration::from_secs(1);

        loop {
            let now = std::time::Instant::now();
            let until_next = if pending_dirs.is_empty() {
                Duration::from_secs(60)
            } else {
                debounce.saturating_sub(now.duration_since(last_event))
            };

            match event_rx.recv_timeout(until_next) {
                Ok(Ok(event)) => {
                    let should_invalidate = matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(_)
                    );

                    if should_invalidate {
                        for path in event.paths {
                            if let Ok(rel) = path.strip_prefix(&workspace_root) {
                                let rel_str = rel.to_string_lossy().into_owned();
                                let dir = if path.is_dir() {
                                    rel_str
                                } else {
                                    rel.parent()
                                        .map(|p| p.to_string_lossy().into_owned())
                                        .unwrap_or_default()
                                };
                                pending_dirs.insert(dir);
                            }
                        }
                        last_event = now;
                    }

                    if let EventKind::Modify(ModifyKind::Name(_)) = event.kind {
                        Self::invalidate_and_refresh(&self, &workspace_root);
                        pending_dirs.clear();
                    }
                }
                Ok(Err(error)) => {
                    log::warn!("file search watcher event error: {}", error);
                }
                Err(RecvTimeoutError::Timeout) => {
                    if !pending_dirs.is_empty() {
                        self.refresh_segments(&workspace_root, std::mem::take(&mut pending_dirs));
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }

            if stop_rx.try_recv().is_ok() {
                break;
            }

            if self
                .watcher
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|handle| handle.id != watcher_id)
            {
                break;
            }
        }
    }

    pub fn invalidate_and_refresh(self: &Arc<Self>, workspace_root: &Path) {
        let generation = self.current_generation.load(Ordering::Acquire);
        self.completed_generation.store(0, Ordering::Release);
        self.worker_generation.store(0, Ordering::Release);

        let index = Arc::clone(self);
        let workspace_root = workspace_root.to_path_buf();
        thread::spawn(move || index.build_index(workspace_root, generation));
    }

    fn build_index(&self, workspace_root: PathBuf, generation: u64) {
        let mut segments: HashMap<String, Vec<IndexedEntry>> = HashMap::new();
        let mut root_entries = Vec::new();

        walk_workspace_entries(&workspace_root, |entry| {
            if entry.path.contains('/') {
                let dir = entry
                    .path
                    .rsplit_once('/')
                    .map(|x| x.0)
                    .unwrap_or("")
                    .to_string();
                segments.entry(dir).or_default().push(entry);
            } else {
                root_entries.push(entry);
            }
            true
        });

        if !root_entries.is_empty() {
            segments.insert(String::new(), root_entries);
        }

        let flat_cache: Vec<IndexedEntry> = segments.values().flatten().cloned().collect();

        {
            let mut snapshot = self.snapshot.lock().unwrap();
            snapshot.segments = Arc::new(segments);
            snapshot.flat_cache = Arc::new(flat_cache);
            snapshot.revision = snapshot.revision.wrapping_add(1);
            self.revision.store(snapshot.revision, Ordering::Release);
        }

        self.completed_generation
            .store(generation, Ordering::Release);
        log::info!(
            "file search index built for {:?}, {} entries",
            workspace_root,
            self.snapshot.lock().unwrap().flat_cache.len()
        );
    }

    fn refresh_segments(&self, workspace_root: &Path, dirs: std::collections::HashSet<String>) {
        let mut snapshot = self.snapshot.lock().unwrap();
        let mut segments = (*snapshot.segments).clone();

        for dir in dirs {
            let new_entries = scan_directory_entries(workspace_root, &dir);
            if new_entries.is_empty() {
                segments.remove(&dir);
            } else {
                segments.insert(dir, new_entries);
            }
        }

        let flat_cache: Vec<IndexedEntry> = segments.values().flatten().cloned().collect();
        snapshot.segments = Arc::new(segments);
        snapshot.flat_cache = Arc::new(flat_cache);
        snapshot.revision = snapshot.revision.wrapping_add(1);
        self.revision.store(snapshot.revision, Ordering::Release);
    }

    pub fn search(&self, query: &str) -> Vec<FileSuggestion> {
        let normalized = query.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            let revision = self.revision.load(Ordering::Acquire);
            let mut cache = self.empty_cache.lock().unwrap();
            if cache.0 == revision {
                return cache.1.clone();
            }
            let entries = {
                let snapshot = self.snapshot.lock().unwrap();
                Arc::clone(&snapshot.flat_cache)
            };
            let result = rank_entries(entries.as_slice(), &normalized);
            *cache = (revision, result.clone());
            return result;
        }

        let entries = {
            let snapshot = self.snapshot.lock().unwrap();
            Arc::clone(&snapshot.flat_cache)
        };

        rank_entries(entries.as_slice(), &normalized)
    }
}

impl Default for FileSearchIndex {
    fn default() -> Self {
        Self {
            root: Mutex::new(None),
            snapshot: Mutex::new(IndexSnapshot::default()),
            watcher: Mutex::new(None),
            current_generation: AtomicU64::new(1),
            completed_generation: AtomicU64::new(0),
            worker_generation: AtomicU64::new(0),
            watcher_id_seed: AtomicU64::new(0),
            revision: AtomicU64::new(0),
            empty_cache: Mutex::new((0, Vec::new())),
        }
    }
}

// ---------------------------------------------------------------------------
// @-fragment extraction
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Workspace walking
// ---------------------------------------------------------------------------

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

        // Skip .git/ internal entries — never useful as suggestions
        let rel_str = rel.to_string_lossy();
        if rel_str == ".git" || rel_str.starts_with(".git/") {
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

fn scan_directory_entries(workspace_root: &Path, dir: &str) -> Vec<IndexedEntry> {
    let dir_path = if dir.is_empty() {
        workspace_root.to_path_buf()
    } else {
        workspace_root.join(dir)
    };

    let Ok(read_dir) = std::fs::read_dir(&dir_path) else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(workspace_root) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }

        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let is_dir = metadata.is_dir();

        // Skip hidden files (starting with .)
        let file_name = rel.file_name();
        if file_name.is_some_and(|name| {
            name.to_string_lossy()
                .chars()
                .next()
                .is_some_and(|c| c == '.')
        }) {
            continue;
        }

        // Skip .git/ internal entries
        let rel_str = rel.to_string_lossy();
        if rel_str == ".git" || rel_str.starts_with(".git/") {
            continue;
        }

        let Some(indexed_entry) = build_indexed_entry(rel, &path, is_dir) else {
            continue;
        };
        entries.push(indexed_entry);
    }

    entries
}

fn build_indexed_entry(rel: &Path, path: &Path, is_dir: bool) -> Option<IndexedEntry> {
    let path_text = rel.to_string_lossy().into_owned();
    if path_text.is_empty() {
        return None;
    }

    let kind = if is_dir {
        FileEntryKind::Directory
    } else if is_image_path(path) {
        FileEntryKind::Image
    } else {
        FileEntryKind::File
    };

    let display = match kind {
        FileEntryKind::Directory => {
            let separator = std::path::MAIN_SEPARATOR;
            format!("{path_text}{separator}")
        }
        _ => path_text.clone(),
    };

    Some(IndexedEntry {
        lowercase_path: path_text.to_ascii_lowercase(),
        lowercase_name: rel
            .file_name()
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default(),
        basename_char_offset: basename_char_offset(&path_text),
        depth: path_text
            .chars()
            .filter(|&c| std::path::is_separator(c))
            .count() as u32,
        is_dotfile: rel
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.')),
        path: path_text,
        display,
        kind,
    })
}

// ---------------------------------------------------------------------------
// Ranking
// ---------------------------------------------------------------------------

fn rank_entries(indexed_entries: &[IndexedEntry], query: &str) -> Vec<FileSuggestion> {
    if query.is_empty() {
        let mut entries: Vec<&IndexedEntry> = indexed_entries.iter().collect();
        entries.sort_by(|left, right| {
            kind_rank(left.kind)
                .cmp(&kind_rank(right.kind))
                .then_with(|| left.depth.cmp(&right.depth))
                .then_with(|| left.is_dotfile.cmp(&right.is_dotfile))
                .then_with(|| left.display.cmp(&right.display))
        });
        entries.truncate(MAX_SUGGESTIONS);
        entries
            .into_iter()
            .map(|entry| entry.suggestion(Vec::new()))
            .collect()
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
                .then_with(|| left.is_dotfile.cmp(&right.is_dotfile))
                .then_with(|| left.display.cmp(&right.display))
        });
        ranked.truncate(MAX_SUGGESTIONS);

        ranked
            .into_iter()
            .map(|(_, entry, matched_indices)| entry.suggestion(matched_indices))
            .collect()
    }
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
        FileEntryKind::Directory => 20,
        FileEntryKind::Image => 8,
        FileEntryKind::File => 0,
    };
    let depth = entry.depth;
    let depth_penalty = depth * 6;
    let root_bonus = if depth == 0 { 50 } else { 0 };
    let dotfile_penalty = if entry.is_dotfile { 60 } else { 0 };

    best.map(|mut candidate| {
        candidate.score = candidate
            .score
            .saturating_sub(depth_penalty)
            .saturating_sub(dotfile_penalty)
            .saturating_add(kind_bonus)
            .saturating_add(root_bonus);
        candidate
    })
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

// ---------------------------------------------------------------------------
// Match helpers
// ---------------------------------------------------------------------------

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

fn kind_rank(kind: FileEntryKind) -> usize {
    match kind {
        FileEntryKind::Directory => 0,
        FileEntryKind::Image => 1,
        FileEntryKind::File => 2,
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_at_fragment() {
        assert_eq!(
            current_at_fragment("hello @wor", 10),
            Some((6, "wor".to_string()))
        );
        assert_eq!(current_at_fragment("hello world", 11), None);
        assert_eq!(current_at_fragment("hello @ world", 12), None);
        assert_eq!(current_at_fragment("hello@world", 11), None);
        assert_eq!(
            current_at_fragment("hello @world", 12),
            Some((6, "world".to_string()))
        );
        assert_eq!(current_at_fragment("hello @world", 5), None);
    }

    #[test]
    fn test_is_image_path() {
        assert!(is_image_path(Path::new("photo.png")));
        assert!(is_image_path(Path::new("photo.jpg")));
        assert!(is_image_path(Path::new("photo.jpeg")));
        assert!(is_image_path(Path::new("photo.webp")));
        assert!(is_image_path(Path::new("photo.gif")));
        assert!(!is_image_path(Path::new("file.txt")));
        assert!(!is_image_path(Path::new("file.rs")));
    }

    #[test]
    fn test_kind_rank() {
        assert_eq!(kind_rank(FileEntryKind::Directory), 0);
        assert_eq!(kind_rank(FileEntryKind::Image), 1);
        assert_eq!(kind_rank(FileEntryKind::File), 2);
    }

    #[test]
    fn test_find_subsequence_indices() {
        assert_eq!(
            find_subsequence_indices("abcdef", "ace"),
            Some(vec![0, 2, 4])
        );
        assert_eq!(find_subsequence_indices("abcdef", "xyz"), None);
        assert_eq!(find_subsequence_indices("abc", ""), None);
    }

    #[test]
    fn test_byte_offset_to_char_index() {
        assert_eq!(byte_offset_to_char_index("hello", 2), 2);
        // "你好世界": 你=bytes 0-2, 好=bytes 3-5, 世=bytes 6-8, 界=bytes 9-11
        // byte offset 6 → "你好" (6 bytes) → 2 chars
        assert_eq!(byte_offset_to_char_index("你好世界", 6), 2);
    }

    #[test]
    fn test_basename_char_offset() {
        assert_eq!(basename_char_offset("foo/bar/baz.txt"), 8);
        assert_eq!(basename_char_offset("single_file.rs"), 0);
    }

    #[test]
    fn test_empty_search_returns_empty_when_no_entries() {
        let index = FileSearchIndex::new();
        let results = index.search("");
        assert!(results.is_empty());
    }
}
