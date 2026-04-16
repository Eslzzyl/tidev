use anyhow::{bail, Context, Result};
use std::{collections::HashSet, ffi::OsString, fs, path::Path, process::Command};

const DEFAULT_IGNORED_DIRS: &[&str] = &[
    "node_modules",
    ".venv",
    "venv",
    "env",
    ".env",
    "dist",
    "build",
    ".pytest_cache",
    ".mypy_cache",
    ".cache",
    ".tox",
    "__pycache__",
    "target",
];

pub fn is_git_repository(path: &Path) -> Result<bool> {
    let output = Command::new("git")
        .current_dir(path)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .context("failed to run git rev-parse")?;

    Ok(output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true")
}

pub fn init_snapshot_repo(gitdir: &Path) -> Result<()> {
    let status = Command::new("git")
        .args(["init"])
        .env("GIT_DIR", gitdir)
        .env("GIT_WORK_TREE", ".")
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

pub fn sync_exclude(gitdir: &Path, worktree: &Path, extra: &[String]) -> Result<()> {
    let source_exclude = worktree.join(".git").join("info").join("exclude");

    let mut content = String::new();

    if source_exclude.exists() {
        if let Ok(text) = fs::read_to_string(&source_exclude) {
            content.push_str(&text);
        }
    }

    for item in extra {
        content.push_str(&format!("\n/{}", item.replace('\\', "/")));
    }

    let info_dir = gitdir.join("info");
    fs::create_dir_all(&info_dir)
        .with_context(|| format!("failed to create {}", info_dir.display()))?;

    let target = info_dir.join("exclude");
    fs::write(&target, content).with_context(|| format!("failed to write {}", target.display()))?;

    Ok(())
}

pub fn find_changed_files(gitdir: &Path, worktree: &Path) -> Result<Vec<String>> {
    let args: Vec<OsString> = vec![
        OsString::from("-c"),
        OsString::from("core.autocrlf=false"),
        OsString::from("-c"),
        OsString::from("core.longpaths=true"),
        OsString::from("-c"),
        OsString::from("core.symlinks=true"),
        OsString::from("-c"),
        OsString::from("core.quotepath=false"),
        OsString::from("--git-dir"),
        gitdir.into(),
        OsString::from("--work-tree"),
        worktree.into(),
        OsString::from("diff-files"),
        OsString::from("--name-only"),
        OsString::from("-z"),
        OsString::from("--"),
        OsString::from("."),
    ];

    let diff_output = Command::new("git")
        .args(&args)
        .output()
        .context("failed to run git diff-files")?;

    let tracked: Vec<String> = String::from_utf8_lossy(&diff_output.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    let args: Vec<OsString> = vec![
        OsString::from("-c"),
        OsString::from("core.autocrlf=false"),
        OsString::from("-c"),
        OsString::from("core.longpaths=true"),
        OsString::from("-c"),
        OsString::from("core.symlinks=true"),
        OsString::from("-c"),
        OsString::from("core.quotepath=false"),
        OsString::from("--git-dir"),
        gitdir.into(),
        OsString::from("--work-tree"),
        worktree.into(),
        OsString::from("ls-files"),
        OsString::from("--others"),
        OsString::from("--exclude-standard"),
        OsString::from("-z"),
        OsString::from("--"),
        OsString::from("."),
    ];

    let untracked_output = Command::new("git")
        .args(&args)
        .output()
        .context("failed to run git ls-files")?;

    let untracked: Vec<String> = String::from_utf8_lossy(&untracked_output.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .filter(|s| !should_ignore_path(s))
        .map(|s| s.to_string())
        .collect();

    let mut all: Vec<String> = tracked;
    all.extend(untracked);
    all.sort();
    all.dedup();

    Ok(all)
}

fn should_ignore_path(path: &str) -> bool {
    let path = Path::new(path);
    path.components().any(|component| {
        if let std::path::Component::Normal(name) = component {
            if let Some(name_str) = name.to_str() {
                return DEFAULT_IGNORED_DIRS.contains(&name_str);
            }
        }
        false
    })
}

pub fn check_ignored(worktree: &Path, files: &[String]) -> Result<HashSet<String>> {
    if files.is_empty() {
        return Ok(HashSet::new());
    }

    let input = files.join("\0") + "\0";

    let output = Command::new("git")
        .current_dir(worktree)
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.longpaths=true",
            "-c",
            "core.symlinks=true",
            "-c",
            "core.quotepath=false",
            "--git-dir",
            &worktree.join(".git").to_string_lossy(),
            "--work-tree",
            &worktree.to_string_lossy(),
            "check-ignore",
            "--no-index",
            "--stdin",
            "-z",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to spawn git check-ignore")?;

    if let Some(mut stdin) = output.stdin.as_ref() {
        use std::io::Write;
        stdin
            .write_all(input.as_bytes())
            .context("failed to write to git check-ignore stdin")?;
    }

    let result = output
        .wait_with_output()
        .context("failed to wait for git check-ignore")?;

    if result.status.code() == Some(0) || result.status.code() == Some(1) {
        let ignored: HashSet<String> = String::from_utf8_lossy(&result.stdout)
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        return Ok(ignored);
    }

    Ok(HashSet::new())
}

pub fn filter_large_files(worktree: &Path, files: &[String], limit: u64) -> Result<Vec<String>> {
    let mut large = Vec::new();

    for file in files {
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

    Ok(large)
}

pub fn stage_files(gitdir: &Path, worktree: &Path, files: &[String]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }

    let input = files.join("\0") + "\0";

    let mut child = Command::new("git")
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.longpaths=true",
            "-c",
            "core.symlinks=true",
            "--git-dir",
            &gitdir.to_string_lossy(),
            "--work-tree",
            &worktree.to_string_lossy(),
            "add",
            "--all",
            "--sparse",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to spawn git add")?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(input.as_bytes())
            .context("failed to write to git add stdin")?;
    }

    let status = child.wait().context("failed to wait for git add")?;

    let _ = status;

    Ok(())
}

pub fn write_tree(gitdir: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["--git-dir", &gitdir.to_string_lossy(), "write-tree"])
        .output()
        .context("failed to run git write-tree")?;

    if !output.status.success() {
        bail!(
            "git write-tree failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn diff_cached_names(gitdir: &Path, worktree: &Path, hash: &str) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.longpaths=true",
            "-c",
            "core.symlinks=true",
            "-c",
            "core.quotepath=false",
            "--git-dir",
            &gitdir.to_string_lossy(),
            "--work-tree",
            &worktree.to_string_lossy(),
            "diff",
            "--cached",
            "--no-ext-diff",
            "--name-only",
            hash,
            "--",
            ".",
        ])
        .output()
        .context("failed to run git diff --cached")?;

    let files: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Ok(files)
}

pub fn diff_cached(gitdir: &Path, worktree: &Path, hash: &str) -> Result<String> {
    let output = Command::new("git")
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.longpaths=true",
            "-c",
            "core.symlinks=true",
            "-c",
            "core.quotepath=false",
            "--git-dir",
            &gitdir.to_string_lossy(),
            "--work-tree",
            &worktree.to_string_lossy(),
            "diff",
            "--cached",
            "--no-ext-diff",
            hash,
            "--",
            ".",
        ])
        .output()
        .context("failed to run git diff --cached")?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn checkout_file(gitdir: &Path, worktree: &Path, hash: &str, file: &str) -> Result<()> {
    let status = Command::new("git")
        .args([
            "-c",
            "core.longpaths=true",
            "-c",
            "core.symlinks=true",
            "--git-dir",
            &gitdir.to_string_lossy(),
            "--work-tree",
            &worktree.to_string_lossy(),
            "checkout",
            hash,
            "--",
            file,
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

pub fn ls_tree(gitdir: &Path, hash: &str, rel: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args([
            "-c",
            "core.longpaths=true",
            "-c",
            "core.symlinks=true",
            "--git-dir",
            &gitdir.to_string_lossy(),
            "ls-tree",
            hash,
            "--",
            rel,
        ])
        .output()
        .context("failed to run git ls-tree")?;

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if text.is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}

pub fn read_tree(gitdir: &Path, hash: &str) -> Result<()> {
    let status = Command::new("git")
        .args([
            "-c",
            "core.longpaths=true",
            "-c",
            "core.symlinks=true",
            "--git-dir",
            &gitdir.to_string_lossy(),
            "read-tree",
            hash,
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
            "-c",
            "core.longpaths=true",
            "-c",
            "core.symlinks=true",
            "--git-dir",
            &gitdir.to_string_lossy(),
            "--work-tree",
            &worktree.to_string_lossy(),
            "checkout-index",
            "-a",
            "-f",
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

pub fn gc_prune(gitdir: &Path, period: &str) -> Result<()> {
    let status = Command::new("git")
        .args([
            "--git-dir",
            &gitdir.to_string_lossy(),
            "gc",
            &format!("--prune={}", period),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run git gc")?;

    if !status.success() {
        bail!("git gc failed");
    }

    Ok(())
}

pub fn drop_files(gitdir: &Path, worktree: &Path, files: &[String]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }

    let input = files.join("\0") + "\0";

    let mut child = Command::new("git")
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.longpaths=true",
            "-c",
            "core.symlinks=true",
            "--git-dir",
            &gitdir.to_string_lossy(),
            "--work-tree",
            &worktree.to_string_lossy(),
            "rm",
            "--cached",
            "-f",
            "--ignore-unmatch",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to spawn git rm")?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(input.as_bytes())
            .context("failed to write to git rm stdin")?;
    }

    let _ = child.wait().context("failed to wait for git rm")?;

    Ok(())
}

pub fn ls_tree_names(gitdir: &Path, hash: &str, rels: &[&str]) -> Result<String> {
    let mut args: Vec<OsString> = vec![
        OsString::from("-c"),
        OsString::from("core.longpaths=true"),
        OsString::from("-c"),
        OsString::from("core.symlinks=true"),
        OsString::from("--git-dir"),
        OsString::from(gitdir.to_string_lossy().to_string()),
        OsString::from("ls-tree"),
        OsString::from("--name-only"),
        OsString::from(hash),
        OsString::from("--"),
    ];
    args.extend(rels.iter().map(|r| OsString::from(*r)));

    let output = Command::new("git")
        .args(&args)
        .output()
        .context("failed to run git ls-tree")?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn checkout_files(gitdir: &Path, worktree: &Path, hash: &str, files: &[&str]) -> Result<()> {
    let mut args: Vec<OsString> = vec![
        OsString::from("-c"),
        OsString::from("core.longpaths=true"),
        OsString::from("-c"),
        OsString::from("core.symlinks=true"),
        OsString::from("--git-dir"),
        OsString::from(gitdir.to_string_lossy().to_string()),
        OsString::from("--work-tree"),
        OsString::from(worktree.to_string_lossy().to_string()),
        OsString::from("checkout"),
        OsString::from(hash),
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

pub fn diff_name_status(
    gitdir: &Path,
    worktree: &Path,
    from: &str,
    to: &str,
) -> Result<Vec<(String, String)>> {
    let output = Command::new("git")
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.longpaths=true",
            "-c",
            "core.symlinks=true",
            "-c",
            "core.quotepath=false",
            "--git-dir",
            &gitdir.to_string_lossy(),
            "--work-tree",
            &worktree.to_string_lossy(),
            "diff",
            "--no-ext-diff",
            "--name-status",
            "--no-renames",
            from,
            to,
            "--",
            ".",
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
    gitdir: &Path,
    worktree: &Path,
    from: &str,
    to: &str,
) -> Result<Vec<(String, String, String)>> {
    let output = Command::new("git")
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.longpaths=true",
            "-c",
            "core.symlinks=true",
            "-c",
            "core.quotepath=false",
            "--git-dir",
            &gitdir.to_string_lossy(),
            "--work-tree",
            &worktree.to_string_lossy(),
            "diff",
            "--no-ext-diff",
            "--no-renames",
            "--numstat",
            from,
            to,
            "--",
            ".",
        ])
        .output()
        .context("failed to run git diff --numstat")?;

    let mut result = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            result.push((
                parts[0].to_string(),
                parts[1].to_string(),
                parts[2].to_string(),
            ));
        }
    }

    Ok(result)
}

pub fn diff_file(
    gitdir: &Path,
    worktree: &Path,
    from: &str,
    to: &str,
    file: &str,
) -> Result<String> {
    let output = Command::new("git")
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.longpaths=true",
            "-c",
            "core.symlinks=true",
            "-c",
            "core.quotepath=false",
            "--git-dir",
            &gitdir.to_string_lossy(),
            "--work-tree",
            &worktree.to_string_lossy(),
            "diff",
            "--no-ext-diff",
            "--no-renames",
            from,
            to,
            "--",
            file,
        ])
        .output()
        .context("failed to run git diff for file")?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
