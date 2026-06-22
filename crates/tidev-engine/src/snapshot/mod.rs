mod git;

use anyhow::{Context, Result};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

use crate::config::{ConfigPaths, SnapshotConfig};
use crate::tooling::builtin::utils::canonicalize_display;

const BATCH_SIZE: usize = 100;

/// SHA-1 of Git's well-known empty tree. Used as a placeholder baseline
/// when a real snapshot is not yet available, e.g. while the very first
/// `track_async()` of a session is still running. **Do not store this
/// hash as a message's `snapshot_hash`** because reverting to it would
/// delete every file in the worktree. It is safe to use as an
/// in-memory step-tracking baseline because `capture_step_snapshot`
/// will replace it before any revert machinery touches it.
pub const EMPTY_TREE_HASH: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Patch {
    pub hash: String,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FileDiff {
    pub file: String,
    pub patch: String,
    pub additions: usize,
    pub deletions: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Clone)]
pub struct SnapshotService {
    worktree: PathBuf,
    gitdir: PathBuf,
    lock: Arc<Mutex<()>>,
    config: Arc<SnapshotConfig>,
}

impl SnapshotService {
    pub fn new(
        workspace_root: &Path,
        paths: &ConfigPaths,
        config: Arc<SnapshotConfig>,
    ) -> Result<Self> {
        let worktree = canonicalize_display(workspace_root);

        let worktree_hash = blake3::hash(worktree.to_string_lossy().as_bytes())
            .to_hex()
            .to_string();

        let gitdir = paths.data_dir.join("snapshot").join(&worktree_hash);

        Ok(Self {
            worktree,
            gitdir,
            lock: Arc::new(Mutex::new(())),
            config,
        })
    }

    /// Returns a reference to the snapshot configuration. Used by
    /// callers that need to know whether snapshot is enabled, etc.
    pub fn config(&self) -> &SnapshotConfig {
        &self.config
    }

