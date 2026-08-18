//! Read-only Git queries for the current workspace.
//!
//! This module deliberately operates on the user's repository. It is separate
//! from `tidev-snapshot`, whose Git repository is an internal undo/redo store.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const MAX_HISTORY_LIMIT: usize = 200;
const MAX_DIFF_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct GitService {
    workspace_root: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitError {
    NotRepository { path: PathBuf },
    WorkspaceMissing { path: PathBuf },
    GitUnavailable,
    CommandFailed { command: String, message: String },
    InvalidOutput(String),
}

impl fmt::Display for GitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRepository { path } => {
                write!(
                    formatter,
                    "No Git repository found in {}",
                    tidev_utils::path::display_path_with_tilde(path)
                )
            }
            Self::WorkspaceMissing { path } => {
                write!(
                    formatter,
                    "Workspace directory does not exist: {}",
                    tidev_utils::path::display_path_with_tilde(path)
                )
            }
            Self::GitUnavailable => formatter.write_str("Git executable was not found"),
            Self::CommandFailed { command, message } => {
                write!(formatter, "git {command} failed: {message}")
            }
            Self::InvalidOutput(message) => {
                write!(formatter, "Git returned invalid output: {message}")
            }
        }
    }
}

impl std::error::Error for GitError {}

pub type GitResult<T> = std::result::Result<T, GitError>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitRepoInfo {
    pub root: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub detached: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitChangeKind {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    Conflicted,
    Untracked,
    TypeChanged,
    Unknown,
}

impl GitChangeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Modified => "modified",
            Self::Renamed => "renamed",
            Self::Copied => "copied",
            Self::Conflicted => "conflicted",
            Self::Untracked => "untracked",
            Self::TypeChanged => "type changed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitFileStatus {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: GitChangeKind,
    pub index_code: char,
    pub worktree_code: char,
    pub staged: bool,
    pub unstaged: bool,
    pub conflict: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GitStatusCounts {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub conflicted: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitStatusSnapshot {
    pub repo: GitRepoInfo,
    pub files: Vec<GitFileStatus>,
    pub counts: GitStatusCounts,
    pub refreshed_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitCommitSummary {
    pub id: String,
    pub short_id: String,
    pub author: String,
    pub author_email: String,
    pub authored_at: String,
    pub refs: Vec<String>,
    pub subject: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitHistoryPage {
    pub head: Option<String>,
    pub commits: Vec<GitCommitSummary>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GitDiffScope {
    Worktree,
    Staged,
    Commit(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitDiffFile {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: GitChangeKind,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
    pub patch: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitDiffSnapshot {
    pub scope: GitDiffScope,
    pub files: Vec<GitDiffFile>,
    pub patch: String,
    pub truncated: bool,
}

impl GitService {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub async fn repo_info(&self) -> GitResult<GitRepoInfo> {
        let root = self
            .git_text(&["rev-parse", "--show-toplevel"])
            .await?
            .trim()
            .to_string();
        if root.is_empty() {
            return Err(GitError::InvalidOutput(
                "repository root is empty".to_string(),
            ));
        }

        let head = self
            .git_optional_text(&["rev-parse", "--verify", "HEAD"])
            .await;
        let branch = self
            .git_optional_text(&["branch", "--show-current"])
            .await
            .filter(|value| !value.trim().is_empty());
        let upstream = self
            .git_optional_text(&[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ])
            .await
            .filter(|value| !value.trim().is_empty());

        let (ahead, behind) = if upstream.is_some() && head.is_some() {
            self.git_optional_text(&["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])
                .await
                .and_then(|value| {
                    let mut parts = value.split_whitespace();
                    let ahead = parts.next()?.parse().ok()?;
                    let behind = parts.next()?.parse().ok()?;
                    Some((Some(ahead), Some(behind)))
                })
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        let detached = branch.is_none();
        Ok(GitRepoInfo {
            root: PathBuf::from(root),
            head,
            branch: branch.map(|value| value.trim().to_string()),
            upstream: upstream.map(|value| value.trim().to_string()),
            ahead,
            behind,
            detached,
        })
    }

    pub async fn status(&self) -> GitResult<GitStatusSnapshot> {
        let repo = self.repo_info().await?;
        let output = self
            .git_output(&[
                "status",
                "--porcelain=v2",
                "--branch",
                "--untracked-files=all",
                "-z",
            ])
            .await?;
        let files = parse_status(&output.stdout)
            .map_err(|error| GitError::InvalidOutput(error.to_string()))?;
        let mut counts = GitStatusCounts::default();
        for file in &files {
            counts.staged += usize::from(file.staged);
            counts.unstaged += usize::from(file.unstaged && file.kind != GitChangeKind::Untracked);
            counts.untracked += usize::from(file.kind == GitChangeKind::Untracked);
            counts.conflicted += usize::from(file.conflict);
        }

        Ok(GitStatusSnapshot {
            repo,
            files,
            counts,
            refreshed_at: Utc::now().to_rfc3339(),
        })
    }

    pub async fn history(
        &self,
        head: Option<&str>,
        skip: usize,
        limit: usize,
    ) -> GitResult<GitHistoryPage> {
        let repo = self.repo_info().await?;
        let head = head.map(str::to_string).or(repo.head);
        let Some(head) = head else {
            return Ok(GitHistoryPage {
                head: None,
                commits: Vec::new(),
                has_more: false,
            });
        };

        let limit = limit.clamp(1, MAX_HISTORY_LIMIT);
        let max_count = format!("--max-count={}", limit + 1);
        let skip_arg = format!("--skip={skip}");
        let args = [
            "log",
            "--date=iso-strict",
            "--decorate=short",
            "--format=%H%x00%h%x00%an%x00%ae%x00%aI%x00%D%x00%s%x1e",
            max_count.as_str(),
            skip_arg.as_str(),
            head.as_str(),
            "--",
        ];
        let output = self.git_output(&args).await?;
        let mut commits = parse_history(&output.stdout)
            .map_err(|error| GitError::InvalidOutput(error.to_string()))?;
        let has_more = commits.len() > limit;
        commits.truncate(limit);

        Ok(GitHistoryPage {
            head: Some(head),
            commits,
            has_more,
        })
    }

    pub async fn diff(&self, scope: GitDiffScope) -> GitResult<GitDiffSnapshot> {
        let repo = self.repo_info().await?;
        let mut patch = match &scope {
            GitDiffScope::Worktree => {
                let base = repo.head.as_deref().unwrap_or(EMPTY_TREE);
                self.diff_command(&[base, "--", "."]).await?
            }
            GitDiffScope::Staged => {
                let base = repo.head.as_deref().unwrap_or(EMPTY_TREE);
                self.git_output(&[
                    "diff",
                    "--cached",
                    "--no-ext-diff",
                    "--no-color",
                    "--no-renames",
                    "--unified=3",
                    base,
                    "--",
                    ".",
                ])
                .await?
                .stdout
            }
            GitDiffScope::Commit(commit) => {
                self.git_output(&[
                    "show",
                    "--no-ext-diff",
                    "--no-color",
                    "--format=",
                    "--no-renames",
                    "--unified=3",
                    commit,
                    "--",
                    ".",
                ])
                .await?
                .stdout
            }
        };

        let mut truncated = false;
        if patch.len() > MAX_DIFF_BYTES {
            patch.truncate(MAX_DIFF_BYTES);
            truncated = true;
        }

        if matches!(scope, GitDiffScope::Worktree) {
            let untracked = self
                .git_output(&[
                    "ls-files",
                    "--others",
                    "--exclude-standard",
                    "-z",
                    "--",
                    ".",
                ])
                .await?;
            for path in parse_nul_paths(&untracked.stdout)
                .map_err(|error| GitError::InvalidOutput(error.to_string()))?
            {
                let output = self
                    .git_output_allow_exit_1(&[
                        "diff",
                        "--no-index",
                        "--no-ext-diff",
                        "--no-color",
                        "--unified=3",
                        "--",
                        "/dev/null",
                        &path,
                    ])
                    .await?;
                if patch.len() + output.stdout.len() > MAX_DIFF_BYTES {
                    truncated = true;
                    break;
                }
                patch.extend_from_slice(&output.stdout);
            }
        }

        let files =
            parse_diff_files(&patch).map_err(|error| GitError::InvalidOutput(error.to_string()))?;
        Ok(GitDiffSnapshot {
            scope,
            files,
            patch: String::from_utf8_lossy(&patch).into_owned(),
            truncated,
        })
    }

    async fn diff_command(&self, range: &[&str]) -> GitResult<Vec<u8>> {
        Ok(self
            .git_output(&[
                "diff",
                "--no-ext-diff",
                "--no-color",
                "--no-renames",
                "--unified=3",
                range[0],
                range[1],
                range[2],
            ])
            .await?
            .stdout)
    }

    async fn git_text(&self, args: &[&str]) -> GitResult<String> {
        let output = self.git_output(args).await?;
        String::from_utf8(output.stdout).map_err(|error| GitError::InvalidOutput(error.to_string()))
    }

    async fn git_optional_text(&self, args: &[&str]) -> Option<String> {
        let output = Command::new("git")
            .current_dir(&self.workspace_root)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_PAGER", "cat")
            .args(args)
            .output()
            .await
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout)
            .ok()
            .map(|value| value.trim().to_string())
    }

    async fn git_output(&self, args: &[&str]) -> GitResult<std::process::Output> {
        if !self.workspace_root.is_dir() {
            return Err(GitError::WorkspaceMissing {
                path: self.workspace_root.clone(),
            });
        }
        let output = Command::new("git")
            .current_dir(&self.workspace_root)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_PAGER", "cat")
            .args(args)
            .output()
            .await
            .map_err(|error| map_git_io_error(error, &self.workspace_root))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if args == ["rev-parse", "--show-toplevel"] && output.status.code() == Some(128) {
                return Err(GitError::NotRepository {
                    path: self.workspace_root.clone(),
                });
            }
            return Err(GitError::CommandFailed {
                command: args.join(" "),
                message: stderr,
            });
        }
        Ok(output)
    }

    async fn git_output_allow_exit_1(&self, args: &[&str]) -> GitResult<std::process::Output> {
        if !self.workspace_root.is_dir() {
            return Err(GitError::WorkspaceMissing {
                path: self.workspace_root.clone(),
            });
        }
        let output = Command::new("git")
            .current_dir(&self.workspace_root)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_PAGER", "cat")
            .args(args)
            .output()
            .await
            .map_err(|error| map_git_io_error(error, &self.workspace_root))?;
        if !output.status.success() && output.status.code() != Some(1) {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(GitError::CommandFailed {
                command: args.join(" "),
                message: stderr,
            });
        }
        Ok(output)
    }
}

fn map_git_io_error(error: io::Error, workspace_root: &Path) -> GitError {
    if error.kind() == io::ErrorKind::NotFound {
        if !workspace_root.is_dir() {
            GitError::WorkspaceMissing {
                path: workspace_root.to_path_buf(),
            }
        } else {
            GitError::GitUnavailable
        }
    } else {
        GitError::CommandFailed {
            command: "<spawn>".to_string(),
            message: error.to_string(),
        }
    }
}

fn parse_status(bytes: &[u8]) -> Result<Vec<GitFileStatus>> {
    let fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut files = Vec::new();
    let mut iter = fields.map(|field| {
        String::from_utf8(field.to_vec()).context("Git returned a non-UTF-8 status path")
    });
    while let Some(field) = iter.next() {
        let field = field?;
        let Some(kind) = field.chars().next() else {
            continue;
        };
        match kind {
            '?' => files.push(GitFileStatus {
                path: field.get(2..).unwrap_or_default().to_string(),
                old_path: None,
                kind: GitChangeKind::Untracked,
                index_code: '?',
                worktree_code: '?',
                staged: false,
                unstaged: true,
                conflict: false,
            }),
            '!' => {}
            '1' | '2' | 'u' => {
                let expected_fields = match kind {
                    '1' => 9,
                    '2' => 10,
                    _ => 11,
                };
                let parts = field.splitn(expected_fields, ' ').collect::<Vec<_>>();
                if parts.len() != expected_fields {
                    bail!("invalid porcelain v2 status record: {field}");
                }
                let xy = parts.get(1).copied().unwrap_or("..");
                let index_code = xy.chars().next().unwrap_or('.');
                let worktree_code = xy.chars().nth(1).unwrap_or('.');
                let path = parts.last().copied().unwrap_or_default().to_string();
                let old_path = if kind == '2' {
                    iter.next().transpose()?.map(|value| value.to_string())
                } else {
                    None
                };
                let conflict = kind == 'u' || xy.contains('U');
                let change_kind = classify_change(kind, index_code, worktree_code, conflict);
                files.push(GitFileStatus {
                    path,
                    old_path,
                    kind: change_kind,
                    index_code,
                    worktree_code,
                    staged: index_code != '.' && index_code != '?',
                    unstaged: worktree_code != '.' && worktree_code != '?',
                    conflict,
                });
            }
            '#' => {}
            _ => bail!("unknown porcelain v2 status record: {field}"),
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn classify_change(record: char, index: char, worktree: char, conflict: bool) -> GitChangeKind {
    if conflict {
        GitChangeKind::Conflicted
    } else if record == '2' || index == 'R' || worktree == 'R' {
        GitChangeKind::Renamed
    } else if index == 'C' || worktree == 'C' {
        GitChangeKind::Copied
    } else if index == 'A' || worktree == 'A' {
        GitChangeKind::Added
    } else if index == 'D' || worktree == 'D' {
        GitChangeKind::Deleted
    } else if index == 'T' || worktree == 'T' {
        GitChangeKind::TypeChanged
    } else if index == 'M' || worktree == 'M' {
        GitChangeKind::Modified
    } else {
        GitChangeKind::Unknown
    }
}

fn parse_history(bytes: &[u8]) -> Result<Vec<GitCommitSummary>> {
    let text = String::from_utf8(bytes.to_vec()).context("Git returned non-UTF-8 history")?;
    let mut commits = Vec::new();
    for record in text
        .split('\u{1e}')
        .map(|record| record.trim_end_matches(['\r', '\n']))
        .filter(|record| !record.is_empty())
    {
        let fields = record.split('\0').collect::<Vec<_>>();
        if fields.len() < 7 {
            bail!("invalid Git history record");
        }
        let refs = fields[5]
            .split(", ")
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect();
        commits.push(GitCommitSummary {
            id: fields[0].to_string(),
            short_id: fields[1].to_string(),
            author: fields[2].to_string(),
            author_email: fields[3].to_string(),
            authored_at: fields[4].to_string(),
            refs,
            subject: fields[6].trim_end_matches('\n').to_string(),
        });
    }
    Ok(commits)
}

fn parse_nul_paths(bytes: &[u8]) -> Result<Vec<String>> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8(path.to_vec()).context("Git returned a non-UTF-8 path"))
        .collect()
}

fn parse_diff_files(bytes: &[u8]) -> Result<Vec<GitDiffFile>> {
    let patch = String::from_utf8_lossy(bytes);
    let mut files = Vec::new();
    for section in split_diff_sections(&patch) {
        let mut lines = section.lines();
        let Some(header) = lines.next() else {
            continue;
        };
        let path = header
            .strip_prefix("diff --git ")
            .and_then(|value| value.rsplit_once(" b/").map(|(_, path)| path.to_string()))
            .or_else(|| lines.find_map(|line| line.strip_prefix("+++ b/").map(str::to_string)))
            .unwrap_or_else(|| "(unknown)".to_string());
        let old_path = section
            .lines()
            .find_map(|line| line.strip_prefix("--- a/").map(str::to_string));
        let binary = section.contains("Binary files ") || section.contains("GIT binary patch");
        let additions = section
            .lines()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .count();
        let deletions = section
            .lines()
            .filter(|line| line.starts_with('-') && !line.starts_with("---"))
            .count();
        let kind = if section.contains("new file mode") {
            GitChangeKind::Added
        } else if section.contains("deleted file mode") {
            GitChangeKind::Deleted
        } else if section.contains("rename from") || section.contains("similarity index") {
            GitChangeKind::Renamed
        } else {
            GitChangeKind::Modified
        };
        files.push(GitDiffFile {
            path,
            old_path,
            kind,
            additions,
            deletions,
            binary,
            patch: section,
        });
    }
    Ok(files)
}

fn split_diff_sections(text: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        if line.starts_with("diff --git ") && !current.is_empty() {
            sections.push(std::mem::take(&mut current));
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        sections.push(current);
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2_status_records() {
        let input = b"# branch.head main\0# branch.ab +1 -2\x001 .M .x 100644 100644 100644 abc def src/lib.rs\0? new.txt\0u UU .x 100644 100644 100644 100644 abc def ghi conflict.rs\0";
        let files = parse_status(input).expect("status should parse");
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, "conflict.rs");
        assert!(files[0].conflict);
        assert_eq!(files[1].path, "new.txt");
        assert_eq!(files[2].worktree_code, 'M');
    }

    #[test]
    fn parses_history_records() {
        let input = b"abc\x00123\0Alice\0alice@example.com\x002026-08-18T12:00:00+08:00\0HEAD -> main, origin/main\0subject\x1e\n";
        let commits = parse_history(input).expect("history should parse");
        assert_eq!(commits[0].short_id, "123");
        assert_eq!(commits[0].refs, vec!["HEAD -> main", "origin/main"]);
    }

    #[test]
    fn parses_diff_sections_and_counts_lines() {
        let input = b"diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let files = parse_diff_files(input).expect("diff should parse");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/lib.rs");
        assert_eq!(files[0].additions, 1);
        assert_eq!(files[0].deletions, 1);
    }

    #[test]
    fn queries_a_real_repository() {
        let repo = TestRepository::new();
        run_git(&repo.path, &["init", "--quiet"]);
        run_git(&repo.path, &["config", "user.name", "Tidev Test"]);
        run_git(&repo.path, &["config", "user.email", "tidev@example.com"]);
        std::fs::write(repo.path.join("tracked.txt"), "before\n").expect("write tracked file");
        run_git(&repo.path, &["add", "tracked.txt"]);
        run_git(&repo.path, &["commit", "--quiet", "-m", "initial"]);

        std::fs::write(repo.path.join("tracked.txt"), "after\n").expect("modify tracked file");
        std::fs::write(repo.path.join("untracked.txt"), "new\n").expect("write untracked file");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build Tokio runtime");
        runtime.block_on(async {
            let service = GitService::new(repo.path.clone());
            let status = service.status().await.expect("query status");
            assert_eq!(status.counts.unstaged, 1);
            assert_eq!(status.counts.untracked, 1);
            assert!(status.files.iter().any(|file| file.path == "tracked.txt"));
            assert!(status.files.iter().any(|file| file.path == "untracked.txt"));

            let history = service.history(None, 0, 50).await.expect("query history");
            assert_eq!(history.commits.len(), 1);
            assert_eq!(history.commits[0].subject, "initial");

            let commit_diff = service
                .diff(GitDiffScope::Commit(history.commits[0].id.clone()))
                .await
                .expect("query commit diff");
            assert_eq!(commit_diff.files.len(), 1);
            assert_eq!(commit_diff.files[0].path, "tracked.txt");

            let worktree_diff = service
                .diff(GitDiffScope::Worktree)
                .await
                .expect("query worktree diff");
            assert_eq!(worktree_diff.files.len(), 2);
            assert!(worktree_diff.patch.contains("after"));
            assert!(worktree_diff.patch.contains("new"));

            run_git(&repo.path, &["add", "tracked.txt"]);
            let staged_diff = service
                .diff(GitDiffScope::Staged)
                .await
                .expect("query staged diff");
            assert_eq!(staged_diff.files.len(), 1);
            assert_eq!(staged_diff.files[0].path, "tracked.txt");
        });
    }

    #[test]
    fn reports_when_workspace_is_not_a_repository() {
        let workspace = TestRepository::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build Tokio runtime");

        runtime.block_on(async {
            let error = GitService::new(workspace.path.clone())
                .status()
                .await
                .expect_err("a plain directory is not a Git repository");
            assert_eq!(
                error,
                GitError::NotRepository {
                    path: workspace.path.clone(),
                }
            );
        });
    }

    #[test]
    fn reports_when_workspace_is_missing() {
        let workspace = std::env::temp_dir().join(format!(
            "tidev-git-missing-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build Tokio runtime");

        runtime.block_on(async {
            let error = GitService::new(workspace.clone())
                .status()
                .await
                .expect_err("a missing directory cannot be queried");
            assert_eq!(error, GitError::WorkspaceMissing { path: workspace });
        });
    }

    struct TestRepository {
        path: PathBuf,
    }

    impl TestRepository {
        fn new() -> Self {
            let suffix = Utc::now().timestamp_nanos_opt().unwrap_or_default();
            let path = std::env::temp_dir().join(format!("tidev-git-test-{suffix}"));
            std::fs::create_dir(&path).expect("create temporary repository directory");
            Self { path }
        }
    }

    impl Drop for TestRepository {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .expect("run git command");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
