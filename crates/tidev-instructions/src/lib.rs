//! Agent instruction file lookup and loading for tidev.
//!
//! This crate finds and loads instruction files (e.g. `AGENTS.md`, `CLAUDE.md`,
//! `.github/copilot-instructions.md`, `CONTEXT.md`) that provide system-level
//! guidance to the AI agent.
//!
//! ## Public API
//!
//! * [`system_prompt`] — build a combined system prompt from all instruction sources
//! * [`system_prompt_and_sources`] — same, but also returns the list of source paths
//! * [`system_prompt_and_sources_with_cache`] — cached variant to avoid redundant I/O
//! * [`resolve_nearby_instructions`] — find instructions near a given file path

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use globset::GlobBuilder;
use ignore::WalkBuilder;
use reqwest::blocking::Client;
use std::time::Duration;

use tidev_utils::path::canonicalize_display;

/// Instruction file names to search for, in order of precedence.
const INSTRUCTION_FILES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    ".github/copilot-instructions.md",
    "CONTEXT.md",
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build the full system prompt from all instruction sources (local files and
/// remote URLs).
pub fn system_prompt(
    workspace_root: &Path,
    config_dir: &Path,
    instructions: &[String],
) -> Result<String> {
    Ok(system_prompt_and_sources(workspace_root, config_dir, instructions)?.0)
}

/// Build system prompt from instruction sources, returning (prompt, sources).
pub fn system_prompt_and_sources(
    workspace_root: &Path,
    config_dir: &Path,
    instructions: &[String],
) -> Result<(String, Vec<String>)> {
    let mut sections = Vec::new();
    let mut sources = Vec::new();
    let paths = system_paths(workspace_root, config_dir, instructions)?;

    for path in paths {
        if let Ok(content) = fs::read_to_string(&path)
            && !content.trim().is_empty()
        {
            sections.push(format!(
                "Instructions from: {}\n{}",
                path.display(),
                content
            ));
            sources.push(path.display().to_string());
        }
    }

    for url in instructions
        .iter()
        .filter(|item| item.starts_with("http://") || item.starts_with("https://"))
    {
        if let Ok(content) = fetch_remote(url)
            && !content.trim().is_empty()
        {
            sections.push(format!("Instructions from: {}\n{}", url, content));
            sources.push(url.clone());
        }
    }

    Ok((sections.join("\n\n"), sources))
}

/// Build system prompt with content caching to avoid redundant file I/O.
/// Returns (prompt, sources, updated_cache).
pub fn system_prompt_and_sources_with_cache(
    workspace_root: &Path,
    config_dir: &Path,
    instructions: &[String],
    cache: &HashMap<String, String>,
) -> Result<(String, Vec<String>, HashMap<String, String>)> {
    let mut sections = Vec::new();
    let mut sources = Vec::new();
    let mut new_cache = cache.clone();
    let paths = system_paths(workspace_root, config_dir, instructions)?;

    for path in paths {
        let path_str = path.display().to_string();
        if let Some(cached_content) = cache.get(&path_str) {
            log::info!(
                "system_prompt_and_sources_with_cache: HIT  path={}",
                path_str,
            );
            if !cached_content.trim().is_empty() {
                sections.push(format!(
                    "Instructions from: {}\n{}",
                    path.display(),
                    cached_content
                ));
                sources.push(path_str);
            }
        } else {
            log::info!(
                "system_prompt_and_sources_with_cache: MISS path={} cache_keys={:?}",
                path_str,
                cache.keys().collect::<Vec<_>>(),
            );
            if let Ok(content) = fs::read_to_string(&path)
                && !content.trim().is_empty()
            {
                new_cache.insert(path_str.clone(), content.clone());
                sections.push(format!(
                    "Instructions from: {}\n{}",
                    path.display(),
                    content
                ));
                sources.push(path_str);
            }
        }
    }

    for url in instructions
        .iter()
        .filter(|item| item.starts_with("http://") || item.starts_with("https://"))
    {
        if let Some(cached_content) = cache.get(url) {
            if !cached_content.trim().is_empty() {
                sections.push(format!("Instructions from: {}\n{}", url, cached_content));
                sources.push(url.clone());
            }
        } else {
            if let Ok(content) = fetch_remote(url)
                && !content.trim().is_empty()
            {
                new_cache.insert(url.clone(), content.clone());
                sections.push(format!("Instructions from: {}\n{}", url, content));
                sources.push(url.clone());
            }
        }
    }

    Ok((sections.join("\n\n"), sources, new_cache))
}

/// Collect all system-level instruction paths (project, global, and configured).
pub fn system_paths(
    workspace_root: &Path,
    config_dir: &Path,
    instructions: &[String],
) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();

    let mut push_unique = |path: PathBuf| {
        let canonical = canonicalize_display(&path);
        if seen.insert(canonical.clone()) {
            paths.push(canonical);
        }
    };

    if let Some(project_path) = find_project_instruction(workspace_root)? {
        push_unique(project_path);
    }

    let global_path = config_dir.join("AGENTS.md");
    if global_path.exists() {
        push_unique(global_path);
    }

    for raw in instructions {
        if raw.starts_with("http://") || raw.starts_with("https://") {
            continue;
        }

        let resolved = resolve_instruction_paths(workspace_root, raw)?;
        for path in resolved {
            push_unique(path);
        }
    }

    Ok(paths)
}

