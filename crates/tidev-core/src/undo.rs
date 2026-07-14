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
/// Walks messages **after** `target_id` and returns the state that existed
/// **before** the first compaction found. This restores the context to what
/// it was at the point of `target_id`.
///
/// Returns `true` if state was restored.
pub fn restore_context_from_compaction(
    messages: &[Message],
    target_id: Uuid,
    summary: &mut Option<String>,
    retained_from: &mut usize,
) -> bool {
    let mut found_target = false;
    for m in messages {
        if m.id == target_id {
            found_target = true;
            continue;
        }
        if !found_target {
            continue;
        }
        // Compaction markers store the state *before* that compaction.
        // The first compaction marker after target_id tells us what state
        // was current when target_id was active.
        if m.content.starts_with(COMPACTION_MESSAGE_LABEL) {
            if let Some((s, r)) = extract_compaction_prior_state(m) {
                *summary = s;
                *retained_from = r;
                return true;
            }
        }
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;

    fn msg_user(id: Uuid) -> Message {
        let mut m = Message::new(MessageRole::User, "test");
        m.id = id;
        m
    }

    fn msg_assistant(id: Uuid) -> Message {
        let mut m = Message::new(MessageRole::Assistant, "response");
        m.id = id;
        m
    }

    fn msg_compaction(summary: &str) -> Message {
        let mut m = Message::compaction(summary);
        m.metadata.prior_summary = Some("old".into());
        m.metadata.prior_retained_from = Some(42);
        m
    }

    fn msg_compaction_with(summary: &str, prior_summary: Option<&str>, prior_retained: usize) -> Message {
        let mut m = Message::compaction(summary);
        m.metadata.prior_summary = prior_summary.map(|s| s.to_string());
        m.metadata.prior_retained_from = Some(prior_retained);
        m
    }

    fn msg_with_patches(id: Uuid, hash: &str, files: &[&str]) -> Message {
        let patch: Vec<serde_json::Value> = vec![serde_json::json!({
            "hash": hash,
            "files": files,
            "step": 1,
        })];
        let json = serde_json::to_string(&patch).unwrap();
        let mut m = msg_assistant(id);
        m.patch_files = Some(json);
        m.snapshot_hash = Some(hash.to_string());
        m
    }

    fn msg_with_flat_patches(id: Uuid, hash: &str, files: &[&str]) -> Message {
        let json = serde_json::to_string(files).unwrap();
        let mut m = msg_assistant(id);
        m.patch_files = Some(json);
        m.snapshot_hash = Some(hash.to_string());
        m
    }

    // ── Message navigation ────────────────────────────────────────

    #[test]
    fn last_visible_user_message_finds_last_user() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let msgs = vec![msg_user(id1), msg_assistant(Uuid::new_v4()), msg_user(id2)];
        assert_eq!(last_visible_user_message(&msgs), Some(id2));
    }

    #[test]
    fn last_visible_user_message_skips_streaming() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let mut streaming = msg_user(id2);
        streaming.streaming = true;
        let msgs = vec![msg_user(id1), streaming];
        assert_eq!(last_visible_user_message(&msgs), Some(id1));
    }

    #[test]
    fn last_visible_user_message_skips_compaction() {
        let id1 = Uuid::new_v4();
        let mut compact = Message::new(MessageRole::User, COMPACTION_MESSAGE_LABEL.to_string());
        compact.id = id1;
        let msgs = vec![compact, msg_user(Uuid::new_v4())];
        let result = last_visible_user_message(&msgs);
        assert_ne!(result, Some(id1));
    }

    #[test]
    fn prev_user_message_before_finds_previous() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();
        let msgs = vec![msg_user(id1), msg_assistant(id2), msg_user(id3)];
        assert_eq!(prev_user_message_before(&msgs, id3), Some(id1));
    }

    #[test]
    fn next_user_message_after_finds_next() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let msgs = vec![msg_user(id1), msg_assistant(Uuid::new_v4()), msg_user(id2)];
        assert_eq!(next_user_message_after(&msgs, id1), Some(id2));
    }

    // ── Compaction prior state ────────────────────────────────────

    #[test]
    fn restore_context_returns_false_when_no_compaction() {
        let msgs = vec![msg_user(Uuid::new_v4())];
        let mut summary = None;
        let mut retained = 0;
        assert!(!restore_context_from_compaction(&msgs, Uuid::new_v4(), &mut summary, &mut retained));
    }

    #[test]
    fn restore_context_returns_state_from_first_compaction_after_target() {
        let target_id = Uuid::new_v4();
        let compaction = msg_compaction("summary after compact");
        let msgs = vec![
            msg_user(target_id),
            compaction,
        ];
        let mut summary = None;
        let mut retained = 0;
        let found = restore_context_from_compaction(&msgs, target_id, &mut summary, &mut retained);
        assert!(found);
        assert_eq!(summary.as_deref(), Some("old"));
        assert_eq!(retained, 42);
    }

    #[test]
    fn restore_context_skips_compaction_before_target() {
        let target_id = Uuid::new_v4();
        let early = msg_compaction("early");
        let after = msg_compaction("after");
        let msgs = vec![early, msg_user(target_id), after];
        let mut summary = None;
        let mut retained = 0;
        let found = restore_context_from_compaction(&msgs, target_id, &mut summary, &mut retained);
        assert!(found);
        // Should use the one AFTER target, not the one before
        assert_eq!(retained, 42);
    }

    #[test]
    fn restore_context_finds_first_compaction_after_target() {
        let target_id = Uuid::new_v4();
        // Multiple compactions after target — should return the first one's prior state
        let c1 = msg_compaction_with("first", Some("summary-a"), 5);
        let c2 = msg_compaction_with("second", Some("summary-b"), 10);
        let msgs = vec![
            msg_user(target_id),
            msg_assistant(Uuid::new_v4()),
            c1,
            c2,
        ];
        let mut summary = None;
        let mut retained = 0;
        let found = restore_context_from_compaction(&msgs, target_id, &mut summary, &mut retained);
        assert!(found);
        assert_eq!(summary.as_deref(), Some("summary-a"));
        assert_eq!(retained, 5);
    }

    #[test]
    fn restore_context_skips_compaction_without_prior_metadata() {
        let target_id = Uuid::new_v4();
        let no_prior = Message::compaction("no prior");
        // Don't set metadata.prior_retained_from — not extractable
        let with_prior = msg_compaction_with("has prior", Some("summary"), 99);
        let msgs = vec![msg_user(target_id), no_prior, with_prior];
        let mut summary = None;
        let mut retained = 0;
        let found = restore_context_from_compaction(&msgs, target_id, &mut summary, &mut retained);
        // Should skip the one without prior_retained_from, find the next one
        assert!(found);
        assert_eq!(retained, 99);
        assert_eq!(summary.as_deref(), Some("summary"));
    }

    // ── Patch extraction ──────────────────────────────────────────

    #[test]
    fn extract_empty_when_no_patch_files() {
        let msg = msg_user(Uuid::new_v4());
        assert!(extract_patches_from_message(&msg).is_empty());
    }

    #[test]
    fn extract_nested_format() {
        let msg = msg_with_patches(Uuid::new_v4(), "abc", &["f1.txt"]);
        let patches = extract_patches_from_message(&msg);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].hash, "abc");
        assert_eq!(patches[0].files, vec!["f1.txt"]);
    }

    #[test]
    fn extract_flat_format() {
        let msg = msg_with_flat_patches(Uuid::new_v4(), "abc", &["f1.txt", "f2.txt"]);
        let patches = extract_patches_from_message(&msg);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].hash, "abc");
        assert_eq!(patches[0].files, vec!["f1.txt", "f2.txt"]);
    }

    #[test]
    fn collect_after_message_orders_correctly() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();
        let msgs = vec![
            msg_with_patches(id1, "h1", &["f1"]),
            msg_with_patches(id2, "h2", &["f2"]),
            msg_with_patches(id3, "h3", &["f3"]),
        ];
        let patches = collect_patches_after_message(&msgs, id1);
        assert_eq!(patches.len(), 3);
        assert_eq!(patches[0].hash, "h1");
        assert_eq!(patches[1].hash, "h2");
        assert_eq!(patches[2].hash, "h3");
    }
}
