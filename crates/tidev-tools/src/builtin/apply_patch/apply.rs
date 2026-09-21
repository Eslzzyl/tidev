//! Core application logic for codex-format patches.
//!
//! Ported from codex' s `lib.rs` — applies parsed hunks to the filesystem
//! using fuzzy seek‑and‑replace matching (no line numbers required).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::{error::Error, fmt};

use anyhow::{Context, Result, anyhow};
use diffy::DiffOptions;

use super::parser::{Hunk, UpdateFileChunk};
use super::seek_sequence::seek_sequence;
use crate::builtin::utils::{
    display_workspace_relative, read_existing_document, resolve_workspace_path,
};

/// Result of applying a patch — which files were added / modified / deleted.
#[derive(Debug, Default, Clone)]
pub struct ApplyPatchResult {
    /// Paths of newly created files.
    pub added: Vec<PathBuf>,
    /// Paths of existing files that were modified.
    pub modified: Vec<PathBuf>,
    /// Paths of deleted files.
    pub deleted: Vec<PathBuf>,
    /// Per‑file diffs for modified files (keyed by absolute path).
    pub diffs: HashMap<PathBuf, String>,
    /// Changes committed in patch order.
    pub changes: Vec<ApplyPatchChange>,
}

impl ApplyPatchResult {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// The kind of file operation represented by an applied patch hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyPatchOperation {
    Add,
    Update,
    Delete,
}

impl ApplyPatchOperation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Add => "Add File",
            Self::Update => "Update File",
            Self::Delete => "Delete File",
        }
    }

    pub fn summary_label(self) -> &'static str {
        match self {
            Self::Add => "A",
            Self::Update => "M",
            Self::Delete => "D",
        }
    }
}

/// A single file operation that was successfully committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPatchChange {
    pub path: PathBuf,
    pub operation: ApplyPatchOperation,
    pub diff: Option<String>,
}

/// The operation where a patch application stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPatchFailureLocation {
    /// One-based operation number in the parsed patch.
    pub operation_index: usize,
    pub path: PathBuf,
    pub operation: ApplyPatchOperation,
}

/// A failed patch application together with changes committed before failure.
#[derive(Debug)]
pub struct ApplyPatchFailure {
    message: String,
    partial_result: Box<ApplyPatchResult>,
    location: Option<ApplyPatchFailureLocation>,
}

impl ApplyPatchFailure {
    fn parse(message: String) -> Self {
        Self {
            message,
            partial_result: Box::new(ApplyPatchResult::default()),
            location: None,
        }
    }

    fn operation(
        message: String,
        partial_result: ApplyPatchResult,
        operation_index: usize,
        hunk: &Hunk,
    ) -> Self {
        let (path, operation) = match hunk {
            Hunk::AddFile { path, .. } => (path.clone(), ApplyPatchOperation::Add),
            Hunk::DeleteFile { path } => (path.clone(), ApplyPatchOperation::Delete),
            Hunk::UpdateFile { path, .. } => (path.clone(), ApplyPatchOperation::Update),
        };

        Self {
            message,
            partial_result: Box::new(partial_result),
            location: Some(ApplyPatchFailureLocation {
                operation_index: operation_index + 1,
                path,
                operation,
            }),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn partial_result(&self) -> &ApplyPatchResult {
        &self.partial_result
    }

    pub fn location(&self) -> Option<&ApplyPatchFailureLocation> {
        self.location.as_ref()
    }

    pub fn into_parts(self) -> (String, ApplyPatchResult, Option<ApplyPatchFailureLocation>) {
        (self.message, *self.partial_result, self.location)
    }
}

impl fmt::Display for ApplyPatchFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for ApplyPatchFailure {}

/// Apply a codex-format patch and retain the committed prefix when execution fails.
///
/// Parsing failures have an empty partial result. Execution failures stop at the
/// first failing operation and expose all changes committed before that point.
pub fn apply_patch(
    workspace_root: &Path,
    patch_text: &str,
    allow_outside: bool,
) -> Result<ApplyPatchResult, ApplyPatchFailure> {
    let parsed = super::parser::parse_patch(patch_text)
        .map_err(|e| ApplyPatchFailure::parse(format!("failed to parse patch: {e}")))?;

    if parsed.hunks.is_empty() {
        return Err(ApplyPatchFailure::parse(
            "patch contains no file operations".to_string(),
        ));
    }

    let mut result = ApplyPatchResult::default();

    for (operation_index, hunk) in parsed.hunks.iter().enumerate() {
        if let Err(error) = apply_hunk(workspace_root, hunk, allow_outside, &mut result) {
            return Err(ApplyPatchFailure::operation(
                format!("{error:#}"),
                result,
                operation_index,
                hunk,
            ));
        }
    }

    Ok(result)
}

fn apply_hunk(
    workspace_root: &Path,
    hunk: &Hunk,
    allow_outside: bool,
    result: &mut ApplyPatchResult,
) -> Result<()> {
    match hunk {
        Hunk::AddFile { path, contents } => {
            apply_add_file(workspace_root, path, contents, allow_outside, result)
        }
        Hunk::DeleteFile { path } => apply_delete_file(workspace_root, path, allow_outside, result),
        Hunk::UpdateFile {
            path,
            move_path,
            chunks,
        } => apply_update_file(
            workspace_root,
            path,
            move_path.as_deref(),
            chunks,
            allow_outside,
            result,
        ),
    }
}

fn apply_add_file(
    workspace_root: &Path,
    patch_path: &Path,
    contents: &str,
    allow_outside: bool,
    result: &mut ApplyPatchResult,
) -> Result<()> {
    let abs_path = resolve_workspace_path(workspace_root, patch_path, allow_outside)?;

    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create directory {}",
                display_workspace_relative(workspace_root, parent)
            )
        })?;
    }

    fs::write(&abs_path, contents).with_context(|| {
        format!(
            "failed to write {}",
            display_workspace_relative(workspace_root, &abs_path)
        )
    })?;

    result.added.push(abs_path.clone());
    result.changes.push(ApplyPatchChange {
        path: abs_path,
        operation: ApplyPatchOperation::Add,
        diff: None,
    });
    Ok(())
}