/// Find instruction files by walking up from a file path.
///
/// Returns `Vec<(filepath, content)>` for nearby instruction files, excluding
/// system-wide paths and the file itself.
pub fn resolve_nearby_instructions(
    workspace_root: &Path,
    config_dir: &Path,
    file_path: &Path,
) -> Result<Vec<(PathBuf, String)>> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    let target = canonicalize_display(file_path);
    let root = canonicalize_display(workspace_root);

    // Collect system-wide instruction paths
    let system_paths = system_paths(workspace_root, config_dir, &[])?;
    let system_set: HashSet<_> = system_paths
        .iter()
        .map(|p| canonicalize_display(p))
        .collect();

    // Walk upward from the file being read
    let mut current = target.parent().unwrap_or(&target);
    while current.starts_with(&root) {
        for file_name in INSTRUCTION_FILES {
            let candidate = current.join(file_name);
            let canonical = canonicalize_display(&candidate);

            // Skip if already loaded, system-wide, or the file itself
            if canonical == target {
                continue;
            }
            if system_set.contains(&canonical) {
                continue;
            }
            if seen.contains(&canonical) {
                continue;
            }

            if candidate.exists()
                && let Ok(content) = fs::read_to_string(&candidate)
                && !content.trim().is_empty()
            {
                seen.insert(canonical.clone());
                results.push((
                    canonical.clone(),
                    format!("Instructions from: {}\n{}", canonical.display(), content),
                ));
            }
        }

        if current == root {
            break;
        }
        current = current.parent().unwrap_or(current);
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn find_project_instruction(workspace_root: &Path) -> Result<Option<PathBuf>> {
    for ancestor in workspace_root.ancestors() {
        for file_name in INSTRUCTION_FILES {
            let candidate = ancestor.join(file_name);
            if candidate.exists() {
                return Ok(Some(candidate));
            }
        }
    }
    Ok(None)
}

fn resolve_instruction_paths(workspace_root: &Path, raw: &str) -> Result<Vec<PathBuf>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }

    let raw = if let Some(stripped) = raw.strip_prefix("~/") {
        dirs::home_dir()
            .map(|dir| dir.join(stripped))
            .unwrap_or_else(|| PathBuf::from(raw))
    } else {
        PathBuf::from(raw)
    };

    if raw.is_absolute() {
        if contains_glob(&raw) {
            return glob_absolute(&raw);
        }

        if raw.exists() {
            return Ok(vec![raw]);
        }

        return Ok(Vec::new());
    }

    if contains_glob(&raw) {
        return glob_relative(workspace_root, &raw);
    }

    let candidate = workspace_root.join(&raw);
    if candidate.exists() {
        return Ok(vec![candidate]);
    }

    Ok(Vec::new())
}

fn contains_glob(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.contains('*') || text.contains('?') || text.contains('[')
}

fn glob_relative(workspace_root: &Path, pattern: &Path) -> Result<Vec<PathBuf>> {
    let matcher = GlobBuilder::new(&pattern.to_string_lossy())
        .literal_separator(false)
        .build()
        .context("invalid glob pattern")?
        .compile_matcher();

    let mut results = Vec::new();
    let walker = WalkBuilder::new(workspace_root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .build();

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };

        if !entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false)
        {
            continue;
        }

        let path = entry.path();
        if let Ok(rel) = path.strip_prefix(workspace_root) {
            let candidate = rel.to_string_lossy();
            if matcher.is_match(&*candidate) {
                results.push(path.to_path_buf());
            }
        }
    }

    results.sort();
    Ok(results)
}

fn glob_absolute(pattern: &Path) -> Result<Vec<PathBuf>> {
    let matcher = GlobBuilder::new(&pattern.to_string_lossy())
        .literal_separator(false)
        .build()
        .context("invalid glob pattern")?
        .compile_matcher();

    let mut results = Vec::new();
    let root = Path::new("/");
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .build();

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };

        if !entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false)
        {
            continue;
        }

        let path = entry.path();
        let candidate = path.to_string_lossy();
        if matcher.is_match(&*candidate) {
            results.push(path.to_path_buf());
        }
    }

    results.sort();
    Ok(results)
}

