//! Port of codex's apply-patch parser — parses the `***` marker format.
//!
//! The official Lark grammar:
//!
//! ```lark
//! start: begin_patch hunk+ end_patch
//! begin_patch: "*** Begin Patch" LF
//! end_patch: "*** End Patch" LF?
//!
//! hunk: add_hunk | delete_hunk | update_hunk
//! add_hunk: "*** Add File: " filename LF add_line+
//! delete_hunk: "*** Delete File: " filename LF
//! update_hunk: "*** Update File: " filename LF change_move? change?
//!
//! filename: /(.+)/
//! add_line: "+" /(.*)/ LF -> line
//!
//! change_move: "*** Move to: " filename LF
//! change: (change_context | change_line)+ eof_line?
//! change_context: ("@@" | "@@ " /(.+)/) LF
//! change_line: ("+" | "-" | " ") /(.*)/ LF
//! eof_line: "*** End of File" LF
//! ```

use std::fmt;
use std::path::PathBuf;

pub(crate) const BEGIN_PATCH_MARKER: &str = "*** Begin Patch";
pub(crate) const END_PATCH_MARKER: &str = "*** End Patch";
pub(crate) const ENVIRONMENT_ID_MARKER: &str = "*** Environment ID: ";
pub(crate) const ADD_FILE_MARKER: &str = "*** Add File: ";
pub(crate) const DELETE_FILE_MARKER: &str = "*** Delete File: ";
pub(crate) const UPDATE_FILE_MARKER: &str = "*** Update File: ";
pub(crate) const MOVE_TO_MARKER: &str = "*** Move to: ";
pub(crate) const EOF_MARKER: &str = "*** End of File";
pub(crate) const CHANGE_CONTEXT_MARKER: &str = "@@ ";
pub(crate) const EMPTY_CHANGE_CONTEXT_MARKER: &str = "@@";

#[derive(Debug, PartialEq, Clone)]
pub enum ParseError {
    InvalidPatch(String),
    InvalidHunk {
        message: String,
        line_number: usize,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::InvalidPatch(msg) => write!(f, "invalid patch: {msg}"),
            ParseError::InvalidHunk {
                message,
                line_number,
            } => write!(f, "invalid hunk at line {line_number}, {message}"),
        }
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, PartialEq, Clone)]
pub enum Hunk {
    AddFile {
        path: PathBuf,
        contents: String,
    },
    DeleteFile {
        path: PathBuf,
    },
    UpdateFile {
        path: PathBuf,
        move_path: Option<PathBuf>,
        chunks: Vec<UpdateFileChunk>,
    },
}

impl Hunk {
    pub fn path(&self) -> &PathBuf {
        match self {
            Hunk::AddFile { path, .. } => path,
            Hunk::DeleteFile { path } => path,
            Hunk::UpdateFile {
                move_path: Some(path),
                ..
            } => path,
            Hunk::UpdateFile {
                path,
                move_path: None,
                ..
            } => path,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct UpdateFileChunk {
    /// A single line of context used to narrow down the position of the chunk
    /// (usually a class, method, or function definition).
    pub change_context: Option<String>,
    /// Lines to find in the existing file.
    pub old_lines: Vec<String>,
    /// Replacement lines.
    pub new_lines: Vec<String>,
    /// If true, old_lines must occur at the end of the file.
    pub is_end_of_file: bool,
}

/// A parsed patch, which may contain multiple file operations.
#[derive(Debug, PartialEq)]
pub struct ParsedPatch {
    pub hunks: Vec<Hunk>,
    pub environment_id: Option<String>,
}

/// Parse a patch string in codex `***` format.
pub fn parse_patch(patch: &str) -> Result<ParsedPatch, ParseError> {
    let lines: Vec<&str> = patch.trim().lines().collect();
    let (hunk_lines, _) = check_patch_boundaries(&lines)?;

    let (environment_id, mut remaining_lines, mut line_number) =
        parse_environment_id_preamble(hunk_lines)?;

    let mut hunks: Vec<Hunk> = Vec::new();
    while !remaining_lines.is_empty() {
        let (hunk, consumed) = parse_one_hunk(remaining_lines, line_number)?;
        hunks.push(hunk);
        line_number += consumed;
        remaining_lines = &remaining_lines[consumed..];
    }

    Ok(ParsedPatch {
        hunks,
        environment_id,
    })
}

/// Check that the patch starts with `*** Begin Patch` and ends with `*** End Patch`,
/// return the inner lines (without the envelope).
fn check_patch_boundaries<'a>(
    lines: &'a [&'a str],
) -> Result<(&'a [&'a str], &'a [&'a str]), ParseError> {
    let first = lines
        .first()
        .ok_or_else(|| ParseError::InvalidPatch("patch is empty".to_string()))?;
    if !first.trim().starts_with(BEGIN_PATCH_MARKER) {
        return Err(ParseError::InvalidPatch(format!(
            "patch must start with '*** Begin Patch', got: {first}"
        )));
    }

    // Find the last non-empty line
    let last = lines
        .iter()
        .rev()
        .find(|l| !l.trim().is_empty())
        .ok_or_else(|| ParseError::InvalidPatch("patch is empty".to_string()))?;
    if !last.trim().starts_with(END_PATCH_MARKER) {
        return Err(ParseError::InvalidPatch(format!(
            "patch must end with '*** End Patch', got: {last}"
        )));
    }

    // Strip begin/end markers
    let inner = &lines[1..];
    let end_idx = inner
        .iter()
        .position(|l| l.trim().starts_with(END_PATCH_MARKER))
        .unwrap_or(inner.len());
    let hunk_lines = &inner[..end_idx];

    let patch_lines = lines;

    Ok((hunk_lines, patch_lines))
}