fn apply_delete_file(
    workspace_root: &Path,
    patch_path: &Path,
    allow_outside: bool,
    result: &mut ApplyPatchResult,
) -> Result<()> {
    let abs_path = resolve_workspace_path(workspace_root, patch_path, allow_outside)?;

    if abs_path.is_dir() {
        anyhow::bail!(
            "cannot delete directory {} via apply_patch",
            display_workspace_relative(workspace_root, &abs_path)
        );
    }

    if abs_path.exists() {
        fs::remove_file(&abs_path).with_context(|| {
            format!(
                "failed to delete {}",
                display_workspace_relative(workspace_root, &abs_path)
            )
        })?;
    }

    result.deleted.push(abs_path.clone());
    result.changes.push(ApplyPatchChange {
        path: abs_path,
        operation: ApplyPatchOperation::Delete,
        diff: None,
    });
    Ok(())
}

fn apply_update_file(
    workspace_root: &Path,
    patch_path: &Path,
    move_target: Option<&Path>,
    chunks: &[UpdateFileChunk],
    allow_outside: bool,
    result: &mut ApplyPatchResult,
) -> Result<()> {
    let abs_path = resolve_workspace_path(workspace_root, patch_path, allow_outside)?;

    // Check file existence early to give a clear error
    if !abs_path.exists() {
        anyhow::bail!(
            "File does not exist: {}",
            display_workspace_relative(workspace_root, &abs_path)
        );
    }

    let document = read_existing_document(&abs_path)
        .with_context(|| {
            format!(
                "failed to read {}",
                display_workspace_relative(workspace_root, &abs_path)
            )
        })?
        .ok_or_else(|| {
            anyhow!(
                "File does not exist: {}",
                display_workspace_relative(workspace_root, &abs_path)
            )
        })?;
    let old_content = document.text();

    // Compute new content from chunks
    let new_content = derive_new_contents(workspace_root, &abs_path, old_content, chunks)?;

    // Determine the final path (handle MoveTo)
    let final_path = if let Some(move_target) = move_target {
        let dest = resolve_workspace_path(workspace_root, move_target, allow_outside)?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create directory {}",
                    display_workspace_relative(workspace_root, parent)
                )
            })?;
        }
        // Write to destination
        let encoded = document.encode_updated(&new_content).with_context(|| {
            format!(
                "failed to encode {}",
                display_workspace_relative(workspace_root, &dest)
            )
        })?;
        fs::write(&dest, encoded).with_context(|| {
            format!(
                "failed to write {}",
                display_workspace_relative(workspace_root, &dest)
            )
        })?;
        // Remove original. The destination write is already committed if this
        // removal fails, so retain it in the partial result before returning.
        if abs_path.exists()
            && abs_path != dest
            && let Err(error) = fs::remove_file(&abs_path).with_context(|| {
                format!(
                    "failed to remove original {}",
                    display_workspace_relative(workspace_root, &abs_path)
                )
            })
        {
            let diff = generate_diff(old_content, &new_content, &dest, workspace_root);
            record_modified_change(result, &dest, diff);
            return Err(error);
        }
        dest
    } else {
        // Write to original path
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create directory {}",
                    display_workspace_relative(workspace_root, parent)
                )
            })?;
        }
        let encoded = document.encode_updated(&new_content).with_context(|| {
            format!(
                "failed to encode {}",
                display_workspace_relative(workspace_root, &abs_path)
            )
        })?;
        fs::write(&abs_path, encoded).with_context(|| {
            format!(
                "failed to write {}",
                display_workspace_relative(workspace_root, &abs_path)
            )
        })?;
        abs_path.clone()
    };

    // Generate a unified diff for the output
    let diff = generate_diff(old_content, &new_content, &final_path, workspace_root);

    record_modified_change(result, &final_path, diff);

    Ok(())
}

