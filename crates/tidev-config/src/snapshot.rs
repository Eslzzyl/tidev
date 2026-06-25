use serde::{Deserialize, Serialize};

/// Runtime configuration for the workspace git-snapshot service.
///
/// The snapshot service backs undo/redo and revert. It is enabled by
/// default because undo is a core feature of tidev. These knobs exist
/// so the user can tune behaviour for very large worktrees (e.g. running
/// tidev inside the home directory), where the initial file scan can
/// otherwise stall the UI.
///
/// Defaults are picked to "just work" on small/medium repos and to
/// degrade gracefully on huge ones.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotConfig {
    /// Master switch. Default: `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Extra ignore globs, applied **in addition to** the built-in
    /// `DEFAULT_IGNORED_DIRS` (`.git`, `node_modules`, `target`, ...).
    #[serde(default)]
    pub ignore_globs: Vec<String>,

    /// Files larger than this many bytes are excluded from the snapshot
    /// index. Default: 2 MiB. Set to `0` to disable the size cap.
    #[serde(default = "default_max_file_size")]
    pub max_file_size: u64,

    /// Hard cap on the number of files included in one snapshot. `0` =
    /// no cap.
    #[serde(default)]
    pub max_files: usize,

    /// Soft timeout for a single track call, in milliseconds.
    /// `0` disables the timeout.
    #[serde(default = "default_track_timeout_ms")]
    pub track_timeout_ms: u64,

    /// Concurrency for the large-file metadata stat probe. Default: 8.
    #[serde(default = "default_stat_concurrency")]
    pub stat_concurrency: usize,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ignore_globs: Vec::new(),
            max_file_size: default_max_file_size(),
            max_files: 0,
            track_timeout_ms: default_track_timeout_ms(),
            stat_concurrency: default_stat_concurrency(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_max_file_size() -> u64 {
    2 * 1024 * 1024
}

fn default_track_timeout_ms() -> u64 {
    30_000
}

fn default_stat_concurrency() -> usize {
    8
}
