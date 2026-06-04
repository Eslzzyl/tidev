//! Codex‑compatible `apply_patch` implementation.
//!
//! This module implements the same `***` marker‑based patch format used by
//! OpenAI's Codex CLI, so that GPT models can generate patches in a single
//! format regardless of which tool they're talking to.
//!
//! Format overview:
//!
//! ```text
//! *** Begin Patch
//! *** Add File: <path>
//! +content
//! *** Update File: <path>
//! *** Move to: <new_path>      (optional rename)
//! @@                          (optional context line)
//!  context
//! -old
//! +new
//! *** End of File             (optional — match at end of file)
//! *** Delete File: <path>
//! *** End Patch
//! ```

mod apply;
mod parser;
mod seek_sequence;

pub use apply::{ApplyPatchResult, apply_patch};
pub use parser::{ParseError, ParsedPatch, Hunk, UpdateFileChunk, parse_patch};