fn record_modified_change(result: &mut ApplyPatchResult, path: &Path, diff: Option<String>) {
    if let Some(diff) = &diff {
        result.diffs.insert(path.to_path_buf(), diff.clone());
    }
    result.changes.push(ApplyPatchChange {
        path: path.to_path_buf(),
        operation: ApplyPatchOperation::Update,
        diff,
    });
    result.modified.push(path.to_path_buf());
}

/// Apply chunks to file content using seek‑and‑replace (ported from codex).
fn derive_new_contents(
    workspace_root: &Path,
    path: &Path,
    old_content: &str,
    chunks: &[UpdateFileChunk],
) -> Result<String> {
    let mut original_lines: Vec<String> = old_content.split('\n').map(String::from).collect();

    // Drop trailing empty element from final newline (like codex does)
    if original_lines.last().is_some_and(String::is_empty) {
        original_lines.pop();
    }

    let replacements = compute_replacements(&original_lines, workspace_root, path, chunks)
        .map_err(|e| anyhow!("{e}"))?;

    let new_lines = apply_replacements(original_lines, &replacements);

    // Re‑join with newlines, ensuring trailing newline
    let mut new_lines = new_lines;
    if !new_lines.last().is_some_and(String::is_empty) {
        new_lines.push(String::new());
    }
    Ok(new_lines.join("\n"))
}

/// Compute a list of replacements needed to transform `original_lines` into
/// the new lines. Each replacement is `(start_index, old_len, new_lines)`.
fn compute_replacements(
    original_lines: &[String],
    workspace_root: &Path,
    path: &Path,
    chunks: &[UpdateFileChunk],
) -> std::result::Result<Vec<(usize, usize, Vec<String>)>, String> {
    let mut replacements: Vec<(usize, usize, Vec<String>)> = Vec::new();
    let mut line_index: usize = 0;

    for chunk in chunks {
        // If chunk has a change_context, seek to it first
        if let Some(ctx_line) = &chunk.change_context {
            if let Some(idx) = seek_sequence(
                original_lines,
                std::slice::from_ref(ctx_line),
                line_index,
                false,
            ) {
                line_index = idx + 1;
            } else {
                return Err(format!(
                    "Failed to find context at position {} in {}. \
                     The file may have changed since the patch was generated.",
                    line_index,
                    display_workspace_relative(workspace_root, path),
                ));
            }
        }

        if chunk.old_lines.is_empty() {
            // Pure addition — add at end
            let insertion_idx = if original_lines.last().is_some_and(String::is_empty) {
                original_lines.len() - 1
            } else {
                original_lines.len()
            };
            replacements.push((insertion_idx, 0, chunk.new_lines.clone()));
            continue;
        }

        // Try to match old_lines in the file
        let mut pattern: &[String] = &chunk.old_lines;
        let mut found = seek_sequence(original_lines, pattern, line_index, chunk.is_end_of_file);

        let mut new_slice: &[String] = &chunk.new_lines;

        if found.is_none() && pattern.last().is_some_and(String::is_empty) {
            // Retry without trailing empty line (represents final newline)
            pattern = &pattern[..pattern.len() - 1];
            if new_slice.last().is_some_and(String::is_empty) {
                new_slice = &new_slice[..new_slice.len() - 1];
            }
            found = seek_sequence(original_lines, pattern, line_index, chunk.is_end_of_file);
        }

        if let Some(start_idx) = found {
            replacements.push((start_idx, pattern.len(), new_slice.to_vec()));
            line_index = start_idx + pattern.len();
        } else {
            return Err(format!(
                "Failed to find text to replace ({} lines, starting at position {}) in {}. \
                 The file may have changed since the patch was generated.",
                chunk.old_lines.len(),
                line_index,
                display_workspace_relative(workspace_root, path),
            ));
        }
    }

    // Sort by position (should already be in order, but be safe)
    replacements.sort_by_key(|(a, _, _)| *a);
    Ok(replacements)
}