    /// Synchronous wrapper. Existing callers (e.g. the TUI revert flow
    /// that already blocks on a `Runtime::block_on`) can keep using
    /// this. Prefer `track_async` in new code paths.
    ///
    /// Safe to call from both inside and outside a tokio runtime:
    /// - Inside: uses `block_in_place` so the inner `block_on` does
    ///   not panic with "Cannot start a runtime from within a runtime".
    /// - Outside: builds a temporary single-threaded runtime.
    pub fn track(&self) -> Result<Option<String>> {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            tokio::task::block_in_place(|| handle.block_on(self.track_async()))
        } else {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("failed to build temporary tokio runtime for track()")?;
            rt.block_on(self.track_async())
        }
    }

    /// Primary async entry point. Respects the configured
    /// `track_timeout_ms`: on timeout, returns `Ok(None)` and logs a
    /// warning. The lock is released on timeout; subsequent tracks
    /// benefit from any work the timed-out call did manage to do.
    pub async fn track_async(&self) -> Result<Option<String>> {
        let total_start = Instant::now();

        if !self.config.enabled {
            log::info!("snapshot: disabled by config; skipping track()");
            return Ok(None);
        }

        let inner = self.track_inner();
        let result = if self.config.track_timeout_ms > 0 {
            match tokio::time::timeout(
                Duration::from_millis(self.config.track_timeout_ms),
                inner,
            )
            .await
            {
                Ok(r) => r,
                Err(_) => {
                    log::warn!(
                        "snapshot: track() exceeded {}ms timeout; returning None",
                        self.config.track_timeout_ms,
                    );
                    return Ok(None);
                }
            }
        } else {
            inner.await
        };

        log::info!(
            "snapshot: track() total elapsed: {:?}",
            total_start.elapsed()
        );
        result
    }

    async fn track_inner(&self) -> Result<Option<String>> {
        let _guard = self.lock.lock().await;

        let phase = Instant::now();
        let existed = self.gitdir.exists();
        std::fs::create_dir_all(&self.gitdir).with_context(|| {
            format!(
                "failed to create snapshot directory {}",
                self.gitdir.display()
            )
        })?;
        if !existed {
            git::init_snapshot_repo(&self.gitdir)?;
        }
        log::info!("snapshot: init phase: {:?}", phase.elapsed());

        let phase = Instant::now();
        let all_files = git::find_changed_files(&self.gitdir, &self.worktree).await?;
        log::info!(
            "snapshot: find_changed_files returned {} files in {:?}",
            all_files.len(),
            phase.elapsed()
        );

        let ignored = if all_files.is_empty() {
            HashSet::new()
        } else {
            let phase = Instant::now();
            let ignored = git::check_ignored(&self.gitdir, &self.worktree, &all_files)?;
            log::info!(
                "snapshot: check_ignored: {:?} ({} ignored)",
                phase.elapsed(),
                ignored.len()
            );
            ignored
        };

        if !ignored.is_empty() {
            let ignored_files: Vec<_> = ignored.iter().cloned().collect();
            git::drop_files(&self.gitdir, &self.worktree, &ignored_files)?;
        }

        let mut allowed: Vec<String> = all_files
            .iter()
            .filter(|f| !ignored.contains(*f))
            .cloned()
            .collect();

        // Apply user-configured cap on number of files.
        if self.config.max_files > 0 && allowed.len() > self.config.max_files {
            log::warn!(
                "snapshot: file list ({} files) exceeds max_files={}; truncating",
                allowed.len(),
                self.config.max_files,
            );
            allowed.truncate(self.config.max_files);
        }

        let phase = Instant::now();
        let large_files = git::filter_large_files(
            &self.worktree,
            &allowed,
            self.config.max_file_size,
            self.config.stat_concurrency,
        )
        .await?;
        log::info!(
            "snapshot: filter_large_files: {:?} ({} large)",
            phase.elapsed(),
            large_files.len()
        );

        let blocked: HashSet<_> = large_files.iter().cloned().collect();
        let to_stage: Vec<_> = allowed
            .iter()
            .filter(|f| !blocked.contains(*f))
            .cloned()
            .collect();

        // Combine user-configured `ignore_globs` with auto-detected
        // large files into the snapshot's info/exclude. Git's
        // `check-ignore` already handled `.gitignore`/`.git/info/exclude`
        // for the file set; this path ensures user globs participate
        // in the same machinery on subsequent tracks.
        let mut exclude_set: Vec<String> = self.config.ignore_globs.clone();
        exclude_set.extend(large_files.iter().cloned());
        if !exclude_set.is_empty() {
            git::sync_exclude(&self.gitdir, &self.worktree, &exclude_set)?;
        } else {
            git::sync_exclude(&self.gitdir, &self.worktree, &[])?;
        }

        if !to_stage.is_empty() {
            git::stage_files(&self.gitdir, &self.worktree, &to_stage)?;
        }

        let hash = git::write_tree(&self.gitdir)?;

        Ok(Some(hash))
    }

    pub async fn patch(&self, hash: &str) -> Result<Patch> {
        let _guard = self.lock.lock().await;

        self.update_index().await?;

        let changed = git::diff_cached_names(&self.gitdir, &self.worktree, hash)?;

        let ignored = git::check_ignored(&self.gitdir, &self.worktree, &changed)?;
        let files: Vec<String> = changed
            .iter()
            .filter(|f| !ignored.contains(*f))
            .map(|f| self.worktree.join(f).to_string_lossy().replace('\\', "/"))
            .collect();

        Ok(Patch {
            hash: hash.to_string(),
            files,
        })
    }

    pub async fn revert(&self, patches: &[Patch]) -> Result<()> {
        let _guard = self.lock.lock().await;

        let mut ops: Vec<(String, String, String)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for patch in patches {
            for file in &patch.files {
                if seen.contains(file) {
                    continue;
                }
                seen.insert(file.clone());

                let rel = Path::new(file)
                    .strip_prefix(&self.worktree)
                    .with_context(|| format!("path {} is not under worktree", file))?
                    .to_string_lossy()
                    .replace('\\', "/");

                ops.push((patch.hash.clone(), file.clone(), rel));
            }
        }

        let mut i = 0;
        while i < ops.len() {
            let first = &ops[i];
            let mut batch_indices = vec![i];
            let mut j = i + 1;

            while j < ops.len() && batch_indices.len() < BATCH_SIZE {
                let next = &ops[j];
                if next.0 != first.0 {
                    break;
                }
                if batch_indices.iter().any(|&idx| clash(&ops[idx].2, &next.2)) {
                    break;
                }
                batch_indices.push(j);
                j += 1;
            }

            if batch_indices.len() == 1 {
                self.revert_single(&first.0, &first.1, &first.2)?;
            } else {
                let batch: Vec<_> = batch_indices.iter().map(|&idx| &ops[idx]).collect();
                self.revert_batch(&batch)?;
            }

            i = j;
        }

        Ok(())
    }

    fn revert_single(&self, hash: &str, file: &str, rel: &str) -> Result<()> {
        match git::checkout_file(&self.gitdir, &self.worktree, hash, file) {
            Ok(()) => return Ok(()),
            Err(_) => match git::ls_tree(&self.gitdir, hash, rel)? {
                Some(_) => {
                    return Ok(());
                }
                None => {
                    self.remove_path(file)?;
                }
            },
        }
        Ok(())
    }

    fn revert_batch(&self, batch: &[&(String, String, String)]) -> Result<()> {
        let hash = &batch[0].0;
        let rels: Vec<&str> = batch.iter().map(|op| op.2.as_str()).collect();

        let tree_output = git::ls_tree_names(&self.gitdir, hash, &rels)?;
        let have: HashSet<&str> = tree_output
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        let to_checkout: Vec<&str> = batch
            .iter()
            .filter(|op| have.contains(op.2.as_str()))
            .map(|op| op.1.as_str())
            .collect();

        if !to_checkout.is_empty()
            && let Err(_) = git::checkout_files(&self.gitdir, &self.worktree, hash, &to_checkout)
        {
            for op in batch {
                if have.contains(op.2.as_str()) {
                    self.revert_single(&op.0, &op.1, &op.2)?;
                }
            }
        }

        for op in batch {
            if !have.contains(op.2.as_str()) {
                self.remove_path(&op.1)?;
            }
        }

        Ok(())
    }

    fn remove_path(&self, file: &str) -> Result<()> {
        let path = Path::new(file);
        if path.exists() {
            if path.is_dir() {
                std::fs::remove_dir_all(path)
                    .with_context(|| format!("failed to remove directory {}", file))?;
            } else {
                std::fs::remove_file(path)
                    .with_context(|| format!("failed to remove file {}", file))?;
            }
        }
        Ok(())
    }

    pub async fn restore(&self, snapshot: &str) -> Result<()> {
        let _guard = self.lock.lock().await;

        git::read_tree(&self.gitdir, snapshot)?;
        git::checkout_index(&self.gitdir, &self.worktree)?;

        Ok(())
    }

    pub async fn cleanup(&self) -> Result<()> {
        let _guard = self.lock.lock().await;

        if !self.gitdir.exists() {
            return Ok(());
        }

        git::gc_prune(&self.gitdir, "7.days")?;

        Ok(())
    }

    pub async fn diff(&self, hash: &str) -> Result<String> {
        let _guard = self.lock.lock().await;

        self.update_index().await?;

        git::diff_cached(&self.gitdir, &self.worktree, hash)
    }

    /// Lightweight diff between two snapshot tree hashes.
    /// Returns FileDiff entries with file, additions, deletions, and status
    /// but WITHOUT full patch content. Used for per-step sidebar updates.
    /// Does NOT call update_index() since it compares two committed trees.
    /// Much cheaper than diff_full() — only 2 git subprocess calls total.
    pub async fn diff_lightweight(&self, from: &str, to: &str) -> Result<Vec<FileDiff>> {
        let _guard = self.lock.lock().await;

        let statuses = git::diff_name_status(&self.gitdir, &self.worktree, from, to)?;
        let numstat = git::diff_numstat(&self.gitdir, &self.worktree, from, to)?;

        let mut status_map: HashMap<String, String> = HashMap::new();
        for (status, file) in &statuses {
            let s = if status.starts_with('A') {
                "added"
            } else if status.starts_with('D') {
                "deleted"
            } else {
                "modified"
            };
            status_map.insert(file.clone(), s.to_string());
        }

        let mut result: Vec<FileDiff> = Vec::new();
        for (adds, dels, file) in &numstat {
            let binary = adds == "-" && dels == "-";
            result.push(FileDiff {
                file: file.clone(),
                patch: String::new(), // lightweight — no patch content
                additions: if binary { 0 } else { adds.parse().unwrap_or(0) },
                deletions: if binary { 0 } else { dels.parse().unwrap_or(0) },
                status: status_map.get(file).cloned(),
            });
        }

        Ok(result)
    }

    pub async fn diff_full(&self, from: &str, to: &str) -> Result<Vec<FileDiff>> {
        let _guard = self.lock.lock().await;

        self.update_index().await?;

        let statuses = git::diff_name_status(&self.gitdir, &self.worktree, from, to)?;
        let numstat = git::diff_numstat(&self.gitdir, &self.worktree, from, to)?;

        let mut status_map: HashMap<String, String> = HashMap::new();
        for (status, file) in &statuses {
            let s = if status.starts_with('A') {
                "added"
            } else if status.starts_with('D') {
                "deleted"
            } else {
                "modified"
            };
            status_map.insert(file.clone(), s.to_string());
        }

        let ignored = git::check_ignored(
            &self.gitdir,
            &self.worktree,
            &numstat
                .iter()
                .map(|(_, _, f)| f.clone())
                .collect::<Vec<_>>(),
        )?;

        let mut result: Vec<FileDiff> = Vec::new();
        let mut total_patch_size = 0;
        let max_patch_size = 10 * 1024 * 1024; // 10MB limit

        for (adds, dels, file) in &numstat {
            if ignored.contains(file) {
                continue;
            }

            let binary = adds == "-" && dels == "-";
            let additions = if binary { 0 } else { adds.parse().unwrap_or(0) };
            let deletions = if binary { 0 } else { dels.parse().unwrap_or(0) };

            let patch = if binary {
                String::new()
            } else {
                let content = git::diff_file(&self.gitdir, &self.worktree, from, to, file)?;
                total_patch_size += content.len();
                if total_patch_size > max_patch_size {
                    return Err(anyhow::anyhow!(
                        "Total diff size exceeded limit of {} bytes",
                        max_patch_size
                    ));
                }
                content
            };

            result.push(FileDiff {
                file: file.clone(),
                patch,
                additions,
                deletions,
                status: status_map.get(file).cloned(),
            });
        }

        Ok(result)
    }

    async fn update_index(&self) -> Result<()> {
        let all_files = git::find_changed_files(&self.gitdir, &self.worktree).await?;
        if all_files.is_empty() {
            return Ok(());
        }

        let ignored = git::check_ignored(&self.gitdir, &self.worktree, &all_files)?;

        if !ignored.is_empty() {
            let ignored_files: Vec<_> = ignored.iter().cloned().collect();
            git::drop_files(&self.gitdir, &self.worktree, &ignored_files)?;
        }

        let allowed: Vec<_> = all_files
            .iter()
            .filter(|f| !ignored.contains(*f))
            .cloned()
            .collect();

        if !allowed.is_empty() {
            git::stage_files(&self.gitdir, &self.worktree, &allowed)?;
        }

        Ok(())
    }
}

