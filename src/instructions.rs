use anyhow::{Context, Result};
use globset::GlobBuilder;
use ignore::WalkBuilder;
use reqwest::blocking::Client;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const INSTRUCTION_FILES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    ".github/copilot-instructions.md",
    "CONTEXT.md",
];

pub fn system_paths(
    workspace_root: &Path,
    config_dir: &Path,
    instructions: &[String],
) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();

    let mut push_unique = |path: PathBuf| {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
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

pub fn system_prompt_and_sources(
    workspace_root: &Path,
    config_dir: &Path,
    instructions: &[String],
) -> Result<(String, Vec<String>)> {
    let mut sections = Vec::new();
    let mut sources = Vec::new();
    let paths = system_paths(workspace_root, config_dir, instructions)?;

    for path in paths {
        if let Ok(content) = fs::read_to_string(&path) {
            if !content.trim().is_empty() {
                sections.push(format!(
                    "Instructions from: {}\n{}",
                    path.display(),
                    content
                ));
                sources.push(path.display().to_string());
            }
        }
    }

    for url in instructions
        .iter()
        .filter(|item| item.starts_with("http://") || item.starts_with("https://"))
    {
        if let Ok(content) = fetch_remote(url) {
            if !content.trim().is_empty() {
                sections.push(format!("Instructions from: {}\n{}", url, content));
                sources.push(url.clone());
            }
        }
    }

    Ok((sections.join("\n\n"), sources))
}

pub fn system_prompt(
    workspace_root: &Path,
    config_dir: &Path,
    instructions: &[String],
) -> Result<String> {
    Ok(system_prompt_and_sources(workspace_root, config_dir, instructions)?.0)
}

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

pub fn resolve_nearby_instructions(
    workspace_root: &Path,
    target: &Path,
    excluded: &HashSet<PathBuf>,
) -> Result<Vec<PathBuf>> {
    let mut results = Vec::new();
    let target = target
        .canonicalize()
        .unwrap_or_else(|_| target.to_path_buf());
    let workspace_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());

    let mut current = target.parent().unwrap_or(&target).to_path_buf();

    while current.starts_with(&workspace_root) {
        for file_name in INSTRUCTION_FILES {
            let candidate = current.join(file_name);
            if candidate.exists() {
                let canonical = candidate.canonicalize().unwrap_or(candidate.clone());
                if excluded.contains(&canonical) || results.contains(&canonical) {
                    continue;
                }
                if canonical != target {
                    results.push(canonical);
                }
                break;
            }
        }

        if current == workspace_root {
            break;
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn make_temp_dir() -> Result<PathBuf> {
        let dir = std::env::temp_dir().join(format!("tidev-instructions-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).context("failed to create temp dir")?;
        Ok(dir)
    }

    #[test]
    fn system_paths_finds_project_agent_file() -> Result<()> {
        let workspace = make_temp_dir()?;
        fs::write(workspace.join("AGENTS.md"), "# Root")?;

        let paths = system_paths(&workspace, &workspace, &[])?;
        assert_eq!(paths, vec![workspace.join("AGENTS.md").canonicalize()?]);
        Ok(())
    }

    #[test]
    fn system_paths_prefers_project_over_global() -> Result<()> {
        let workspace = make_temp_dir()?;
        let global = make_temp_dir()?;

        fs::write(workspace.join("AGENTS.md"), "# Root")?;
        fs::write(global.join("AGENTS.md"), "# Global")?;

        let paths = system_paths(&workspace, &global, &[])?;
        assert_eq!(
            paths,
            vec![
                workspace.join("AGENTS.md").canonicalize()?,
                global.join("AGENTS.md").canonicalize()?,
            ],
        );
        Ok(())
    }

    #[test]
    fn system_prompt_loads_config_instructions() -> Result<()> {
        let workspace = make_temp_dir()?;
        let global = make_temp_dir()?;
        let extra = workspace.join("docs");
        fs::create_dir_all(&extra)?;
        fs::write(extra.join("style.md"), "# Style")?;

        let prompt = system_prompt(&workspace, &global, &["docs/style.md".to_string()])?;
        assert!(prompt.contains("Instructions from:"));
        assert!(prompt.contains("# Style"));
        Ok(())
    }

    #[test]
    fn system_paths_finds_github_copilot_instructions() -> Result<()> {
        let workspace = make_temp_dir()?;
        fs::create_dir_all(workspace.join(".github"))?;
        fs::write(
            workspace.join(".github").join("copilot-instructions.md"),
            "# Copilot",
        )?;

        let paths = system_paths(&workspace, &workspace, &[])?;
        assert_eq!(
            paths,
            vec![
                workspace
                    .join(".github")
                    .join("copilot-instructions.md")
                    .canonicalize()?
            ]
        );
        Ok(())
    }

    #[test]
    fn resolve_nearby_instructions_finds_github_copilot_instructions() -> Result<()> {
        let workspace = make_temp_dir()?;
        let subdir = workspace.join("subdir").join("nested");
        fs::create_dir_all(&subdir)?;
        fs::create_dir_all(workspace.join(".github"))?;
        fs::write(
            workspace.join(".github").join("copilot-instructions.md"),
            "# Copilot",
        )?;
        fs::write(subdir.join("file.rs"), "let x = 1;")?;

        let excluded = HashSet::new();
        let results = resolve_nearby_instructions(&workspace, &subdir.join("file.rs"), &excluded)?;
        assert_eq!(
            results,
            vec![
                workspace
                    .join(".github")
                    .join("copilot-instructions.md")
                    .canonicalize()?
            ]
        );
        Ok(())
    }

    #[test]
    fn resolve_nearby_instructions_finds_subdirectory_agents() -> Result<()> {
        let workspace = make_temp_dir()?;
        let subdir = workspace.join("subdir").join("nested");
        fs::create_dir_all(&subdir)?;
        fs::write(workspace.join("subdir").join("AGENTS.md"), "# Subdir")?;
        fs::write(subdir.join("file.rs"), "let x = 1;")?;

        let excluded = HashSet::new();
        let results = resolve_nearby_instructions(&workspace, &subdir.join("file.rs"), &excluded)?;
        assert_eq!(
            results,
            vec![workspace.join("subdir").join("AGENTS.md").canonicalize()?],
        );
        Ok(())
    }
}
