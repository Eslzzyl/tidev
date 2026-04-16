use anyhow::{Context, Result, bail};
use globset::GlobBuilder;
use grep::{
    regex::RegexMatcherBuilder,
    searcher::{SearcherBuilder, sinks},
};
use ignore::WalkBuilder;
use serde_json::Value;
use std::{path::{Path, PathBuf}, time::SystemTime};
use std::time::UNIX_EPOCH;

use crate::tooling::tools::{GlobArgs, GrepArgs};
use super::utils::{display_workspace_relative, resolve_workspace_path, truncate_in_place};
use crate::tooling::{ToolDefinition, ToolPermission};

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::new::<GlobArgs>(
            "glob",
            "Find files matching a glob pattern inside the workspace",
            ToolPermission::Search,
        ),
        ToolDefinition::new::<GrepArgs>(
            "grep",
            "Search workspace files with a regular expression",
            ToolPermission::Search,
        ),
    ]
}

pub fn execute_tool_call(
    workspace_root: &Path,
    call: &crate::session::ToolCall,
    _max_output_bytes: usize,
) -> Result<String> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;

    match crate::tooling::canonical_tool_name(&call.name) {
        Some("glob") => {
            let args = serde_json::from_value::<GlobArgs>(arguments)
                .with_context(|| format!("failed to decode arguments for tool '{}'", call.name))?;
            let path = args.path.unwrap_or_else(|| ".".to_string());
            glob_paths(workspace_root, path, &args.pattern)
        }
        Some("grep") => {
            let args = serde_json::from_value::<GrepArgs>(arguments)
                .with_context(|| format!("failed to decode arguments for tool '{}'", call.name))?;
            let path = args.path.unwrap_or_else(|| ".".to_string());
            grep_paths(workspace_root, path, &args.pattern, args.include.as_deref())
        }
        Some(other) => bail!("unsupported search tool '{}'", other),
        None => bail!("unknown tool '{}'", call.name),
    }
}

fn glob_paths(
    workspace_root: &Path,
    relative_path: impl AsRef<Path>,
    pattern: &str,
) -> Result<String> {
    let search_root = resolve_workspace_path(workspace_root, relative_path.as_ref())?;
    if !search_root.exists() {
        bail!("{} does not exist", search_root.display());
    }

    let matcher = GlobBuilder::new(pattern)
        .literal_separator(false)
        .build()
        .with_context(|| format!("invalid glob pattern '{pattern}'"))?
        .compile_matcher();

    let mut matches = Vec::new();
    let mut skipped = 0usize;

    if search_root.is_file() {
        let candidate = search_root.as_path();
        if glob_matches_path(candidate, &search_root, &matcher, pattern) {
            matches.push(SearchHit::from_path(&search_root)?);
        }
    } else {
        for result in WalkBuilder::new(&search_root)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .follow_links(false)
            .build()
        {
            let entry = match result {
                Ok(entry) => entry,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };

            if !entry
                .file_type()
                .map(|file_type| file_type.is_file())
                .unwrap_or(false)
            {
                continue;
            }

            let path = entry.into_path();
            if !glob_matches_path(&path, &search_root, &matcher, pattern) {
                continue;
            }

            matches.push(SearchHit::from_path(&path)?);
        }
    }

    matches.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| left.path.cmp(&right.path))
    });

    if matches.is_empty() {
        let mut output = String::from("No files found");
        if skipped > 0 {
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
    if skipped > 0 {
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
) -> Result<String> {
    if pattern.trim().is_empty() {
        bail!("pattern cannot be empty");
    }

    let search_root = resolve_workspace_path(workspace_root, relative_path.as_ref())?;
    if !search_root.exists() {
        bail!("{} does not exist", search_root.display());
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

    let mut searcher = SearcherBuilder::new().line_number(true).build();
    let mut matches = Vec::new();
    let mut skipped = 0usize;

    let files: Box<dyn Iterator<Item = PathBuf>> = if search_root.is_file() {
        Box::new(std::iter::once(search_root.clone()))
    } else {
        Box::new(
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
                .filter_map(|(entry, is_file)| is_file.then(|| entry.into_path())),
        )
    };

    for path in files {
        if let Some(include_matcher) = &include_matcher {
            let relative_candidate = path.strip_prefix(&search_root).unwrap_or(path.as_path());
            if !include_matcher.is_match(relative_candidate)
                && (!include_has_separator
                    && !path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| include_matcher.is_match(name))
                        .unwrap_or(false))
            {
                continue;
            }
        }

        let modified_at = path
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        let path_for_sink = path.clone();
        let mut file_hits = Vec::new();
        let sink = sinks::Lossy(|line_number, line| {
            file_hits.push(SearchHit {
                path: path_for_sink.clone(),
                line_number,
                line_text: line.to_string(),
                modified_at,
            });
            Ok(true)
        });

        if searcher.search_path(matcher.clone(), &path, sink).is_err() {
            skipped += 1;
            continue;
        }

        matches.extend(file_hits);
    }

    matches.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.line_number.cmp(&right.line_number))
    });

    if matches.is_empty() {
        let mut output = String::from("No files found");
        if skipped > 0 {
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
        "Found {} matches{}",
        matches.len(),
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
            matches.len()
        ));
    }
    if skipped > 0 {
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