fn clash(a: &str, b: &str) -> bool {
    a == b || a.starts_with(&format!("{}/", b)) || b.starts_with(&format!("{}/", a))
}

#[cfg(test)]
mod tests {
    use super::{ConfigPaths, SnapshotConfig, SnapshotService};
    use std::sync::Arc;
    use std::{fs, path::PathBuf};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{}-{}", prefix, uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    fn default_snapshot_config() -> Arc<SnapshotConfig> {
        Arc::new(SnapshotConfig::default())
    }

    fn test_paths(data_dir: &std::path::Path) -> ConfigPaths {
        ConfigPaths {
            config_dir: data_dir.join("config"),
            data_dir: data_dir.to_path_buf(),
            config_file: data_dir.join("config/config.toml"),
            auth_file: data_dir.join("auth.json"),
            database_file: data_dir.join("sessions.sqlite3"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn track_returns_snapshot_even_when_worktree_is_clean() {
        let workspace_root = unique_temp_dir("tidev-snapshot-worktree");
        let data_dir = unique_temp_dir("tidev-snapshot-data");
        let file_path = workspace_root.join("note.txt");

        fs::write(&file_path, "hello\n").expect("file should be written");

        let paths = test_paths(&data_dir);

        let snapshot = SnapshotService::new(&workspace_root, &paths, default_snapshot_config())
            .expect("snapshot should init");

        let first = snapshot
            .track()
            .expect("initial track should succeed");
        assert!(first.is_some(), "initial track should capture a snapshot");

        let second = snapshot.track().expect("clean track should succeed");
        assert!(
            second.is_some(),
            "clean track should still capture a snapshot for redo"
        );
        assert_eq!(
            first, second,
            "clean worktree should produce the same tree hash"
        );

        let _ = fs::remove_dir_all(&workspace_root);
        let _ = fs::remove_dir_all(&data_dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn restore_round_trips_without_git_repo() {
        let workspace_root = unique_temp_dir("tidev-restore-worktree");
        let data_dir = unique_temp_dir("tidev-restore-data");
        let file_path = workspace_root.join("note.txt");

        fs::write(&file_path, "before\n").expect("file should be written");

        let paths = test_paths(&data_dir);

        let snapshot = SnapshotService::new(&workspace_root, &paths, default_snapshot_config())
            .expect("snapshot should init");
        let hash = snapshot
            .track()
            .expect("track should succeed")
            .expect("hash should exist");

        fs::write(&file_path, "after\n").expect("file should be modified");

        snapshot
            .restore(&hash)
            .await
            .expect("restore should succeed");

        assert_eq!(
            fs::read_to_string(&file_path).expect("file should be readable"),
            "before\n"
        );

        let _ = fs::remove_dir_all(&workspace_root);
        let _ = fs::remove_dir_all(&data_dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn revert_round_trips_without_git_repo() {
        let workspace_root = unique_temp_dir("tidev-revert-worktree");
        let data_dir = unique_temp_dir("tidev-revert-data");
        let file_path = workspace_root.join("note.txt");

        fs::write(&file_path, "before\n").expect("file should be written");

        let paths = test_paths(&data_dir);

        let snapshot = SnapshotService::new(&workspace_root, &paths, default_snapshot_config())
            .expect("snapshot should init");
        let hash = snapshot
            .track()
            .expect("track should succeed")
            .expect("hash should exist");

        fs::write(&file_path, "after\n").expect("file should be modified");

        let patch = snapshot.patch(&hash).await.expect("patch should succeed");
        assert!(
            !patch.files.is_empty(),
            "patch should include the modified file"
        );

        snapshot
            .revert(&[patch])
            .await
            .expect("revert should succeed");

        assert_eq!(
            fs::read_to_string(&file_path).expect("file should be readable"),
            "before\n"
        );

        let _ = fs::remove_dir_all(&workspace_root);
        let _ = fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn empty_tree_hash_is_the_git_well_known_constant() {
        // The empty-tree SHA is part of Git's stable contract; if this
        // ever changes we want a loud test failure rather than silent
        // corruption of revert targets.
        assert_eq!(
            super::EMPTY_TREE_HASH,
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
        );
    }

    #[test]
    fn snapshot_config_defaults_match_design() {
        let cfg = SnapshotConfig::default();
        assert!(cfg.enabled, "snapshot is enabled by default");
        assert!(
            cfg.ignore_globs.is_empty(),
            "no default ignore globs; users opt-in"
        );
        assert_eq!(cfg.max_file_size, 2 * 1024 * 1024, "2 MiB default");
        assert_eq!(cfg.max_files, 0, "no file-count cap by default");
        assert_eq!(cfg.track_timeout_ms, 30_000, "30s default timeout");
        assert_eq!(cfg.stat_concurrency, 8, "8-way default stat");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn track_async_returns_none_when_disabled() {
        let workspace_root = unique_temp_dir("tidev-snapshot-disabled");
        let data_dir = unique_temp_dir("tidev-snapshot-disabled-data");
        fs::write(workspace_root.join("file.txt"), "x").expect("write");

        let cfg = Arc::new(SnapshotConfig {
            enabled: false,
            ..SnapshotConfig::default()
        });
        let snapshot = SnapshotService::new(&workspace_root, &test_paths(&data_dir), cfg)
            .expect("snapshot should init");
        let res = snapshot.track_async().await.expect("track_async should not error");
        assert!(res.is_none(), "disabled snapshot returns None");

        let _ = fs::remove_dir_all(&workspace_root);
        let _ = fs::remove_dir_all(&data_dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn track_async_respects_timeout() {
        // Build a service whose track_timeout_ms is 1ms; even on a
        // fresh worktree the discovery phase has to do enough work
        // (process spawn + git init) to exceed that, so we should see
        // Ok(None) with a logged warning.
        let workspace_root = unique_temp_dir("tidev-snapshot-timeout");
        let data_dir = unique_temp_dir("tidev-snapshot-timeout-data");
        for i in 0..64 {
            fs::write(workspace_root.join(format!("file_{i}.txt")), "x").expect("write");
        }

        let cfg = Arc::new(SnapshotConfig {
            track_timeout_ms: 1,
            ..SnapshotConfig::default()
        });
        let snapshot = SnapshotService::new(&workspace_root, &test_paths(&data_dir), cfg)
            .expect("snapshot should init");
        let res = snapshot
            .track_async()
            .await
            .expect("track_async should not error on timeout");
        assert!(res.is_none(), "1ms timeout returns None");

        let _ = fs::remove_dir_all(&workspace_root);
        let _ = fs::remove_dir_all(&data_dir);
    }
}
