use anyhow::{Context, Result};
use ignore::WalkBuilder;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use reqwest::blocking::Client;

use super::builtin::utils::canonicalize_display;

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

fn discover_inner(
    workspace_root: &Path,
    config_dir: &Path,
    extra_sources: &[String],
    worktree: Option<&Path>,
) -> SkillCatalogInner {
    log::debug!(
        "discover_inner: start, worktree={:?}",
        worktree.map(|p| p.display().to_string())
    );
    let mut skills = Vec::new();
    let mut seen_names = HashSet::new();
    let mut seen_locations = HashSet::new();

    let roots = candidate_roots(workspace_root, config_dir, worktree);
    log::debug!(
        "discover_inner: candidate_roots returned {} roots",
        roots.len()
    );
    for (i, root) in roots.iter().enumerate() {
        log::debug!("discover_inner: root[{}] = {}", i, root.display());
        for skill_file in discover_skill_files(root) {
            log::debug!(
                "discover_inner: found skill_file = {}",
                skill_file.display()
            );
            let canonical_location = canonicalize_display(&skill_file);
            if !seen_locations.insert(canonical_location) {
                log::debug!("discover_inner: duplicate location, skip");
                continue;
            }

            let Ok(skill) = parse_skill_file(&skill_file) else {
                log::debug!(
                    "discover_inner: parse_skill_file failed for {}",
                    skill_file.display()
                );
                continue;
            };

            if !seen_names.insert(skill.name.clone()) {
                log::debug!(
                    "discover_inner: duplicate skill name '{}', skip",
                    skill.name
                );
                continue;
            }

            log::info!(
                "discover_inner: loaded skill '{}' from {}",
                skill.name,
                skill_file.display()
            );
            skills.push(skill);
        }
    }

    for raw_source in extra_sources {
        log::debug!("discover_inner: loading extra source = {}", raw_source);
        let Some(skill) = load_additional_skill_source(raw_source, workspace_root) else {
            log::debug!(
                "discover_inner: load_additional_skill_source returned None for {}",
                raw_source
            );
            continue;
        };

        let canonical_location = canonicalize_display(&skill.location);
        if !seen_locations.insert(canonical_location) {
            log::debug!("discover_inner: duplicate extra location, skip");
            continue;
        }

        if !seen_names.insert(skill.name.clone()) {
            log::debug!(
                "discover_inner: duplicate extra skill name '{}', skip",
                skill.name
            );
            continue;
        }

        log::info!(
            "discover_inner: loaded extra skill '{}' from {}",
            skill.name,
            skill.location.display()
        );
        skills.push(skill);
    }

    // Inject bundled (compiled-in) skills.
    // Disk-based skills take precedence via `seen_names` dedup.
    let bundled_count_before = skills.len();
    for skill in crate::bundled_skills::load() {
        if !seen_names.insert(skill.name.clone()) {
            log::debug!(
                "discover_inner: bundled skill '{}' skipped (duplicate name)",
                skill.name
            );
            continue;
        }
        log::info!("discover_inner: loaded bundled skill '{}'", skill.name);
        skills.push(skill);
    }
    let bundled_count = skills.len() - bundled_count_before;
    if bundled_count > 0 {
        log::info!("discover_inner: loaded {} bundled skill(s)", bundled_count);
    }

    log::info!("discover_inner: done, total skills = {}", skills.len());
    SkillCatalogInner { skills }
}

