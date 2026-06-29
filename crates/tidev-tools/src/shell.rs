//! Re-export shell detection from [`tidev_config::shell`].
//!
//! The canonical implementation now lives in `tidev-config`. This module
//! re-exports the public API so that existing `crate::shell::*` paths
//! inside tidev-tools continue to work.

pub use tidev_config::shell::{ResolvedShell, get, init, is_bash_like};