fn fetch_remote(url: &str) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build http client")?;

    let response = client
        .get(url)
        .send()
        .context("failed to fetch remote instruction")?;

    let status = response.status();
    if !status.is_success() {
        return Ok(String::new());
    }

    response
        .text()
        .context("failed to read remote instruction body")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Auto-cleaning temp directory wrapper.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Result<Self> {
            let path =
                std::env::temp_dir().join(format!("{}-{}", prefix, uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).context("failed to create temp dir")?;
            Ok(Self { path })
        }

        fn path(&self) -> &PathBuf {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn make_temp_dir() -> Result<TempDir> {
        TempDir::new("tidev-instructions")
    }

    #[test]
    fn system_paths_finds_project_agent_file() -> Result<()> {
        let workspace = make_temp_dir()?;
        let ws_path = workspace.path().clone();
        fs::write(ws_path.join("AGENTS.md"), "# Root")?;

        let paths = system_paths(&ws_path, &ws_path, &[])?;
        assert_eq!(
            paths,
            vec![canonicalize_display(&ws_path.join("AGENTS.md"))]
        );
        Ok(())
    }

    #[test]
    fn system_paths_prefers_project_over_global() -> Result<()> {
        let workspace = make_temp_dir()?;
        let global = make_temp_dir()?;
        let ws_path = workspace.path().clone();
        let gl_path = global.path().clone();

        fs::write(ws_path.join("AGENTS.md"), "# Root")?;
        fs::write(gl_path.join("AGENTS.md"), "# Global")?;

        let paths = system_paths(&ws_path, &gl_path, &[])?;
        assert_eq!(
            paths,
            vec![
                canonicalize_display(&ws_path.join("AGENTS.md")),
                canonicalize_display(&gl_path.join("AGENTS.md")),
            ],
        );
        Ok(())
    }

    #[test]
    fn system_prompt_loads_config_instructions() -> Result<()> {
        let workspace = make_temp_dir()?;
        let global = make_temp_dir()?;
        let ws_path = workspace.path();
        let gl_path = global.path();
        let extra = ws_path.join("docs");
        fs::create_dir_all(&extra)?;
        fs::write(extra.join("style.md"), "# Style")?;

        let prompt = system_prompt(ws_path, gl_path, &["docs/style.md".to_string()])?;
        assert!(prompt.contains("Instructions from:"));
        assert!(prompt.contains("# Style"));
        Ok(())
    }

    #[test]
    fn system_paths_finds_github_copilot_instructions() -> Result<()> {
        let workspace = make_temp_dir()?;
        let ws_path = workspace.path();
        fs::create_dir_all(ws_path.join(".github"))?;
        fs::write(
            ws_path.join(".github").join("copilot-instructions.md"),
            "# Copilot",
        )?;

        let paths = system_paths(ws_path, ws_path, &[])?;
        assert_eq!(
            paths,
            vec![canonicalize_display(
                &ws_path.join(".github").join("copilot-instructions.md")
            )]
        );
        Ok(())
    }

    #[test]
    fn resolve_nearby_instructions_finds_github_copilot_instructions() -> Result<()> {
        let workspace = make_temp_dir()?;
        let config_dir = TempDir::new("tidev-test-config")?;
        let ws_path = workspace.path();
        let cf_path = config_dir.path();
        let subdir = ws_path.join("subdir").join("nested");
        fs::create_dir_all(&subdir)?;
        // Put the instruction file in a subdirectory, not in workspace root
        // to avoid being excluded by find_project_instruction
        fs::create_dir_all(subdir.join(".github"))?;
        fs::write(
            subdir.join(".github").join("copilot-instructions.md"),
            "# Copilot",
        )?;
        fs::write(subdir.join("file.rs"), "let x = 1;")?;

        let results = resolve_nearby_instructions(ws_path, cf_path, &subdir.join("file.rs"))?;

        let expected_path =
            canonicalize_display(&subdir.join(".github").join("copilot-instructions.md"));
        let expected_content = format!(
            "Instructions from: {}\n{}",
            expected_path.display(),
            "# Copilot"
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, expected_path);
        assert_eq!(results[0].1, expected_content);
        Ok(())
    }

    #[test]
    fn resolve_nearby_instructions_finds_subdirectory_agents() -> Result<()> {
        let workspace = make_temp_dir()?;
        let config_dir = TempDir::new("tidev-test-config")?;
        let ws_path = workspace.path();
        let cf_path = config_dir.path();
        let subdir = ws_path.join("subdir").join("nested");
        fs::create_dir_all(&subdir)?;
        fs::write(ws_path.join("subdir").join("AGENTS.md"), "# Subdir")?;
        fs::write(subdir.join("file.rs"), "let x = 1;")?;

        let results = resolve_nearby_instructions(ws_path, cf_path, &subdir.join("file.rs"))?;
        let expected_path = canonicalize_display(&ws_path.join("subdir").join("AGENTS.md"));
        let expected_content = format!(
            "Instructions from: {}\n{}",
            expected_path.display(),
            "# Subdir"
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, expected_path);
        assert_eq!(results[0].1, expected_content);
        Ok(())
    }
}
