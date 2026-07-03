//! Low-level Git command helpers for the snapshot service.
//!
//! All functions operate on an isolated snapshot repository (separate `GIT_DIR`).

use anyhow::{Context, Result, bail};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    path::Path,
    process::Command,
};
use tokio::task::JoinSet;

/// Directories ignored by default in snapshot operations.
pub const DEFAULT_IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    ".venv", "venv", "env", ".env", "__pycache__",
    ".pytest_cache", ".mypy_cache", ".tox",
    "dist", "build", "target", ".cache",
    ".idea", ".vscode",
    ".DS_Store", "Thumbs.db",
    ".gradle", "Pods", ".terraform",
    ".next", ".nuxt", ".parcel-cache",
    "Library", "AppData",
    ".ssh", ".gnupg", ".aws",
];

// ---------------------------------------------------------------------------
// Repo initialisation
// ---------------------------------------------------------------------------

pub fn init_snapshot_repo(gitdir: &Path) -> Result<()> {
    if let Some(parent) = gitdir.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create snapshot directory {}", parent.display()))?;
    }

    let status = Command::new("git")
        .args(["init", "--bare"])
        .arg(&gitdir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run git init")?;

    if !status.success() {
        bail!("git init failed for snapshot repo");
    }

    for (key, value) in [
        ("core.autocrlf", "false"),
        ("core.longpaths", "true"),
        ("core.symlinks", "true"),
        ("core.fsmonitor", "false"),
        ("feature.manyFiles", "true"),
        ("index.version", "4"),
        ("index.threads", "true"),
        ("core.untrackedCache", "true"),
    ] {
        let status = Command::new("git")
            .args(["--git-dir", &gitdir.to_string_lossy(), "config", key, value])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .with_context(|| format!("failed to set git config {}", key))?;

        if !status.success() {
            bail!("git config {} failed", key);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Exclude file sync
// ---------------------------------------------------------------------------

pub fn sync_exclude(gitdir: &Path, worktree: &Path, extra: &[String]) -> Result<()> {
    let source_exclude = worktree.join(".git").join("info").join("exclude");
    let mut content = String::new();

    if source_exclude.exists()
        && let Ok(text) = fs::read_to_string(&source_exclude)
    {
        content.push_str(&text);
    }

    for item in extra {
        content.push_str(&format!("\n/{}", item.replace('\\', "/")));
    }

    let info_dir = gitdir.join("info");
    fs::create_dir_all(&info_dir)
        .with_context(|| format!("failed to create {}", info_dir.display()))?;

    let target = info_dir.join("exclude");
    fs::write(&target, content)
        .with_context(|| format!("failed to write {}", target.display()))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// File discovery
// ---------------------------------------------------------------------------

pub async fn find_changed_files(gitdir: &Path, worktree: &Path) -> Result<Vec<String>> {
    use tokio::process::Command as TokioCmd;

    let diff_args: Vec<OsString> = vec![
        OsString::from("-c"), OsString::from("core.autocrlf=false"),
        OsString::from("-c"), OsString::from("core.longpaths=true"),
        OsString::from("-c"), OsString::from("core.symlinks=true"),
        OsString::from("-c"), OsString::from("core.quotepath=false"),
        OsString::from("--git-dir"), gitdir.into(),
        OsString::from("--work-tree"), worktree.into(),
        OsString::from("diff-files"),
        OsString::from("--name-only"), OsString::from("-z"),
        OsString::from("--"), OsString::from("."),
    ];

    let ls_args: Vec<OsString> = vec![
        OsString::from("-c"), OsString::from("core.autocrlf=false"),
        OsString::from("-c"), OsString::from("core.longpaths=true"),
        OsString::from("-c"), OsString::from("core.symlinks=true"),
        OsString::from("-c"), OsString::from("core.quotepath=false"),
        OsString::from("--git-dir"), gitdir.into(),
        OsString::from("--work-tree"), worktree.into(),
        OsString::from("ls-files"),
        OsString::from("--others"), OsString::from("--exclude-standard"),
        OsString::from("-z"),
        OsString::from("--"), OsString::from("."),
    ];

    let (diff_output, untracked_output) = tokio::try_join!(
        TokioCmd::new("git").args(&diff_args).output(),
        TokioCmd::new("git").args(&ls_args).output(),
    )
    .map_err(|e| anyhow::anyhow!("failed to spawn git file-listing subprocess: {e}"))?;

    if !diff_output.status.success() {
        bail!("git diff-files failed: {}", String::from_utf8_lossy(&diff_output.stderr));
    }
    if !untracked_output.status.success() {
        bail!("git ls-files --others failed: {}", String::from_utf8_lossy(&untracked_output.stderr));
    }

    let tracked: Vec<String> = String::from_utf8_lossy(&diff_output.stdout)
        .split('\0').filter(|s| !s.is_empty()).map(String::from).collect();

    let untracked: Vec<String> = String::from_utf8_lossy(&untracked_output.stdout)
        .split('\0').filter(|s| !s.is_empty())
        .filter(|s| !should_ignore_path(s))
        .map(String::from).collect();

    let mut all = tracked;
    all.extend(untracked);
    all.sort();
    all.dedup();
    Ok(all)
}

fn should_ignore_path(path: &str) -> bool {
    let path = std::path::Path::new(path);
    path.components().any(|component| {
        if let std::path::Component::Normal(name) = component
            && let Some(name_str) = name.to_str()
        {
            return DEFAULT_IGNORED_DIRS.contains(&name_str);
        }
        false
    })
}

// ---------------------------------------------------------------------------
// Ignore checking
// ---------------------------------------------------------------------------

pub fn check_ignored(gitdir: &Path, worktree: &Path, files: &[String]) -> Result<HashSet<String>> {
    if files.is_empty() {
        return Ok(HashSet::new());
    }

    let input = files.join("\0") + "\0";
    let child = Command::new("git")
        .current_dir(worktree)
        .args([
            "-c", "core.autocrlf=false",
            "-c", "core.longpaths=true",
            "-c", "core.symlinks=true",
            "-c", "core.quotepath=false",
            "--git-dir", &gitdir.to_string_lossy(),
            "--work-tree", &worktree.to_string_lossy(),
            "check-ignore", "--no-index", "--stdin", "-z",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to spawn git check-ignore")?;

    if let Some(mut stdin) = child.stdin.as_ref() {
        use std::io::Write;
        stdin.write_all(input.as_bytes())
            .context("failed to write to git check-ignore stdin")?;
    }

    let result = child.wait_with_output()
        .context("failed to wait for git check-ignore")?;

    if result.status.code() == Some(0) || result.status.code() == Some(1) {
        let ignored: HashSet<String> = String::from_utf8_lossy(&result.stdout)
            .split('\0').filter(|s| !s.is_empty()).map(String::from).collect();
        return Ok(ignored);
    }

    Ok(HashSet::new())
}

// ---------------------------------------------------------------------------
// Large file filtering
// ---------------------------------------------------------------------------

pub async fn filter_large_files(
    worktree: &Path,
    files: &[String],
    limit: u64,
    concurrency: usize,
) -> Result<Vec<String>> {
    if files.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }

    let workers = concurrency.max(1).min(files.len());
    let chunk_size = (files.len() + workers - 1) / workers;
    let mut set: JoinSet<Vec<String>> = JoinSet::new();

    for chunk in files.chunks(chunk_size) {
        let worktree = worktree.to_path_buf();
        let chunk = chunk.to_vec();
        set.spawn_blocking(move || {
            let mut large = Vec::new();
            for file in &chunk {
                let path = worktree.join(file);
                match fs::metadata(&path) {
                    Ok(meta) => {
                        if meta.is_file() && meta.len() > limit {
                            large.push(file.clone());
                        }
                    }
                    Err(_) => continue,
                }
            }
            large
        });
    }

    let mut out = Vec::new();
    while let Some(res) = set.join_next().await {
        out.extend(res?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Staging
// ---------------------------------------------------------------------------

pub fn stage_files(gitdir: &Path, worktree: &Path, files: &[String]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }

    let input = files.join("\0") + "\0";
    let mut child = Command::new("git")
        .args([
            "-c", "core.autocrlf=false",
            "-c", "core.longpaths=true",
            "-c", "core.symlinks=true",
            "--git-dir", &gitdir.to_string_lossy(),
            "--work-tree", &worktree.to_string_lossy(),
            "add", "--all", "--sparse",
            "--pathspec-from-file=-", "--pathspec-file-nul",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to spawn git add")?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(input.as_bytes())
            .context("failed to write to git add stdin")?;
    }

    let _ = child.wait().context("failed to wait for git add")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Drop (remove from index)
// ---------------------------------------------------------------------------

pub fn drop_files(gitdir: &Path, worktree: &Path, files: &[String]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }

    let input = files.join("\0") + "\0";
    let mut child = Command::new("git")
        .args([
            "-c", "core.autocrlf=false",
            "-c", "core.longpaths=true",
            "-c", "core.symlinks=true",
            "--git-dir", &gitdir.to_string_lossy(),
            "--work-tree", &worktree.to_string_lossy(),
            "rm", "--cached", "-f", "--ignore-unmatch",
            "--pathspec-from-file=-", "--pathspec-file-nul",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to spawn git rm")?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(input.as_bytes())
            .context("failed to write to git rm stdin")?;
    }

    let _ = child.wait().context("failed to wait for git rm")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Write tree
// ---------------------------------------------------------------------------

pub fn write_tree(gitdir: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["--git-dir", &gitdir.to_string_lossy(), "write-tree"])
        .output()
        .context("failed to run git write-tree")?;

    if !output.status.success() {
        bail!("git write-tree failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// ---------------------------------------------------------------------------
// Diff helpers
// ---------------------------------------------------------------------------

pub fn diff_cached_names(gitdir: &Path, worktree: &Path, hash: &str) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args([
            "-c", "core.autocrlf=false",
            "-c", "core.longpaths=true",
            "-c", "core.symlinks=true",
            "-c", "core.quotepath=false",
            "--git-dir", &gitdir.to_string_lossy(),
            "--work-tree", &worktree.to_string_lossy(),
            "diff", "--cached", "--no-ext-diff", "--name-only",
            hash, "--", ".",
        ])
        .output()
        .context("failed to run git diff --cached")?;

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
}

pub fn diff_cached(gitdir: &Path, worktree: &Path, hash: &str) -> Result<String> {
    let output = Command::new("git")
        .args([
            "-c", "core.autocrlf=false",
            "-c", "core.longpaths=true",
            "-c", "core.symlinks=true",
            "-c", "core.quotepath=false",
            "--git-dir", &gitdir.to_string_lossy(),
            "--work-tree", &worktree.to_string_lossy(),
            "diff", "--cached", "--no-ext-diff",
            hash, "--", ".",
        ])
        .output()
        .context("failed to run git diff --cached")?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn diff_name_status(
    gitdir: &Path, worktree: &Path, from: &str, to: &str,
) -> Result<Vec<(String, String)>> {
    let output = Command::new("git")
        .args([
            "-c", "core.autocrlf=false",
            "-c", "core.longpaths=true",
            "-c", "core.symlinks=true",
            "-c", "core.quotepath=false",
            "--git-dir", &gitdir.to_string_lossy(),
            "--work-tree", &worktree.to_string_lossy(),
            "diff", "--no-ext-diff", "--name-status", "--no-renames",
            from, to, "--", ".",
        ])
        .output()
        .context("failed to run git diff --name-status")?;

    let mut result = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 2 {
            result.push((parts[0].to_string(), parts[1].to_string()));
        }
    }
    Ok(result)
}

pub fn diff_numstat(
    gitdir: &Path, worktree: &Path, from: &str, to: &str,
) -> Result<Vec<(String, String, String)>> {
    let output = Command::new("git")
        .args([
            "-c", "core.autocrlf=false",
            "-c", "core.longpaths=true",
            "-c", "core.symlinks=true",
            "-c", "core.quotepath=false",
            "--git-dir", &gitdir.to_string_lossy(),
            "--work-tree", &worktree.to_string_lossy(),
            "diff", "--no-ext-diff", "--numstat", "--no-renames",
            from, to, "--", ".",
        ])
        .output()
        .context("failed to run git diff --numstat")?;

    let mut result = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            result.push((parts[0].to_string(), parts[1].to_string(), parts[2].to_string()));
        }
    }
    Ok(result)
}

pub fn diff_file(
    gitdir: &Path, worktree: &Path, from: &str, to: &str, file: &str,
) -> Result<String> {
    let output = Command::new("git")
        .args([
            "-c", "core.autocrlf=false",
            "-c", "core.longpaths=true",
            "-c", "core.symlinks=true",
            "-c", "core.quotepath=false",
            "--git-dir", &gitdir.to_string_lossy(),
            "--work-tree", &worktree.to_string_lossy(),
            "diff", "--no-ext-diff", "--no-renames",
            from, to, "--", file,
        ])
        .output()
        .context("failed to run git diff for file")?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// ---------------------------------------------------------------------------
// Checkout helpers
// ---------------------------------------------------------------------------

pub fn checkout_file(gitdir: &Path, worktree: &Path, hash: &str, file: &str) -> Result<()> {
    let status = Command::new("git")
        .args([
            "-c", "core.longpaths=true",
            "-c", "core.symlinks=true",
            "--git-dir", &gitdir.to_string_lossy(),
            "--work-tree", &worktree.to_string_lossy(),
            "checkout", hash, "--", file,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run git checkout")?;

    if !status.success() {
        bail!("git checkout failed");
    }
    Ok(())
}

pub fn checkout_files(gitdir: &Path, worktree: &Path, hash: &str, files: &[&str]) -> Result<()> {
    let mut args: Vec<OsString> = vec![
        OsString::from("-c"), OsString::from("core.longpaths=true"),
        OsString::from("-c"), OsString::from("core.symlinks=true"),
        OsString::from("--git-dir"), OsString::from(gitdir.to_string_lossy().to_string()),
        OsString::from("--work-tree"), OsString::from(worktree.to_string_lossy().to_string()),
        OsString::from("checkout"), OsString::from(hash),
        OsString::from("--"),
    ];
    args.extend(files.iter().map(|f| OsString::from(*f)));

    let status = Command::new("git")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run git checkout")?;

    if !status.success() {
        bail!("git checkout failed");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ls-tree helpers
// ---------------------------------------------------------------------------

pub fn ls_tree(gitdir: &Path, hash: &str, rel: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args([
            "-c", "core.longpaths=true",
            "-c", "core.symlinks=true",
            "--git-dir", &gitdir.to_string_lossy(),
            "ls-tree", hash, "--", rel,
        ])
        .output()
        .context("failed to run git ls-tree")?;

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() { Ok(None) } else { Ok(Some(text)) }
}

pub fn ls_tree_names(gitdir: &Path, hash: &str, rels: &[&str]) -> Result<String> {
    let mut args: Vec<OsString> = vec![
        OsString::from("-c"), OsString::from("core.longpaths=true"),
        OsString::from("-c"), OsString::from("core.symlinks=true"),
        OsString::from("--git-dir"), OsString::from(gitdir.to_string_lossy().to_string()),
        OsString::from("ls-tree"), OsString::from("--name-only"),
        OsString::from(hash), OsString::from("--"),
    ];
    args.extend(rels.iter().map(|r| OsString::from(*r)));

    let output = Command::new("git")
        .args(&args)
        .output()
        .context("failed to run git ls-tree")?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// ---------------------------------------------------------------------------
// Read-tree / checkout-index
// ---------------------------------------------------------------------------

pub fn read_tree(gitdir: &Path, hash: &str) -> Result<()> {
    let status = Command::new("git")
        .args([
            "-c", "core.longpaths=true",
            "-c", "core.symlinks=true",
            "--git-dir", &gitdir.to_string_lossy(),
            "read-tree", hash,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run git read-tree")?;

    if !status.success() {
        bail!("git read-tree failed");
    }
    Ok(())
}

pub fn checkout_index(gitdir: &Path, worktree: &Path) -> Result<()> {
    let status = Command::new("git")
        .args([
            "-c", "core.longpaths=true",
            "-c", "core.symlinks=true",
            "--git-dir", &gitdir.to_string_lossy(),
            "--work-tree", &worktree.to_string_lossy(),
            "checkout-index", "-a", "-f",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run git checkout-index")?;

    if !status.success() {
        bail!("git checkout-index failed");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Maintenance
// ---------------------------------------------------------------------------

pub fn gc_prune(gitdir: &Path, period: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["--git-dir", &gitdir.to_string_lossy(), "gc", &format!("--prune={period}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run git gc")?;

    if !status.success() {
        bail!("git gc failed");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::should_ignore_path;

    #[test]
    fn ignores_git_metadata_paths() {
        assert!(should_ignore_path(".git"));
        assert!(should_ignore_path(".git/config"));
        assert!(should_ignore_path("repo/.git/info/exclude"));
        assert!(!should_ignore_path(".gitignore"));
        assert!(!should_ignore_path("src/git.rs"));
    }

    #[test]
    fn ignores_common_build_outputs() {
        assert!(should_ignore_path("node_modules/foo"));
        assert!(should_ignore_path("project/node_modules/foo"));
        assert!(should_ignore_path("target/debug/build"));
        assert!(should_ignore_path("dist/bundle.js"));
        assert!(should_ignore_path("build/output"));
        assert!(should_ignore_path(".venv/lib/python"));
        assert!(should_ignore_path("__pycache__/x.pyc"));
        assert!(should_ignore_path(".pytest_cache/v"));
        assert!(should_ignore_path(".mypy_cache/x"));
        assert!(should_ignore_path(".tox/x"));
        assert!(should_ignore_path(".cache/foo"));
    }

    #[test]
    fn ignores_ide_and_platform_junk() {
        assert!(should_ignore_path(".idea/workspace.xml"));
        assert!(should_ignore_path(".vscode/settings.json"));
        assert!(should_ignore_path(".DS_Store"));
        assert!(should_ignore_path("Thumbs.db"));
    }

    #[test]
    fn ignores_home_directory_bloat() {
        assert!(should_ignore_path("Library/Caches/foo"));
        assert!(should_ignore_path("AppData/Local/foo"));
        assert!(should_ignore_path(".ssh/id_rsa"));
        assert!(should_ignore_path(".gnupg/pubring.kbx"));
        assert!(should_ignore_path(".aws/credentials"));
        assert!(should_ignore_path(".next/cache/webpack"));
        assert!(should_ignore_path(".nuxt/dist"));
        assert!(should_ignore_path(".parcel-cache/x"));
        assert!(should_ignore_path(".gradle/caches"));
        assert!(should_ignore_path("Pods/Headers"));
        assert!(should_ignore_path(".terraform/plugins"));
    }

    #[test]
    fn does_not_match_substring_components() {
        assert!(!should_ignore_path("node_modules_legacy/foo"));
        assert!(!should_ignore_path("my_target/x"));
        assert!(!should_ignore_path("src/LibraryParser.rs"));
    }
}