fn parse_environment_id_preamble<'a>(
    hunk_lines: &'a [&'a str],
) -> Result<(Option<String>, &'a [&'a str], usize), ParseError> {
    let Some(first_line) = hunk_lines.first() else {
        return Ok((None, hunk_lines, 2));
    };
    let Some(environment_id) = first_line
        .trim_start()
        .strip_prefix(ENVIRONMENT_ID_MARKER)
    else {
        return Ok((None, hunk_lines, 2));
    };
    let environment_id = environment_id.trim();
    if environment_id.is_empty() {
        return Err(ParseError::InvalidPatch(
            "Environment ID marker found but no ID provided".to_string(),
        ));
    }
    Ok((
        Some(environment_id.to_string()),
        &hunk_lines[1..],
        3,
    ))
}

fn parse_one_hunk(lines: &[&str], line_number: usize) -> Result<(Hunk, usize), ParseError> {
    let first_line = lines[0].trim();

    if let Some(path) = first_line.strip_prefix(ADD_FILE_MARKER) {
        let mut contents = String::new();
        let mut parsed_lines = 1;
        for add_line in &lines[1..] {
            if let Some(line_to_add) = add_line.strip_prefix('+') {
                contents.push_str(line_to_add);
                contents.push('\n');
                parsed_lines += 1;
            } else {
                break;
            }
        }
        return Ok((
            Hunk::AddFile {
                path: PathBuf::from(path),
                contents,
            },
            parsed_lines,
        ));
    }

    if let Some(path) = first_line.strip_prefix(DELETE_FILE_MARKER) {
        return Ok((
            Hunk::DeleteFile {
                path: PathBuf::from(path),
            },
            1,
        ));
    }

    if let Some(path) = first_line.strip_prefix(UPDATE_FILE_MARKER) {
        let mut remaining = &lines[1..];
        let mut parsed = 1;

        let move_path = remaining
            .first()
            .and_then(|x| x.strip_prefix(MOVE_TO_MARKER));

        if move_path.is_some() {
            remaining = &remaining[1..];
            parsed += 1;
        }

        let mut chunks = Vec::new();
        while !remaining.is_empty() {
            if remaining[0].trim().is_empty() {
                parsed += 1;
                remaining = &remaining[1..];
                continue;
            }

            if remaining[0].starts_with('*') {
                break;
            }

            let (chunk, consumed) =
                parse_update_file_chunk(remaining, line_number + parsed, chunks.is_empty())?;
            chunks.push(chunk);
            parsed += consumed;
            remaining = &remaining[consumed..];
        }

        if chunks.is_empty() {
            return Err(ParseError::InvalidHunk {
                message: format!(
                    "Update file hunk for path '{}' is empty",
                    PathBuf::from(path).display()
                ),
                line_number,
            });
        }

        return Ok((
            Hunk::UpdateFile {
                path: PathBuf::from(path),
                move_path: move_path.map(PathBuf::from),
                chunks,
            },
            parsed,
        ));
    }

    Err(ParseError::InvalidHunk {
        message: format!(
            "'{first_line}' is not a valid hunk header. \
             Valid headers: '*** Add File: <path>', '*** Delete File: <path>', \
             '*** Update File: <path>'"
        ),
        line_number,
    })
}

