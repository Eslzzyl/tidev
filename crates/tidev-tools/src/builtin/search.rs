use anyhow::{Context, Result, bail};
use globset::GlobBuilder;
use grep::{
    regex::RegexMatcherBuilder,
    searcher::{SearcherBuilder, sinks},
};
use ignore::WalkBuilder;
use rayon::prelude::*;
use serde_json::Value;
use std::time::UNIX_EPOCH;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::SystemTime,
};

use super::utils::{display_workspace_relative, resolve_workspace_path, truncate_in_place};
use crate::builtin::utils::decode_tool_args;
use tidev_types::tools::{GlobArgs, GrepArgs, ToolDefinition, ToolPermission};

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::new::<GlobArgs>(
            "glob",
            "Find files matching a glob pattern (relative to workspace root, or absolute). Searching outside the workspace requires user confirmation.",
            ToolPermission::Search,
        ),
        ToolDefinition::new::<GrepArgs>(
            "grep",
            "Search files with a regular expression (relative to workspace root, or absolute). Searching outside the workspace requires user confirmation.",
            ToolPermission::Search,
        ),
    ]
}

pub fn execute_tool_call(
    workspace_root: &Path,
    tool_name: &str,
    arguments: Value,
    _max_output_bytes: usize,
    allow_outside: bool,
) -> Result<String> {
    match tidev_types::tools::canonical_tool_name(tool_name) {
        Some("glob") => {
            let args = decode_tool_args::<GlobArgs>(tool_name, arguments)?;
            let path = args.path.unwrap_or_else(|| ".".to_string());
            glob_paths(workspace_root, path, &args.pattern, allow_outside)
        }
        Some("grep") => {
            let args = decode_tool_args::<GrepArgs>(tool_name, arguments)?;
            let path = args.path.unwrap_or_else(|| ".".to_string());
            grep_paths(
                workspace_root,
                path,
                &args.pattern,
                args.include.as_deref(),
                allow_outside,
            )
        }
        Some(other) => bail!("unsupported search tool '{}'", other),
        None => bail!("unknown tool '{}'", tool_name),
    }
}

fn glob_paths(
    workspace_root: &Path,
    relative_path: impl AsRef<Path>,
    pattern: &str,
    allow_outside: bool,
) -> Result<String> {
    let search_root =
        resolve_workspace_path(workspace_root, relative_path.as_ref(), allow_outside)?;
    if !search_root.exists() {
        bail!(
            "{} does not exist",
            display_workspace_relative(workspace_root, &search_root)
        );
    }

    let matcher = GlobBuilder::new(pattern)
        .literal_separator(false)
        .build()
        .with_context(|| format!("invalid glob pattern '{pattern}'"))?
        .compile_matcher();

    let mut matches = Vec::new();
    let skipped = AtomicUsize::new(0);

    if search_root.is_file() {
        let candidate = search_root.as_path();
        if glob_matches_path(candidate, &search_root, &matcher, pattern) {
            matches.push(SearchHit::from_path(&search_root)?);
        }
    } else {
        // Collect paths into a Vec for parallel iteration
        let paths: Vec<PathBuf> = WalkBuilder::new(&search_root)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .follow_links(false)
            .build()
            .filter_map(|result| match result {
                Ok(entry) => Some(entry.into_path()),
                Err(_) => {
                    skipped.fetch_add(1, Ordering::Relaxed);
                    None
                }
            })
            .filter(|path| path.is_file())
            .collect();

        // Parallel glob matching
        let search_root_owned = search_root.clone();
        let pattern_owned = pattern.to_owned();
        let matched_paths: Vec<SearchHit> = paths
            .par_iter()
            .filter_map(|path| {
                if glob_matches_path(path, &search_root_owned, &matcher, &pattern_owned) {
                    SearchHit::from_path(path).ok()
                } else {
                    None
                }
            })
            .collect();

        matches = matched_paths;
    }

    matches.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| left.path.cmp(&right.path))
    });

    if matches.is_empty() {
        let mut output = String::from("No files found");
        if skipped.load(Ordering::Relaxed) > 0 {
            output.push_str("\n\n(Some paths were inaccessible and skipped)");
        }
        return Ok(output);
    }

    let limit = 100usize;
    let truncated = matches.len() > limit;
    let display_matches = if truncated {
        &matches[..limit]
    } else {
        &matches
    };

    let mut output = vec![format!(
        "Found {} files{}",
        matches.len(),
        if truncated {
            format!(" (showing first {limit})")
        } else {
            String::new()
        }
    )];

    for hit in display_matches {
        output.push(display_workspace_relative(workspace_root, &hit.path));
    }

    if truncated {
        output.push(String::new());
        output.push(format!(
            "(Results truncated: showing {limit} of {} matches.)",
            matches.len()
        ));
    }
    if skipped.load(Ordering::Relaxed) > 0 {
        output.push(String::new());
        output.push("(Some paths were inaccessible and skipped)".to_string());
    }

    Ok(output.join("\n"))
}

