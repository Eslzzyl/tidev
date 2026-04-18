use anyhow::{Context, Result};
use ignore::WalkBuilder;
use serde::Deserialize;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use reqwest::blocking::Client;

const SKILL_FILE_NAME: &str = "SKILL.md";
const SKILL_ROOTS: &[&str] = &[".opencode/skills", ".claude/skills", ".agents/skills"];
const MAX_COMPANION_FILES: usize = 10;

static CATALOG: OnceLock<Arc<SkillCatalogInner>> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub directory: PathBuf,
    pub location: PathBuf,
    pub content: String,
    pub companion_files: Vec<PathBuf>,
}

#[derive(Debug, Default)]
struct SkillCatalogInner {
    skills: Vec<SkillInfo>,
}

#[derive(Clone, Debug, Default)]
pub struct SkillCatalog {
    inner: Arc<SkillCatalogInner>,
}

#[derive(Clone, Debug, Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
}

fn discover_inner(
    workspace_root: &Path,
    config_dir: &Path,
    extra_sources: &[String],
) -> SkillCatalogInner {
    let mut skills = Vec::new();
    let mut seen_names = HashSet::new();
    let mut seen_locations = HashSet::new();

    for root in candidate_roots(workspace_root, config_dir) {
        for skill_file in discover_skill_files(&root) {
            let canonical_location = skill_file
                .canonicalize()
                .unwrap_or_else(|_| skill_file.clone());
            if !seen_locations.insert(canonical_location) {
                continue;
            }

            let Ok(skill) = parse_skill_file(&skill_file) else {
                continue;
            };

            if !seen_names.insert(skill.name.clone()) {
                continue;
            }

            skills.push(skill);
        }
    }

    for raw_source in extra_sources {
        let Some(skill) = load_additional_skill_source(raw_source, workspace_root) else {
            continue;
        };

        let canonical_location = skill
            .location
            .canonicalize()
            .unwrap_or_else(|_| skill.location.clone());
        if !seen_locations.insert(canonical_location) {
            continue;
        }

        if !seen_names.insert(skill.name.clone()) {
            continue;
        }

        skills.push(skill);
    }

    SkillCatalogInner { skills }
}

impl SkillCatalog {
    pub fn discover(workspace_root: &Path, config_dir: &Path, skill_sources: &[String]) -> Self {
        let inner = CATALOG
            .get_or_init(|| {
                let inner = discover_inner(workspace_root, config_dir, skill_sources);
                Arc::new(inner)
            })
            .clone();
        Self { inner }
    }

    pub fn all(&self) -> &[SkillInfo] {
        &self.inner.skills
    }

    pub fn is_empty(&self) -> bool {
        self.inner.skills.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&SkillInfo> {
        self.inner.skills.iter().find(|skill| skill.name == name)
    }

    pub fn tool_description(&self) -> String {
        if self.inner.skills.is_empty() {
            return String::from("Load a reusable skill by name. No skills were discovered.");
        }

        let mut description = String::from("Load a reusable skill by name. Available skills:\n");
        for skill in &self.inner.skills {
            description.push_str("- ");
            description.push_str(&skill.name);
            description.push_str(": ");
            description.push_str(skill.description.trim());
            description.push('\n');
        }

        description.trim_end().to_string()
    }

    pub fn permission_key_for_name(name: &str) -> String {
        format!("skill:{name}")
    }

    pub fn render_skill(&self, name: &str) -> Result<String> {
        let skill = self
            .get(name)
            .with_context(|| format!("unknown skill '{name}'"))?;

        let mut output = String::new();
        output.push_str(&format!("# Skill: {}\n\n", skill.name));
        output.push_str(&format!("Location: {}\n\n", skill.location.display()));
        output.push_str(skill.content.trim());

        if !skill.companion_files.is_empty() {
            output.push_str("\n\n## Companion files\n");
            for file in &skill.companion_files {
                output.push_str("- ");
                output.push_str(&file.display().to_string());
                output.push('\n');
            }
        }

        Ok(output)
    }
}

fn candidate_roots(workspace_root: &Path, config_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    for ancestor in workspace_root.ancestors() {
        for root in SKILL_ROOTS {
            let candidate = ancestor.join(root);
            if !candidate.is_dir() {
                continue;
            }

            let canonical = candidate
                .canonicalize()
                .unwrap_or_else(|_| candidate.clone());
            if seen.insert(canonical.clone()) {
                roots.push(canonical);
            }
        }
    }

    let global_root = config_dir.join("skills");
    if global_root.is_dir() {
        let canonical = global_root
            .canonicalize()
            .unwrap_or_else(|_| global_root.clone());
        if seen.insert(canonical.clone()) {
            roots.push(canonical);
        }
    }

    roots
}

fn discover_skill_files(root: &Path) -> Vec<PathBuf> {
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .build();

    let mut files = Vec::new();
    for entry in walker {
        let Ok(entry) = entry else {
            continue;
        };

        if !entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false)
        {
            continue;
        }

        if entry.file_name().to_string_lossy() == SKILL_FILE_NAME {
            files.push(entry.path().to_path_buf());
        }
    }

