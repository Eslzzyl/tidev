use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub tracked_patch: String,
    pub untracked_files: Vec<SnapshotFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotFile {
    pub path: PathBuf,
    pub content: Vec<u8>,
    pub executable: bool,
}

impl WorkspaceSnapshot {
    pub fn capture(workspace_root: &Path) -> Result<Self> {
        ensure_git_repository(workspace_root)?;

        let tracked_patch = run_git_output(
            workspace_root,
            [
                "diff",
                "--binary",
                "--no-color",
                "--no-ext-diff",
                "HEAD",
                "--",
                ".",
            ],
        )?;

        let untracked_output = run_git_output(
            workspace_root,
            ["ls-files", "--others", "--exclude-standard", "-z"],
        )?;

        let mut untracked_files = Vec::new();
        for path in untracked_output.split_terminator('\0') {
            if path.is_empty() {
                continue;
            }

            let file_path = workspace_root.join(path);
            let content = fs::read(&file_path)
                .with_context(|| format!("failed to read {}", file_path.display()))?;
            let executable = is_executable(&file_path)?;

            untracked_files.push(SnapshotFile {
                path: PathBuf::from(path),
                content,
                executable,
            });
        }

        Ok(Self {
            tracked_patch,
            untracked_files,
        })
    }

    pub fn restore(&self, workspace_root: &Path) -> Result<()> {
        ensure_git_repository(workspace_root)?;

        run_git_status(workspace_root, ["reset", "--hard", "HEAD"])?;
        run_git_status(workspace_root, ["clean", "-fd"])?;

        if !self.tracked_patch.trim().is_empty() {
            let mut child = Command::new("git")
                .arg("-C")
                .arg(workspace_root)
                .args(["apply", "--binary", "--whitespace=nowarn", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .context("failed to start git apply")?;

            if let Some(stdin) = child.stdin.as_mut() {
                stdin
                    .write_all(self.tracked_patch.as_bytes())
                    .context("failed to stream patch to git apply")?;
            }

            let output = child
                .wait_with_output()
                .context("failed to finish git apply")?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("git apply failed: {stderr}");
            }
        }

        for file in &self.untracked_files {
            let file_path = workspace_root.join(&file.path);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create directory {}", parent.display()))?;
            }

            fs::write(&file_path, &file.content)
                .with_context(|| format!("failed to write {}", file_path.display()))?;
            set_executable_bit(&file_path, file.executable)?;
        }

        Ok(())
    }
}

fn ensure_git_repository(workspace_root: &Path) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("failed to inspect git repository")?;

    if !output.status.success() {
        bail!("workspace root is not a git repository");
    }

    Ok(())
}

fn run_git_output<const N: usize>(workspace_root: &Path, args: [&str; N]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to run git {:?}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {:?} failed: {stderr}", args);
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_git_status<const N: usize>(workspace_root: &Path, args: [&str; N]) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to run git {:?}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {:?} failed: {stderr}", args);
    }

    Ok(())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;

    Ok(fs::metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?
        .permissions()
        .mode()
        & 0o111
        != 0)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> Result<bool> {
    Ok(false)
}

#[cfg(unix)]
fn set_executable_bit(path: &Path, executable: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata =
        fs::metadata(path).with_context(|| format!("failed to inspect {}", path.display()))?;
    let mut permissions = metadata.permissions();
    let mut mode = permissions.mode();
    if executable {
        mode |= 0o111;
    } else {
        mode &= !0o111;
    }
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("failed to update permissions for {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable_bit(_path: &Path, _executable: bool) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::SystemTime};

    fn run_git(workspace_root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(workspace_root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "TiDev")
            .env("GIT_AUTHOR_EMAIL", "tidev@example.com")
            .env("GIT_COMMITTER_NAME", "TiDev")
            .env("GIT_COMMITTER_EMAIL", "tidev@example.com")
            .status()
            .expect("git command should run");
        assert!(status.success(), "git {:?} failed", args);
    }

    #[test]
    fn snapshot_restore_brings_back_tracked_and_untracked_files() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let workspace_root = std::env::temp_dir().join(format!("tidev-snapshot-{unique}"));
        fs::create_dir_all(&workspace_root).expect("workspace should be created");

        fs::write(workspace_root.join("tracked.txt"), "base\n").expect("seed file");
        run_git(&workspace_root, &["init", "-q"]);
        run_git(&workspace_root, &["add", "tracked.txt"]);
        run_git(
            &workspace_root,
            &[
                "-c",
                "user.name=TiDev",
                "-c",
                "user.email=tidev@example.com",
                "commit",
                "-q",
                "-m",
                "init",
            ],
        );

        let snapshot = WorkspaceSnapshot::capture(&workspace_root).expect("snapshot should capture");

        fs::write(workspace_root.join("tracked.txt"), "changed\n").expect("modify tracked file");
        fs::write(workspace_root.join("new.txt"), "new\n").expect("create untracked file");

        snapshot.restore(&workspace_root).expect("snapshot should restore");

        let tracked = fs::read_to_string(workspace_root.join("tracked.txt")).expect("tracked file should exist");
        assert_eq!(tracked, "base\n");
        assert!(!workspace_root.join("new.txt").exists());

        let _ = fs::remove_dir_all(&workspace_root);
    }
}
