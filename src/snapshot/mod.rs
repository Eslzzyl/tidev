mod git;

use anyhow::{Context, Result};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;

use crate::config::ConfigPaths;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Patch {
    pub hash: String,
    pub files: Vec<String>,
}

pub struct SnapshotService {
    worktree: PathBuf,
    gitdir: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl SnapshotService {
    pub fn new(workspace_root: &Path, paths: &ConfigPaths) -> Result<Self> {
        let worktree = workspace_root.canonicalize()
            .with_context(|| format!("failed to canonicalize workspace root {}", workspace_root.display()))?;
        
        let worktree_hash = blake3::hash(worktree.to_string_lossy().as_bytes()).to_hex().to_string();
        
        let gitdir = paths.data_dir
            .join("snapshot")
            .join(&worktree_hash);
        
        Ok(Self {
            worktree,
            gitdir,
            lock: Arc::new(Mutex::new(())),
        })
    }

    pub async fn track(&self) -> Result<Option<String>> {
        let _guard = self.lock.lock().await;
        
        if !git::is_git_repository(&self.worktree)? {
            return Ok(None);
        }
        
        let existed = self.gitdir.exists();
        std::fs::create_dir_all(&self.gitdir)
            .with_context(|| format!("failed to create snapshot directory {}", self.gitdir.display()))?;
        
        if !existed {
            git::init_snapshot_repo(&self.gitdir)?;
        }
        
        git::sync_exclude(&self.gitdir, &self.worktree, &[])?;
        
        let all_files = git::find_changed_files(&self.gitdir, &self.worktree)?;
        if all_files.is_empty() {
            return Ok(None);
        }
        
        let ignored = git::check_ignored(&self.worktree, &all_files)?;
        let allowed: Vec<_> = all_files.iter()
            .filter(|f| !ignored.contains(*f))
            .cloned()
            .collect();
        
        if allowed.is_empty() {
            return Ok(None);
        }
        
        let large_files = git::filter_large_files(&self.worktree, &allowed, 2 * 1024 * 1024)?;
        let blocked: HashSet<_> = large_files.iter().cloned().collect();
        
        let to_stage: Vec<_> = allowed.iter()
            .filter(|f| !blocked.contains(*f))
            .cloned()
            .collect();
        
        if !large_files.is_empty() {
            git::sync_exclude(&self.gitdir, &self.worktree, &large_files)?;
        }
        
        git::stage_files(&self.gitdir, &self.worktree, &to_stage)?;
        
        let hash = git::write_tree(&self.gitdir)?;
        
        Ok(Some(hash))
    }

    pub async fn patch(&self, hash: &str) -> Result<Patch> {
        let _guard = self.lock.lock().await;
        
        let all_files = git::find_changed_files(&self.gitdir, &self.worktree)?;
        if !all_files.is_empty() {
            git::sync_exclude(&self.gitdir, &self.worktree, &[])?;
            git::stage_files(&self.gitdir, &self.worktree, &all_files)?;
        }
        
        let changed = git::diff_cached_names(&self.gitdir, &self.worktree, hash)?;
        
        let ignored = git::check_ignored(&self.worktree, &changed)?;
        let files: Vec<String> = changed.iter()
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
        
        for (hash, file, rel) in ops {
            self.revert_single(&hash, &file, &rel).await?;
        }
        
        Ok(())
    }

    async fn revert_single(&self, hash: &str, file: &str, rel: &str) -> Result<()> {
        match git::checkout_file(&self.gitdir, &self.worktree, hash, file) {
            Ok(()) => return Ok(()),
            Err(_) => {
                match git::ls_tree(&self.gitdir, hash, rel)? {
                    Some(_) => {
                        return Ok(());
                    }
                    None => {
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
                    }
                }
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
        
        let all_files = git::find_changed_files(&self.gitdir, &self.worktree)?;
        if !all_files.is_empty() {
            git::sync_exclude(&self.gitdir, &self.worktree, &[])?;
            git::stage_files(&self.gitdir, &self.worktree, &all_files)?;
        }
        
        git::diff_cached(&self.gitdir, &self.worktree, hash)
    }
}