/// Apply sorted, non-overlapping replacements in one linear pass.
fn apply_replacements(
    lines: Vec<String>,
    replacements: &[(usize, usize, Vec<String>)],
) -> Vec<String> {
    // Replacements are sorted by their original positions. Rebuild the output
    // once so unchanged lines are moved instead of repeatedly shifted by
    // Vec::remove and Vec::insert.
    let additional_capacity: usize = replacements
        .iter()
        .map(|(_, old_len, new_segment)| new_segment.len().saturating_sub(*old_len))
        .sum();
    let mut result = Vec::with_capacity(lines.len() + additional_capacity);
    let mut remaining = lines.into_iter();
    let mut cursor = 0;

    for (start_idx, old_len, new_segment) in replacements {
        while cursor < *start_idx {
            if let Some(line) = remaining.next() {
                result.push(line);
            }
            cursor += 1;
        }

        for _ in 0..*old_len {
            remaining.next();
            cursor += 1;
        }

        result.extend(new_segment.iter().cloned());
    }

    result.extend(remaining);
    result
}

/// Generate a unified diff string for the change (using diffy, already a dep).
fn generate_diff(
    old_content: &str,
    new_content: &str,
    abs_path: &Path,
    workspace_root: &Path,
) -> Option<String> {
    let relative = abs_path
        .strip_prefix(workspace_root)
        .unwrap_or(abs_path)
        .to_string_lossy();

    let mut options = DiffOptions::new();
    options.set_context_len(3);
    options.set_original_filename(format!("a/{relative}"));
    options.set_modified_filename(format!("b/{relative}"));

    let patch = options.create_patch(old_content, new_content);
    if patch.hunks().is_empty() {
        return None;
    }
    Some(patch.to_string())
}

#[cfg(test)]
mod tests {
    use super::{apply_patch, apply_replacements, derive_new_contents};
    use crate::builtin::apply_patch::UpdateFileChunk;

    #[test]
    fn apply_replacements_preserves_original_positions() {
        let lines = vec!["a", "b", "c", "d"]
            .into_iter()
            .map(String::from)
            .collect();
        let replacements = vec![
            (1, 1, vec![String::from("B")]),
            (3, 1, vec![String::from("D1"), String::from("D2")]),
        ];

        assert_eq!(
            apply_replacements(lines, &replacements),
            vec!["a", "B", "c", "D1", "D2"]
        );
    }

    #[test]
    fn apply_replacements_preserves_order_of_same_position_insertions() {
        let lines = vec![String::from("a")];
        let replacements = vec![
            (1, 0, vec![String::from("b")]),
            (1, 0, vec![String::from("c")]),
        ];

        assert_eq!(
            apply_replacements(lines, &replacements),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn derive_new_contents_applies_multiple_chunks() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("file.txt");
        let chunks = vec![
            UpdateFileChunk {
                change_context: Some(String::from("a")),
                old_lines: vec![String::from("b")],
                new_lines: vec![String::from("B")],
                is_end_of_file: false,
            },
            UpdateFileChunk {
                change_context: Some(String::from("c")),
                old_lines: vec![String::from("d")],
                new_lines: vec![String::from("D1"), String::from("D2")],
                is_end_of_file: false,
            },
        ];

        let result = derive_new_contents(workspace.path(), &path, "a\nb\nc\nd\n", &chunks).unwrap();

        assert_eq!(result, "a\nB\nc\nD1\nD2\n");
    }

    #[test]
    fn failure_exposes_committed_prefix_and_location() {
        let workspace = tempfile::tempdir().unwrap();
        let first = workspace.path().join("first.txt");
        std::fs::write(&first, "old\n").unwrap();

        let patch = "*** Begin Patch
*** Update File: first.txt
@@
-old
+new
*** Update File: missing.txt
@@
-old
+new
*** End Patch";

        let failure = apply_patch(workspace.path(), patch, false).expect_err("patch should fail");

        assert_eq!(std::fs::read_to_string(&first).unwrap(), "new\n");
        assert_eq!(failure.partial_result().changes.len(), 1);
        assert_eq!(failure.partial_result().changes[0].path, first);
        assert_eq!(
            failure.location(),
            Some(&super::ApplyPatchFailureLocation {
                operation_index: 2,
                path: "missing.txt".into(),
                operation: super::ApplyPatchOperation::Update,
            })
        );
        assert!(failure.message().contains("File does not exist"));
    }
}
