//! Core application logic for codex-format patches.
//!
//! Ported from codex' s `lib.rs` — applies parsed hunks to the filesystem
//! using fuzzy seek‑and‑replace matching (no line numbers required).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use diffy::DiffOptions;

use super::parser::{Hunk, UpdateFileChunk};
use super::seek_sequence::seek_sequence;
use crate::tooling::builtin::utils::{read_existing_text, resolve_workspace_path};

/// Result of applying a patch — which files were added / modified / deleted.
#[derive(Debug, Default)]
pub struct ApplyPatchResult {
    /// Paths of newly created files.
    pub added: Vec<PathBuf>,
    /// Paths of existing files that were modified.
    pub modified: Vec<PathBuf>,
    /// Paths of deleted files.
    pub deleted: Vec<PathBuf>,
    /// Per‑file diffs for modified files (keyed by absolute path).
    pub diffs: HashMap<PathBuf, String>,
}

/// Apply a codex‑format patch to the workspace.
///
/// `workspace_root` is used to resolve relative paths from the patch.
/// `patch_text` is the raw patch body in codex `***` format.
/// `allow_outside` controls whether paths outside the workspace are permitted.
pub fn apply_patch(
    workspace_root: &Path,
    patch_text: &str,
    allow_outside: bool,
) -> Result<ApplyPatchResult> {
    let parsed = super::parser::parse_patch(patch_text)
        .map_err(|e| anyhow!("failed to parse patch: {e}"))?;

    if parsed.hunks.is_empty() {
        anyhow::bail!("patch contains no file operations");
    }

    let mut result = ApplyPatchResult::default();

    for hunk in &parsed.hunks {
        apply_hunk(workspace_root, hunk, allow_outside, &mut result)?;
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
        Hunk::DeleteFile { path } => {
            apply_delete_file(workspace_root, path, allow_outside, result)
        }
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
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    fs::write(&abs_path, contents)
        .with_context(|| format!("failed to write {}", abs_path.display()))?;

    result.added.push(abs_path);
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
        anyhow::bail!("cannot delete directory {} via apply_patch", abs_path.display());
    }

    if abs_path.exists() {
        fs::remove_file(&abs_path)
            .with_context(|| format!("failed to delete {}", abs_path.display()))?;
    }

    result.deleted.push(abs_path);
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
    let old_content = read_existing_text(&abs_path)?;

    // Compute new content from chunks
    let new_content = derive_new_contents(&abs_path, &old_content, chunks)?;

    // Determine the final path (handle MoveTo)
    let final_path = if let Some(move_target) = move_target {
        let dest = resolve_workspace_path(workspace_root, move_target, allow_outside)?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        // Write to destination
        fs::write(&dest, &new_content)
            .with_context(|| format!("failed to write {}", dest.display()))?;
        // Remove original
        if abs_path.exists() && abs_path != dest {
            fs::remove_file(&abs_path)
                .with_context(|| format!("failed to remove original {}", abs_path.display()))?;
        }
        dest
    } else {
        // Write to original path
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        fs::write(&abs_path, &new_content)
            .with_context(|| format!("failed to write {}", abs_path.display()))?;
        abs_path.clone()
    };

    // Generate a unified diff for the output
    let diff = generate_diff(&old_content, &new_content, &final_path, workspace_root);

    if let Some(diff) = diff {
        result.diffs.insert(final_path.clone(), diff);
    }
    result.modified.push(final_path);

    Ok(())
}

/// Apply chunks to file content using seek‑and‑replace (ported from codex).
fn derive_new_contents(
    path: &Path,
    old_content: &str,
    chunks: &[UpdateFileChunk],
) -> Result<String> {
    let mut original_lines: Vec<String> =
        old_content.split('\n').map(String::from).collect();

    // Drop trailing empty element from final newline (like codex does)
    if original_lines.last().is_some_and(String::is_empty) {
        original_lines.pop();
    }

    let replacements = compute_replacements(&original_lines, path, chunks)
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
                    "Failed to find context '{}' in {}",
                    ctx_line,
                    path.display()
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
        let mut found = seek_sequence(
            original_lines,
            pattern,
            line_index,
            chunk.is_end_of_file,
        );

        let mut new_slice: &[String] = &chunk.new_lines;

        if found.is_none() && pattern.last().is_some_and(String::is_empty) {
            // Retry without trailing empty line (represents final newline)
            pattern = &pattern[..pattern.len() - 1];
            if new_slice.last().is_some_and(String::is_empty) {
                new_slice = &new_slice[..new_slice.len() - 1];
            }
            found = seek_sequence(
                original_lines,
                pattern,
                line_index,
                chunk.is_end_of_file,
            );
        }

        if let Some(start_idx) = found {
            replacements.push((start_idx, pattern.len(), new_slice.to_vec()));
            line_index = start_idx + pattern.len();
        } else {
            return Err(format!(
                "Failed to find expected lines in {}:\n{}",
                path.display(),
                chunk.old_lines.join("\n"),
            ));
        }
    }

    replacements.sort_by(|(a, _, _), (b, _, _)| a.cmp(b));
    Ok(replacements)
}

/// Apply replacements in reverse order so indices stay valid.
fn apply_replacements(
    mut lines: Vec<String>,
    replacements: &[(usize, usize, Vec<String>)],
) -> Vec<String> {
    for (start_idx, old_len, new_segment) in replacements.iter().rev() {
        let start_idx = *start_idx;
        let old_len = *old_len;

        for _ in 0..old_len {
            if start_idx < lines.len() {
                lines.remove(start_idx);
            }
        }

        for (offset, new_line) in new_segment.iter().enumerate() {
            lines.insert(start_idx + offset, new_line.clone());
        }
    }

    lines
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
