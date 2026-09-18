//! Build metadata embedded in the tidev binaries.

include!(concat!(env!("OUT_DIR"), "/build_info.rs"));

/// Whether the source worktree contained uncommitted changes at build time.
pub const fn is_dirty() -> bool {
    DIRTY
}