fn parse_update_file_chunk(
    lines: &[&str],
    line_number: usize,
    allow_missing_context: bool,
) -> Result<(UpdateFileChunk, usize), ParseError> {
    if lines.is_empty() {
        return Err(ParseError::InvalidHunk {
            message: "Update hunk does not contain any lines".to_string(),
            line_number,
        });
    }

    let (change_context, start_index) = if lines[0] == EMPTY_CHANGE_CONTEXT_MARKER {
        (None, 1)
    } else if let Some(context) = lines[0].strip_prefix(CHANGE_CONTEXT_MARKER) {
        (Some(context.to_string()), 1)
    } else if !allow_missing_context {
        return Err(ParseError::InvalidHunk {
            message: format!(
                "Expected update hunk to start with a @@ context marker, got: '{}'",
                lines[0]
            ),
            line_number,
        });
    } else {
        (None, 0)
    };

    if start_index >= lines.len() {
        return Err(ParseError::InvalidHunk {
            message: "Update hunk does not contain any lines".to_string(),
            line_number: line_number + 1,
        });
    }

    let mut chunk = UpdateFileChunk {
        change_context,
        old_lines: Vec::new(),
        new_lines: Vec::new(),
        is_end_of_file: false,
    };

    let mut parsed_lines = 0;
    for line in &lines[start_index..] {
        if *line == EOF_MARKER {
            if parsed_lines == 0 {
                return Err(ParseError::InvalidHunk {
                    message: "Update hunk does not contain any lines".to_string(),
                    line_number: line_number + 1,
                });
            }
            chunk.is_end_of_file = true;
            parsed_lines += 1;
            break;
        }

        match line.chars().next() {
            None => {
                // Empty line — treat as context (empty line in both old and new)
                chunk.old_lines.push(String::new());
                chunk.new_lines.push(String::new());
            }
            Some(' ') => {
                let content = &line[1..];
                chunk.old_lines.push(content.to_string());
                chunk.new_lines.push(content.to_string());
            }
            Some('+') => {
                chunk.new_lines.push(line[1..].to_string());
            }
            Some('-') => {
                chunk.old_lines.push(line[1..].to_string());
            }
            _ => {
                if parsed_lines == 0 {
                    return Err(ParseError::InvalidHunk {
                        message: format!(
                            "Unexpected line in update hunk: '{line}'. \
                             Lines should start with ' ' (context), '+' (added), or '-' (removed)"
                        ),
                        line_number: line_number + 1,
                    });
                }
                // Assume this is the start of the next hunk.
                break;
            }
        }
        parsed_lines += 1;
    }

    Ok((chunk, parsed_lines + start_index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_begin_marker() {
        let err =
            parse_patch("*** Update File: x\n@@\n-old\n+new\n*** End Patch\n").unwrap_err();
        assert!(err.to_string().contains("must start with '*** Begin Patch'"));
    }

    #[test]
    fn test_missing_end_marker() {
        let err =
            parse_patch("*** Begin Patch\n*** Update File: x\n@@\n-old\n+new\n").unwrap_err();
        assert!(err.to_string().contains("must end with '*** End Patch'"));
    }

    #[test]
    fn test_add_file() {
        let patch = "*** Begin Patch\n*** Add File: hello.txt\n+Hello\n+World\n*** End Patch\n";
        let parsed = parse_patch(patch).unwrap();
        assert_eq!(parsed.hunks.len(), 1);
        assert_eq!(
            parsed.hunks[0],
            Hunk::AddFile {
                path: PathBuf::from("hello.txt"),
                contents: "Hello\nWorld\n".to_string(),
            }
        );
    }

    #[test]
    fn test_delete_file() {
        let patch = "*** Begin Patch\n*** Delete File: old.txt\n*** End Patch\n";
        let parsed = parse_patch(patch).unwrap();
        assert_eq!(parsed.hunks.len(), 1);
        assert_eq!(
            parsed.hunks[0],
            Hunk::DeleteFile {
                path: PathBuf::from("old.txt"),
            }
        );
    }

    #[test]
    fn test_update_file() {
        let patch =
            "*** Begin Patch\n*** Update File: main.rs\n@@\n-foo\n+bar\n*** End Patch\n";
        let parsed = parse_patch(patch).unwrap();
        assert_eq!(parsed.hunks.len(), 1);
        match &parsed.hunks[0] {
            Hunk::UpdateFile {
                path,
                move_path,
                chunks,
            } => {
                assert_eq!(path, &PathBuf::from("main.rs"));
                assert!(move_path.is_none());
                assert_eq!(chunks.len(), 1);
                assert_eq!(chunks[0].old_lines, vec!["foo"]);
                assert_eq!(chunks[0].new_lines, vec!["bar"]);
            }
            _ => panic!("expected UpdateFile"),
        }
    }

    #[test]
    fn test_update_file_with_move() {
        let patch =
            "*** Begin Patch\n*** Update File: src.rs\n*** Move to: dst.rs\n@@\n-old\n+new\n*** End Patch\n";
        let parsed = parse_patch(patch).unwrap();
        match &parsed.hunks[0] {
            Hunk::UpdateFile { path, move_path, .. } => {
                // path is the original from *** Update File: header
                assert_eq!(path, &PathBuf::from("src.rs"));
                // move_path is the destination from *** Move to: header
                assert_eq!(move_path, &Some(PathBuf::from("dst.rs")));
                // .path() returns the effective path (move_path when present)
                assert_eq!(parsed.hunks[0].path(), &PathBuf::from("dst.rs"));
            }
            _ => panic!("expected UpdateFile"),
        }
    }

    #[test]
    fn test_update_file_with_context() {
        let patch =
            "*** Begin Patch\n*** Update File: app.py\n@@ def foo():\n-bar\n+baz\n*** End Patch\n";
        let parsed = parse_patch(patch).unwrap();
        match &parsed.hunks[0] {
            Hunk::UpdateFile { chunks, .. } => {
                assert_eq!(chunks[0].change_context, Some("def foo():".to_string()));
            }
            _ => panic!("expected UpdateFile"),
        }
    }

    #[test]
    fn test_multiple_chunks() {
        let patch = "*** Begin Patch\n*** Update File: multi.txt\n@@\n foo\n-bar\n+BAR\n@@\n baz\n-qux\n+QUX\n*** End Patch\n";
        let parsed = parse_patch(patch).unwrap();
        match &parsed.hunks[0] {
            Hunk::UpdateFile { chunks, .. } => {
                assert_eq!(chunks.len(), 2);
                assert_eq!(chunks[0].old_lines, vec!["foo", "bar"]);
                assert_eq!(chunks[0].new_lines, vec!["foo", "BAR"]);
                assert_eq!(chunks[1].old_lines, vec!["baz", "qux"]);
                assert_eq!(chunks[1].new_lines, vec!["baz", "QUX"]);
            }
            _ => panic!("expected UpdateFile"),
        }
    }

    #[test]
    fn test_end_of_file_marker() {
        let patch = "*** Begin Patch\n*** Update File: tail.txt\n@@\n foo\n-bar\n+baz\n*** End of File\n*** End Patch\n";
        let parsed = parse_patch(patch).unwrap();
        match &parsed.hunks[0] {
            Hunk::UpdateFile { chunks, .. } => {
                assert!(chunks[0].is_end_of_file);
            }
            _ => panic!("expected UpdateFile"),
        }
    }

    #[test]
    fn test_multiple_file_ops() {
        let patch = "\
*** Begin Patch
*** Add File: new.txt
+content
*** Update File: existing.py
@@
-old
+new
*** Delete File: old.py
*** End Patch
";
        let parsed = parse_patch(patch).unwrap();
        assert_eq!(parsed.hunks.len(), 3);
    }
}