    files.sort();
    files
}

fn parse_skill_file(path: &Path) -> Result<SkillInfo, ()> {
    let raw_content = fs::read_to_string(path).map_err(|_| ())?;
    parse_skill_content(
        path.to_path_buf(),
        Some(path.parent().ok_or(())?.to_path_buf()),
        raw_content,
    )
}

fn parse_skill_content(
    location: PathBuf,
    directory: Option<PathBuf>,
    raw_content: String,
) -> Result<SkillInfo, ()> {
    let normalized_content = raw_content.replace("\r\n", "\n");
    let (frontmatter, body) = split_frontmatter(&normalized_content).ok_or(())?;
    let parsed: SkillFrontmatter = serde_yaml::from_str(frontmatter).map_err(|_| ())?;

    if !is_valid_skill_name(&parsed.name) {
        return Err(());
    }

    let companion_files = directory
        .as_ref()
        .map(|dir| collect_companion_files(dir, &location))
        .unwrap_or_default();

    if let Some(directory) = directory.as_ref() {
        let directory_name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(())?;

        if directory_name != parsed.name {
            return Err(());
        }
    }

    Ok(SkillInfo {
        name: parsed.name,
        description: parsed.description,
        directory: directory.unwrap_or_else(|| location.clone()),
        location,
        content: body.trim().to_string(),
        companion_files,
    })
}

fn load_additional_skill_source(raw_source: &str, workspace_root: &Path) -> Option<SkillInfo> {
    let raw_source = raw_source.trim();
    if raw_source.is_empty() {
        return None;
    }

    if raw_source.starts_with("http://") || raw_source.starts_with("https://") {
        return fetch_remote_skill(raw_source).ok();
    }

    let resolved = resolve_local_skill_source(workspace_root, raw_source)?;
    let content = fs::read_to_string(&resolved).ok()?;
    parse_skill_content(
        resolved.clone(),
        resolved.parent().map(Path::to_path_buf),
        content,
    )
    .ok()
}

fn resolve_local_skill_source(workspace_root: &Path, raw_source: &str) -> Option<PathBuf> {
    let candidate = if let Some(stripped) = raw_source.strip_prefix("~/") {
        dirs::home_dir()
            .map(|dir| dir.join(stripped))
            .unwrap_or_else(|| PathBuf::from(raw_source))
    } else {
        PathBuf::from(raw_source)
    };

    let candidate = if candidate.is_absolute() {
        candidate
    } else {
        workspace_root.join(candidate)
    };

    if candidate.is_file() {
        return Some(candidate);
    }

    let skill_file = candidate.join(SKILL_FILE_NAME);
    if skill_file.is_file() {
        return Some(skill_file);
    }

    None
}

fn fetch_remote_skill(url: &str) -> Result<SkillInfo, ()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| ())?;

    let response = client.get(url).send().map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }

    let content = response.text().map_err(|_| ())?;
    parse_skill_content(PathBuf::from(url), None, content)
}

fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let content = content.strip_prefix("---\n")?;
    let (frontmatter, body) = content.split_once("\n---\n")?;
    Some((frontmatter, body))
}

fn collect_companion_files(skill_dir: &Path, skill_file: &Path) -> Vec<PathBuf> {
    let walker = WalkBuilder::new(skill_dir)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .build();

    let mut files = Vec::new();
    for entry in walker {
        let Ok(entry) = entry else {
            continue;
        };

        if !entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false)
        {
            continue;
        }

        let path = entry.path();
        if path == skill_file {
            continue;
        }

        if let Ok(relative) = path.strip_prefix(skill_dir) {
            files.push(relative.to_path_buf());
        } else {
            files.push(path.to_path_buf());
        }

        if files.len() >= MAX_COMPANION_FILES {
            break;
        }
    }

    files.sort();
    files
}

fn is_valid_skill_name(name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() || name.len() > 64 {
        return false;
    }

    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return false;
    }

    name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    })
}