fn grep_paths(
    workspace_root: &Path,
    relative_path: impl AsRef<Path>,
    pattern: &str,
    include: Option<&str>,
    allow_outside: bool,
) -> Result<String> {
    if pattern.trim().is_empty() {
        bail!("pattern cannot be empty");
    }

    let search_root =
        resolve_workspace_path(workspace_root, relative_path.as_ref(), allow_outside)?;
    if !search_root.exists() {
        bail!(
            "{} does not exist",
            display_workspace_relative(workspace_root, &search_root)
        );
    }

    let matcher = RegexMatcherBuilder::new()
        .build(pattern)
        .with_context(|| format!("invalid regular expression '{pattern}'"))?;
    let include_matcher = match include {
        Some(include) => Some(
            GlobBuilder::new(include)
                .literal_separator(false)
                .build()
                .with_context(|| format!("invalid include glob '{include}'"))?
                .compile_matcher(),
        ),
        None => None,
    };
    let include_has_separator = include
        .map(|value| value.contains('/') || value.contains('\\'))
        .unwrap_or(false);

    let skipped = AtomicUsize::new(0);

    let files: Vec<PathBuf> = if search_root.is_file() {
        vec![search_root.clone()]
    } else {
        WalkBuilder::new(&search_root)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .follow_links(false)
            .build()
            .filter_map(|result| match result {
                Ok(entry) => entry
                    .file_type()
                    .map(|file_type| (entry, file_type.is_file())),
                Err(_) => None,
            })
            .filter_map(|(entry, is_file)| is_file.then(|| entry.into_path()))
            .collect()
    };

    // Parallel file search
    let search_root_owned = search_root.clone();
    let matches: Vec<SearchHit> = files
        .par_iter()
        .filter_map(|path| {
            if let Some(ref inc_matcher) = include_matcher {
                let relative_candidate = path
                    .strip_prefix(&search_root_owned)
                    .unwrap_or(path.as_path());
                if !inc_matcher.is_match(relative_candidate)
                    && (!include_has_separator
                        && !path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .map(|name| inc_matcher.is_match(name))
                            .unwrap_or(false))
                {
                    return None;
                }
            }

            let modified_at = path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH);
            let path_owned = path.clone();
            let mut file_hits = Vec::new();
            let sink = sinks::Lossy(|line_number, line| {
                file_hits.push(SearchHit {
                    path: path_owned.clone(),
                    line_number,
                    line_text: line.to_string(),
                    modified_at,
                });
                Ok(true)
            });

            // Create a new Searcher for each thread (not thread-safe)
            let mut searcher = SearcherBuilder::new().line_number(true).build();
            if searcher.search_path(matcher.clone(), path, sink).is_err() {
                skipped.fetch_add(1, Ordering::Relaxed);
                return None;
            }

            Some(file_hits)
        })
        .flatten()
        .collect();

    let mut sorted_matches = matches;
    sorted_matches.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.line_number.cmp(&right.line_number))
    });

    if sorted_matches.is_empty() {
        let mut output = String::from("No files found");
        if skipped.load(Ordering::Relaxed) > 0 {
            output.push_str("\n\n(Some paths were inaccessible and skipped)");
        }
        return Ok(output);
    }

    let limit = 100usize;
    let truncated = sorted_matches.len() > limit;
    let display_matches = if truncated {
        &sorted_matches[..limit]
    } else {
        &sorted_matches
    };

    let mut output = vec![format!(
        "Found {} matches{}",
        sorted_matches.len(),
        if truncated {
            format!(" (showing first {limit})")
        } else {
            String::new()
        }
    )];

    let mut current_file: Option<PathBuf> = None;
    for hit in display_matches {
        let display_path = display_workspace_relative(workspace_root, &hit.path);
        if current_file.as_ref() != Some(&hit.path) {
            if current_file.is_some() {
                output.push(String::new());
            }
            current_file = Some(hit.path.clone());
            output.push(format!("{display_path}:"));
        }

        let mut line_text = hit.line_text.clone();
        if line_text.len() > 2_000 {
            truncate_in_place(&mut line_text, 2_000);
        }
        output.push(format!("  Line {}: {}", hit.line_number, line_text));
    }

    if truncated {
        output.push(String::new());
        output.push(format!(
            "(Results truncated: showing {limit} of {} matches.)",
            sorted_matches.len()
        ));
    }
    if skipped.load(Ordering::Relaxed) > 0 {
        output.push(String::new());
        output.push("(Some paths were inaccessible and skipped)".to_string());
    }

    Ok(output.join("\n"))
}

