//! Undo/redo helpers — message navigation and patch collection.
//!
//! These are pure functions that operate on message lists. The orchestration
//! (snapshot capture, persistence, event emission) lives in [`Runtime`](crate::Runtime).

use tidev_snapshot::Patch;
use tidev_types::message::{Message, MessageRole, COMPACTION_MESSAGE_LABEL};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Message navigation
// ---------------------------------------------------------------------------

/// Find the last non-streaming User message that isn't a compaction marker.
pub fn last_visible_user_message(messages: &[Message]) -> Option<Uuid> {
    messages
        .iter()
        .rev()
        .find(|m| {
            m.role == MessageRole::User && !m.streaming && m.content != COMPACTION_MESSAGE_LABEL
        })
        .map(|m| m.id)
}

/// Find the first visible User message **before** `before_id` (going backwards).
pub fn prev_user_message_before(messages: &[Message], before_id: Uuid) -> Option<Uuid> {
    let mut found = false;
    for m in messages.iter().rev() {
        if found {
            if m.role == MessageRole::User && !m.streaming && m.content != COMPACTION_MESSAGE_LABEL {
                return Some(m.id);
            }
        }
        if m.id == before_id {
            found = true;
        }
    }
    None
}

/// Find the first visible User message **after** `after_id` (going forwards).
pub fn next_user_message_after(messages: &[Message], after_id: Uuid) -> Option<Uuid> {
    let mut found = false;
    for m in messages {
        if found {
            if m.role == MessageRole::User && !m.streaming && m.content != COMPACTION_MESSAGE_LABEL {
                return Some(m.id);
            }
        }
        if m.id == after_id {
            found = true;
        }
    }
    None
}

/// Restore [`ContextManager`](crate::ContextManager) state from a compaction
/// message's stored prior state, if the target message is within a compacted
/// range.
///
/// Returns `true` if state was restored.
pub fn restore_context_from_compaction(
    messages: &[Message],
    target_id: Uuid,
    summary: &mut Option<String>,
    retained_from: &mut usize,
) -> bool {
    // Walk messages **before or at** target_id. Find the *last* compaction
    // message whose prior state we should restore.
    let mut found_compaction = false;
    let mut prior_summary: Option<String> = None;
    let mut prior_retained_from: usize = 0;

    for m in messages {
        if m.id == target_id {
            break; // stop once we pass the target
        }
        if m.role == MessageRole::User
            && m.content == COMPACTION_MESSAGE_LABEL
            && !m.streaming
        {
            // This compaction message's metadata stores the state *before*
            // the compaction was applied.
            if let Some((s, r)) = extract_compaction_prior_state(m) {
                prior_summary = s;
                prior_retained_from = r;
                found_compaction = true;
            }
        }
    }

    if found_compaction {
        *summary = prior_summary;
        *retained_from = prior_retained_from;
        true
    } else {
        false
    }
}

/// Extract prior context state stored on a compaction message's metadata.
fn extract_compaction_prior_state(message: &Message) -> Option<(Option<String>, usize)> {
    let prior_summary = message.metadata.prior_summary.clone();
    let prior_retained_from = message.metadata.prior_retained_from?;
    Some((prior_summary, prior_retained_from))
}
// ---------------------------------------------------------------------------
// Patch collection
// ---------------------------------------------------------------------------

/// Collect all patches that need to be reverted to restore the workspace to
/// the state at (or just before) `target_id`.
///
/// Patches are returned in **application order** (oldest first), suitable for
/// [`SnapshotService::revert`](tidev_snapshot::SnapshotService::revert).
pub fn collect_patches_after_message(messages: &[Message], target_id: Uuid) -> Vec<Patch> {
    let mut patches = Vec::new();
    let mut found = false;

    for m in messages {
        if found {
            accumulate_patches(&mut patches, m);
            continue;
        }
        if m.id == target_id {
            found = true;
            accumulate_patches(&mut patches, m);
        }
    }

    // Reverse so oldest patches are first (revert applies oldest-first).
    patches.reverse();
    patches
}

/// Accumulate patches from a single message into `patches`, inserting each at
/// the front so that newest-step-first order is maintained within one message.
fn accumulate_patches(patches: &mut Vec<Patch>, message: &Message) {
    let extracted = extract_patches_from_message(message);
    if extracted.is_empty() {
        return;
    }
    // Insert at front (reversed step order within a message).
    for p in extracted.into_iter().rev() {
        patches.insert(0, p);
    }
}

/// Decode the patches stored inside a message's `patch_files` field.
///
/// Supports both the current StepPatch format and a legacy flat file-list format.
fn extract_patches_from_message(message: &Message) -> Vec<Patch> {
    let Some(patch_files_str) = &message.patch_files else {
        return Vec::new();
    };

    let arr: Vec<serde_json::Value> = match serde_json::from_str(patch_files_str) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    // If the first element is an object → StepPatch format.
    if arr.first().and_then(|v| v.as_object()).is_some() {
        return arr
            .into_iter()
            .filter_map(|v| {
                let hash = v.get("hash")?.as_str()?.to_string();
                let files = v
                    .get("files")?
                    .as_array()?
                    .iter()
                    .filter_map(|f| f.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>();
                Some(Patch { hash, files })
            })
            .collect();
    }

    // Fallback: flat format ["file1", "file2"] — use message's snapshot_hash.
    let files: Vec<String> = arr
        .into_iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();

    if !files.is_empty() {
        if let Some(hash) = &message.snapshot_hash {
            return vec![Patch {
                hash: hash.clone(),
                files,
            }];
        }
    }

    Vec::new()
}
