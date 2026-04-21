mod git;

use anyhow::{Context, Result};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;

use crate::config::ConfigPaths;

const BATCH_SIZE: usize = 100;

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
}

impl SnapshotService {
    pub fn new(workspace_root: &Path, paths: &ConfigPaths) -> Result<Self> {
        let worktree = workspace_root.canonicalize().with_context(|| {
            format!(
                "failed to canonicalize workspace root {}",
                workspace_root.display()
            )
        })?;

        let worktree_hash = blake3::hash(worktree.to_string_lossy().as_bytes())
            .to_hex()
            .to_string();

        let gitdir = paths.data_dir.join("snapshot").join(&worktree_hash);

        Ok(Self {
            worktree,
            gitdir,
            lock: Arc::new(Mutex::new(())),
        })
    }

    pub async fn track(&self) -> Result<Option<String>> {
        let _guard = self.lock.lock().await;

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

        let all_files = git::find_changed_files(&self.gitdir, &self.worktree)?;
        if all_files.is_empty() {
            return Ok(None);
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

        if allowed.is_empty() {
            return Ok(None);
        }

        let large_files = git::filter_large_files(&self.worktree, &allowed, 2 * 1024 * 1024)?;
        let blocked: HashSet<_> = large_files.iter().cloned().collect();

        let to_stage: Vec<_> = allowed
            .iter()
            .filter(|f| !blocked.contains(*f))
            .cloned()
            .collect();

        if !large_files.is_empty() {
            git::sync_exclude(&self.gitdir, &self.worktree, &large_files)?;
        } else {
            git::sync_exclude(&self.gitdir, &self.worktree, &[])?;
        }

        git::stage_files(&self.gitdir, &self.worktree, &to_stage)?;

        let hash = git::write_tree(&self.gitdir)?;

        Ok(Some(hash))
    }

    pub async fn patch(&self, hash: &str) -> Result<Patch> {
        let _guard = self.lock.lock().await;

        self.update_index()?;

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

        self.update_index()?;

        git::diff_cached(&self.gitdir, &self.worktree, hash)
    }

    pub async fn diff_full(&self, from: &str, to: &str) -> Result<Vec<FileDiff>> {
        let _guard = self.lock.lock().await;

        self.update_index()?;

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

    fn update_index(&self) -> Result<()> {
        let all_files = git::find_changed_files(&self.gitdir, &self.worktree)?;
        if all_files.is_empty() {
            return Ok(());
        }

        let ignored = git::check_ignored(&self.gitdir, &self.worktree, &all_files)?;

        if !ignored.is_empty() {
            let ignored_files: Vec<_> = ignored.iter().cloned().collect();
            git::drop_files(&self.gitdir, &self.worktree, &ignored_files)?;
        }

        git::sync_exclude(&self.gitdir, &self.worktree, &[])?;

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