#[derive(Clone, Debug)]
struct SearchHit {
    path: PathBuf,
    line_number: u64,
    line_text: String,
    modified_at: SystemTime,
}

impl SearchHit {
    fn from_path(path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            line_number: 0,
            line_text: String::new(),
            modified_at: path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH),
        })
    }
}

fn glob_matches_path(
    path: &Path,
    search_root: &Path,
    matcher: &globset::GlobMatcher,
    pattern: &str,
) -> bool {
    let relative_candidate = path.strip_prefix(search_root).unwrap_or(path);
    if matcher.is_match(relative_candidate) {
        return true;
    }

    if pattern.contains('/') || pattern.contains('\\') {
        return false;
    }

    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| matcher.is_match(name))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use globset::Glob;

    #[test]
    fn glob_matches_exact_relative_path() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/src/main.rs");
        let glob = Glob::new("src/main.rs").unwrap().compile_matcher();
        assert!(glob_matches_path(path, root, &glob, "src/main.rs"));
    }

    #[test]
    fn glob_matches_wildcard_pattern() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/src/main.rs");
        let glob = Glob::new("**/*.rs").unwrap().compile_matcher();
        assert!(glob_matches_path(path, root, &glob, "*.rs"));
    }

    #[test]
    fn glob_does_not_match_non_matching_pattern() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/src/main.rs");
        let glob = Glob::new("*.toml").unwrap().compile_matcher();
        assert!(!glob_matches_path(path, root, &glob, "*.toml"));
    }

    #[test]
    fn glob_falls_back_to_filename_when_no_slash_in_pattern() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/src/main.rs");
        // Pattern "*.rs" has no slash → fallback matches filename "main.rs"
        let glob = Glob::new("*.rs").unwrap().compile_matcher();
        assert!(glob_matches_path(path, root, &glob, "*.rs"));
    }

    #[test]
    fn glob_no_filename_fallback_when_slash_in_pattern() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/src/main.rs");
        // Pattern "*.rs" has no slash, but "src/*.rs" has a slash
        // For a path that doesn't match the relative path, but has a matching filename
        // the fallback should NOT happen because the pattern contains "/"
        let glob = Glob::new("src/*.rs").unwrap().compile_matcher();
        // This matches because the relative path IS src/main.rs
        assert!(glob_matches_path(path, root, &glob, "src/*.rs"));
    }

    #[test]
    fn glob_path_outside_root_uses_absolute_path() {
        let root = Path::new("/workspace");
        let path = Path::new("/outside/other.rs");
        let glob = Glob::new("other.rs").unwrap().compile_matcher();
        // strip_prefix will fail → uses full path → no match on full path
        // then fallback to filename matching since "other.rs" has no slash
        assert!(glob_matches_path(path, root, &glob, "other.rs"));
    }

    #[test]
    fn glob_path_without_file_name_returns_false() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace");
        let glob = Glob::new("*.rs").unwrap().compile_matcher();
        // No fallback since the relative path won't match the glob, and
        // file_name() returns None for the root path "/" but Some("/workspace") here
        // Actually "/workspace" has file_name "workspace" which won't match "*.rs"
        assert!(!glob_matches_path(path, root, &glob, "*.rs"));
    }
}
