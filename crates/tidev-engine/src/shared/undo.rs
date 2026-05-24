//! Shared undo/redo primitives used by all frontends.
//!
//! Extracting these from the TUI and web frontends avoids code duplication
//! and makes it straightforward to add undo support to gateway frontends
//! (Telegram, QQ, …).
//!
//! # What belongs here
//!
//! - [`StepPatch`] — serialised per-step snapshot metadata stored in
//!   `message.patch_files`.
//! - [`extract_patches_from_message`] — decode patch data from a single
//!   [`Message`].
//! - [`collect_patches_from_message`] — accumulate patches from one message
//!   into an existing vec.
//! - [`collect_patches_after_message`] — gather all patches that need to be
//!   reverted to restore the state at (or before) a given message.
//!
//! # What stays in the frontend
//!
//! - **TUI** (`src/tui/core/undo.rs`) — user-facing undo/redo/revert navigation,
//!   step-tracking fields (`step_snapshot_hashes` etc.), async diff for the
//!   sidebar, and [`SnapshotService`] / [`SessionStore`] wiring.
//! - **Web** (`src/web/routes/messages.rs`) — HTTP handlers, SSE publishing,
//!   and the remaining file-operation glue.
//!
//! Eventually the file-level revert/redo operations themselves
//! (`SnapshotService::revert` / `SnapshotService::restore` / `SessionStore`
//!  persistence) *could* be wrapped in a shared [`UndoService`] struct, but
//!  for now each frontend still composes them inline because their async /
//!  sync boundaries differ.

use anyhow::Result;
use uuid::Uuid;

use tidev_session::session::Message;
use crate::snapshot::Patch;

/// A single step-level patch stored within a round.
///
/// Multiple step patches are serialised as a JSON array in
/// `message.patch_files`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct StepPatch {
    pub hash: String,
    pub files: Vec<String>,
    pub step: usize,
}

/// Decode the patches stored inside a single message's `patch_files` field.
///
/// Supports both the current nested format
/// (`[{"hash":…,"files":[…],"step":N}]`) and the legacy flat format
/// (`["file1","file2"]`).
pub fn extract_patches_from_message(message: &Message) -> Vec<Patch> {
    let Some(patch_files_str) = &message.patch_files else {
        return Vec::new();
    };

    // Try nested format first.
    if let Ok(step_patches) = serde_json::from_str::<Vec<StepPatch>>(patch_files_str) {
        return step_patches
            .into_iter()
            .map(|sp| Patch {
                hash: sp.hash,
                files: sp.files,
            })
            .collect();
    }

    // Fallback: old flat format `["file1","file2"]` – use the message's
    // snapshot_hash.
    if let Ok(files) = serde_json::from_str::<Vec<String>>(patch_files_str)
        && !files.is_empty()
        && let Some(hash) = &message.snapshot_hash
    {
        return vec![Patch {
            hash: hash.clone(),
            files,
        }];
    }

    Vec::new()
}

/// Collect patches from a single message, inserting them at the **front** of
/// `patches` (newest step first, oldest step last).
///
/// The caller is responsible for reversing the final list so that oldest
/// patches are applied first during revert.
pub fn collect_patches_from_message(mut patches: Vec<Patch>, message: &Message) -> Vec<Patch> {
    let msg_patches = extract_patches_from_message(message);
    if msg_patches.is_empty() {
        return patches;
    }

    for msg_patch in msg_patches.into_iter().rev() {
        patches.insert(0, msg_patch);
    }
    patches
}

/// Collect all patches that need to be reverted to restore the workspace to
/// the state it was at (or just before) `message_id`.
///
/// *Patches are returned in application order* — oldest (closest to the
/// target message) first — so they can be fed directly to
/// [`SnapshotService::revert`](crate::snapshot::SnapshotService::revert).
pub fn collect_patches_after_message(messages: &[Message], message_id: Uuid) -> Result<Vec<Patch>> {
    let mut patches = Vec::new();
    let mut found = false;

    for message in messages {
        if found {
            patches = collect_patches_from_message(patches, message);
            continue;
        }

        if message.id == message_id {
            found = true;
            // Also include the target message's own patches.
            patches = collect_patches_from_message(patches, message);
        }
    }

    // Reverse so the OLDEST patches (closest to target message) are
    // processed first.  This ensures the initial snapshot hash takes
    // priority in revert dedup.
    patches.reverse();

    Ok(patches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidev_session::session::{Message, MessageRole};

    fn make_message(id: Uuid, snapshot_hash: Option<&str>, patch_files: Option<&str>) -> Message {
        let mut msg = Message::new(MessageRole::User, "test");
        msg.id = id;
        msg.snapshot_hash = snapshot_hash.map(|s| s.to_string());
        msg.patch_files = patch_files.map(|s| s.to_string());
        msg
    }

    #[test]
    fn extract_empty_when_no_patch_files() {
        let msg = make_message(Uuid::new_v4(), Some("abc"), None);
        assert!(extract_patches_from_message(&msg).is_empty());
    }

    #[test]
    fn extract_nested_format() {
        let msg = make_message(
            Uuid::new_v4(),
            Some("abc"),
            Some(r#"[{"hash":"h1","files":["a.txt"],"step":1}]"#),
        );
        let patches = extract_patches_from_message(&msg);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].hash, "h1");
        assert_eq!(patches[0].files, vec!["a.txt"]);
    }

    #[test]
    fn extract_flat_format() {
        let msg = make_message(Uuid::new_v4(), Some("abc"), Some(r#"["a.txt","b.txt"]"#));
        let patches = extract_patches_from_message(&msg);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].hash, "abc");
        assert_eq!(patches[0].files, vec!["a.txt", "b.txt"]);
    }

    #[test]
    fn collect_after_message_orders_correctly() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        let msgs = vec![
            make_message(
                id1,
                Some("h1"),
                Some(r#"[{"hash":"h1","files":["f1"],"step":1}]"#),
            ),
            make_message(
                id2,
                Some("h2"),
                Some(r#"[{"hash":"h2","files":["f2"],"step":1}]"#),
            ),
            make_message(
                id3,
                Some("h3"),
                Some(r#"[{"hash":"h3","files":["f3"],"step":1}]"#),
            ),
        ];

        // Collect patches after id1 → gets patches for id1, id2, and id3
        // (the target message's own patches are included).
        let patches = collect_patches_after_message(&msgs, id1).unwrap();
        assert_eq!(patches.len(), 3);
        // After reverse: oldest (id1) first.
        assert_eq!(patches[0].hash, "h1");
        assert_eq!(patches[1].hash, "h2");
        assert_eq!(patches[2].hash, "h3");
    }
}