impl SkillCatalog {
    pub fn discover(
        workspace_root: &Path,
        config_dir: &Path,
        skill_sources: &[String],
        worktree: Option<&Path>,
    ) -> Self {
        log::debug!(
            "SkillCatalog::discover: workspace_root={}, config_dir={}, skill_sources={:?}, SKILL_ROOTS={:?}, worktree={:?}",
            workspace_root.display(),
            config_dir.display(),
            skill_sources,
            SKILL_ROOTS,
            worktree.map(|p| p.display().to_string())
        );
        let inner = CATALOG
            .get_or_init(|| {
                let start = std::time::Instant::now();
                log::info!("SkillCatalog::discover: initializing catalog (first call)");
                let inner = discover_inner(workspace_root, config_dir, skill_sources, worktree);
                log::info!(
                    "SkillCatalog::discover: catalog initialized with {} skills in {:?}",
                    inner.skills.len(),
                    start.elapsed()
                );
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

fn candidate_roots(
    workspace_root: &Path,
    config_dir: &Path,
    worktree: Option<&Path>,
) -> Vec<PathBuf> {
    log::debug!(
        "candidate_roots: workspace_root={}, config_dir={}, worktree={:?}",
        workspace_root.display(),
        config_dir.display(),
        worktree.map(|p| p.display().to_string())
    );
    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    for ancestor in workspace_root.ancestors() {
        // Stop if we've reached the worktree boundary (if specified)
        if let Some(wt) = worktree {
            if ancestor == wt {
                log::debug!(
                    "candidate_roots: reached worktree boundary at {}",
                    ancestor.display()
                );
                // Still check this directory (the worktree root itself)
            } else if !ancestor.starts_with(wt) {
                log::debug!("candidate_roots: passed worktree boundary, stopping traversal");
                break;
            }
        }

        for root in SKILL_ROOTS {
            let candidate = ancestor.join(root);
            log::debug!(
                "candidate_roots: checking candidate={}",
                candidate.display()
            );
            if !candidate.is_dir() {
                log::debug!("candidate_roots: not a directory, skip");
                continue;
            }

            let canonical = canonicalize_display(&candidate);
            if seen.insert(canonical.clone()) {
                log::info!(
                    "candidate_roots: found skill directory: {}",
                    canonical.display()
                );
                roots.push(canonical);
            } else {
                log::debug!("candidate_roots: already seen, skip");
            }
        }

        // Stop after processing the worktree root
        if let Some(wt) = worktree
            && ancestor == wt
        {
            log::debug!("candidate_roots: stopping at worktree root");
            break;
        }
    }

    let global_root = config_dir.join("skills");
    log::debug!(
        "candidate_roots: checking global root={}",
        global_root.display()
    );
    if global_root.is_dir() {
        let canonical = canonicalize_display(&global_root);
        if seen.insert(canonical.clone()) {
            log::info!(
                "candidate_roots: found global skill directory: {}",
                canonical.display()
            );
            roots.push(canonical);
        }
    }

    log::debug!("candidate_roots: returning {} roots", roots.len());
    roots
}

fn discover_skill_files(root: &Path) -> Vec<PathBuf> {
    log::debug!("discover_skill_files: root={}", root.display());
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .build();

    let mut files = Vec::new();
    let mut entry_count = 0u64;
    for entry in walker {
        entry_count += 1;
        let Ok(entry) = entry else {
            log::debug!("discover_skill_files: walk error: {:?}", entry);
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
            log::debug!(
                "discover_skill_files: found SKILL.md at {}",
                entry.path().display()
            );
            files.push(entry.path().to_path_buf());
        }
    }

    log::debug!(
        "discover_skill_files: walked {} entries, found {} SKILL.md files",
        entry_count,
        files.len()
    );
    files.sort();
    files
}

fn parse_skill_file(path: &Path) -> Result<SkillInfo, ()> {
    log::debug!("parse_skill_file: path={}", path.display());
    let raw_content = fs::read_to_string(path).map_err(|e| {
        log::debug!("parse_skill_file: read error for {}: {}", path.display(), e);
    })?;
    let parent = path.parent().ok_or_else(|| {
        log::debug!("parse_skill_file: no parent for {}", path.display());
    })?;
    parse_skill_content(path.to_path_buf(), Some(parent.to_path_buf()), raw_content)
}

fn parse_skill_content(
    location: PathBuf,
    directory: Option<PathBuf>,
    raw_content: String,
) -> Result<SkillInfo, ()> {
    log::debug!("parse_skill_content: location={}", location.display());
    let normalized_content = raw_content.replace("\r\n", "\n");
    let (name, description, body) = parse_frontmatter(&normalized_content).map_err(|e| {
        log::debug!(
            "parse_skill_content: frontmatter parse error for {}: {}",
            location.display(),
            e
        );
    })?;

    if !is_valid_skill_name(&name) {
        log::debug!(
            "parse_skill_content: invalid skill name '{}' in {}",
            name,
            location.display()
        );
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
            .ok_or_else(|| {
                log::debug!(
                    "parse_skill_content: invalid directory name in {}",
                    location.display()
                );
            })?;

        if directory_name != name {
            log::debug!(
                "parse_skill_content: directory name '{}' does not match skill name '{}' in {}",
                directory_name,
                name,
                location.display()
            );
            return Err(());
        }
    }

    log::debug!(
        "parse_skill_content: success name='{}', description='{}', companion_files={}",
        name,
        description,
        companion_files.len()
    );
    Ok(SkillInfo {
        name,
        description,
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

/// Parse YAML frontmatter from a SKILL.md file.
///
/// Extracts `name` and `description` from the `---` delimited block
/// at the start of the content, returning them along with the body
/// text (everything after the frontmatter).
///
/// This avoids pulling in the full `serde_yaml` crate, which is
/// deprecated and no longer maintained.
pub(crate) fn parse_frontmatter(content: &str) -> Result<(String, String, &str), String> {
    let content = content
        .strip_prefix("---\n")
        .ok_or_else(|| "missing opening `---`".to_string())?;
    let (frontmatter, body) = content
        .split_once("\n---\n")
        .ok_or_else(|| "missing closing `---`".to_string())?;

    let mut name: Option<&str> = None;
    let mut description: Option<&str> = None;

    for line in frontmatter.lines() {
        if let Some(stripped) = line.strip_prefix("name: ") {
            name = Some(stripped.trim());
        } else if let Some(stripped) = line.strip_prefix("description: ") {
            description = Some(stripped.trim());
        }
    }

    let name = name.ok_or_else(|| "missing 'name' field".to_string())?;
    let description = description.ok_or_else(|| "missing 'description' field".to_string())?;

    Ok((name.to_string(), description.to_string(), body))
}

fn collect_companion_files(skill_dir: &Path, skill_file: &Path) -> Vec<PathBuf> {
    log::debug!(
        "collect_companion_files: skill_dir={}, skill_file={}",
        skill_dir.display(),
        skill_file.display()
    );
    let walker = WalkBuilder::new(skill_dir)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .build();

    let mut files = Vec::new();
    let mut entry_count = 0u64;
    for entry in walker {
        entry_count += 1;
        let Ok(entry) = entry else {
            log::debug!("collect_companion_files: walk error: {:?}", entry);
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
            log::debug!(
                "collect_companion_files: reached max companion files ({})",
                MAX_COMPANION_FILES
            );
            break;
        }
    }

    files.sort();
    log::debug!(
        "collect_companion_files: walked {} entries, found {} companion files",
        entry_count,
        files.len()
    );
    files
}

pub(crate) fn is_valid_skill_name(name: &str) -> bool {
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
