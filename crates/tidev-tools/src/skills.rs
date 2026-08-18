use anyhow::{Context, Result, bail};
use ignore::WalkBuilder;
use std::sync::Arc;
use std::time::Duration;
use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
};

use reqwest::blocking::Client;

use tidev_utils::path::{canonicalize_display, canonicalize_for_comparison};

const SKILL_FILE_NAME: &str = "SKILL.md";
const SKILL_ROOTS: &[&str] = &[".opencode/skills", ".claude/skills", ".agents/skills"];
/// Home-directory skill roots, always scanned as default global locations
/// regardless of configuration.
const HOME_SKILL_ROOTS: &[&str] = &[".agents/skills", ".claude/skills"];
const MAX_COMPANION_FILES: usize = 10;
/// Default page size for listing skills through the `skill` tool.
pub const DEFAULT_SKILL_PAGE_SIZE: usize = 20;
/// Upper bound for a single skill listing page.
pub const MAX_SKILL_PAGE_SIZE: usize = 100;
/// Static description for the `skill` tool. The live skill catalog is never
/// embedded here: it is injected into the system prompt once at session
/// creation (see [`SkillCatalog::catalog_section`]) and can be re-listed on
/// demand by calling the tool without a name.
pub const SKILL_TOOL_DESCRIPTION: &str = "Load a reusable skill or read a file inside a skill's directory. Call with no arguments to \
     list available skills (optional offset/limit paginate the list). Call with `name` to load \
     the skill's main document (SKILL.md). Call with `name` and `path` (relative to the skill's \
     directory) to read a companion file such as a doc or script. Paths are confined to the \
     skill's own directory.";

#[derive(Clone, Debug)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub directory: PathBuf,
    pub location: PathBuf,
    /// The complete normalized SKILL.md document, including frontmatter.
    pub document: String,
    /// The body of SKILL.md after frontmatter.
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
    home_dir: Option<&Path>,
    extra_sources: &[String],
    worktree: Option<&Path>,
) -> SkillCatalogInner {
    log::debug!(
        "discover_inner: start, home_dir={:?}, worktree={:?}",
        home_dir.map(|p| p.display().to_string()),
        worktree.map(|p| p.display().to_string())
    );
    let mut skills = Vec::new();
    let mut seen_names = HashSet::new();
    let mut seen_locations = HashSet::new();

    let roots = candidate_roots(workspace_root, config_dir, home_dir, worktree);
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
            "SkillCatalog::discover: workspace_root={}, config_dir={}, home_dir={:?}, skill_sources={:?}, SKILL_ROOTS={:?}, worktree={:?}",
            workspace_root.display(),
            config_dir.display(),
            dirs::home_dir().map(|p| p.display().to_string()),
            skill_sources,
            SKILL_ROOTS,
            worktree.map(|p| p.display().to_string())
        );
        let start = std::time::Instant::now();
        let inner = discover_inner(
            workspace_root,
            config_dir,
            dirs::home_dir().as_deref(),
            skill_sources,
            worktree,
        );
        log::info!(
            "SkillCatalog::discover: catalog initialized with {} skills in {:?}",
            inner.skills.len(),
            start.elapsed()
        );
        Self {
            inner: Arc::new(inner),
        }
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

    /// The "Available skills" block injected into the system prompt once at
    /// session creation. The block is persisted with the session and stays
    /// byte-identical for its entire lifetime, so it never participates in
    /// per-turn re-composition. The live catalog can still be queried through
    /// [`SkillCatalog::list_skills`].
    pub fn catalog_section(&self) -> String {
        let mut section = String::from("Available skills:");
        if self.inner.skills.is_empty() {
            section.push_str(" none discovered.");
            return section;
        }
        for skill in &self.inner.skills {
            section.push_str("\n- ");
            section.push_str(&skill.name);
            section.push_str(": ");
            section.push_str(skill.description.trim());
        }
        section
    }

    /// List the live skill catalog as a paginated text page. `offset` is
    /// 1-based; `limit` is clamped to [`MAX_SKILL_PAGE_SIZE`]. The returned
    /// text carries a footer pointing at the next page when more remain.
    pub fn list_skills(&self, offset: usize, limit: usize) -> Result<String> {
        let total = self.inner.skills.len();
        let offset = offset.max(1);
        let limit = limit.clamp(1, MAX_SKILL_PAGE_SIZE);
        let start = offset - 1;
        let end = start.saturating_add(limit).min(total);

        if start >= total {
            return Ok(format!(
                "Available skills: no skills at offset {offset} (total {total})."
            ));
        }

        let mut output = format!(
            "Available skills (showing {}-{} of {}):\n",
            offset, end, total
        );
        for skill in &self.inner.skills[start..end] {
            output.push_str(&format!("- {}: {}\n", skill.name, skill.description.trim()));
        }
        if end < total {
            output.push_str(&format!(
                "(use the skill tool with offset={} to list the next page)",
                end + 1
            ));
        }

        Ok(output.trim_end().to_string())
    }

    /// Read a file inside a skill's directory, or list a directory when
    /// `relative_path` points at one. The path must stay within the skill
    /// directory: absolute paths, `..` traversal, and symlink escapes are
    /// rejected so this tool never reads outside the skill's own files.
    ///
    /// `max_output_bytes` bounds the returned text.
    pub fn read_skill_file(
        &self,
        name: &str,
        relative_path: &str,
        max_output_bytes: usize,
    ) -> Result<String> {
        let skill = self
            .get(name)
            .with_context(|| format!("unknown skill '{name}'"))?;
        let normalized_path = normalize_skill_relative_path(relative_path)?;

        if normalized_path == Path::new(SKILL_FILE_NAME) {
            let mut document = skill.document.clone();
            crate::builtin::utils::truncate_in_place(&mut document, max_output_bytes);
            return Ok(document);
        }

        let resolved = resolve_skill_relative_path(&skill.directory, relative_path)?;

        if resolved.is_dir() {
            return list_skill_directory(relative_path, &resolved);
        }

        if normalized_path.as_os_str().is_empty() {
            return list_virtual_skill_directory(skill);
        }

        if !resolved.is_file() {
            bail!(
                "failed to read {} in skill '{}': file not found",
                relative_path,
                name
            );
        }

        let bytes = fs::read(&resolved)
            .with_context(|| format!("failed to read {} in skill '{name}'", relative_path))?;

        // Mirror the generic read tool: reject binary content.
        if bytes.iter().take(1024).any(|&byte| byte == 0) {
            bail!("Cannot read binary file: {relative_path}");
        }

        let document = tidev_utils::encoding::decode_text(&bytes)
            .with_context(|| format!("failed to decode {} in skill '{name}'", relative_path))?;
        let mut text = document.into_text();
        crate::builtin::utils::truncate_in_place(&mut text, max_output_bytes);
        Ok(text)
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
                output.push_str(&file.display().to_string().replace('\\', "/"));
                output.push('\n');
            }
            output.push_str(&format!(
                "\nRead a companion file by calling the skill tool with name \"{}\" and path set \
                 to its path relative to the skill directory (e.g. \"docs/guide.md\").",
                skill.name
            ));
        }

        Ok(output)
    }
}

fn normalize_skill_relative_path(relative_path: &str) -> Result<PathBuf> {
    let path = Path::new(relative_path);
    if path.as_os_str().is_empty() {
        bail!("path must not be empty");
    }
    if path.is_absolute() {
        bail!("path must be relative to the skill directory");
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                bail!("path '{relative_path}' escapes the skill directory")
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("path must be relative to the skill directory")
            }
            Component::Normal(value) => normalized.push(value),
        }
    }

    Ok(normalized)
}

/// Resolve a skill-relative path against the skill directory, rejecting any
/// path that escapes it: absolute paths, `.`/`..` segments are checked at the
/// component level, and the canonicalized result must remain inside the
/// canonicalized skill directory (which also blocks symlink escapes).
fn resolve_skill_relative_path(skill_dir: &Path, relative_path: &str) -> Result<PathBuf> {
    let path = normalize_skill_relative_path(relative_path)?;

    let resolved = skill_dir.join(path);
    let canonical_dir = canonicalize_for_comparison(skill_dir);
    let canonical_resolved = canonicalize_for_comparison(&resolved);
    if !canonical_resolved.starts_with(&canonical_dir) {
        bail!("path '{relative_path}' escapes the skill directory");
    }
    Ok(resolved)
}

fn list_virtual_skill_directory(skill: &SkillInfo) -> Result<String> {
    let mut entries = vec![SKILL_FILE_NAME.to_string()];
    entries.extend(
        skill
            .companion_files
            .iter()
            .map(|path| path.display().to_string().replace('\\', "/")),
    );
    entries.sort();
    entries.dedup();
    Ok(format!("./\n{}", entries.join("\n")))
}

/// Render a directory listing inside a skill, mirroring the generic read
/// tool's format but with the header shown relative to the skill directory.
fn list_skill_directory(relative_path: &str, resolved: &Path) -> Result<String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(resolved)
        .with_context(|| format!("failed to read directory {relative_path}"))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {relative_path}"))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        let mut name = entry.file_name().to_string_lossy().to_string();
        if file_type.is_dir() {
            name.push('/');
        }
        entries.push(name);
    }

    entries.sort();

    if entries.is_empty() {
        return Ok("(empty)".to_string());
    }
    let header = relative_path.replace('\\', "/");
    Ok(format!("{header}/\n{}", entries.join("\n")))
}

fn candidate_roots(
    workspace_root: &Path,
    config_dir: &Path,
    home_dir: Option<&Path>,
    worktree: Option<&Path>,
) -> Vec<PathBuf> {
    log::debug!(
        "candidate_roots: workspace_root={}, config_dir={}, home_dir={:?}, worktree={:?}",
        workspace_root.display(),
        config_dir.display(),
        home_dir.map(|p| p.display().to_string()),
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

    // Default global roots in the user's home directory, always scanned
    // regardless of configuration. Dedup via canonical path handles the case
    // where the workspace (or one of its ancestors) is the home directory
    // itself, in which case the ancestor walk already found these roots.
    if let Some(home) = home_dir {
        for root in HOME_SKILL_ROOTS {
            let candidate = home.join(root);
            log::debug!(
                "candidate_roots: checking home global root={}",
                candidate.display()
            );
            if !candidate.is_dir() {
                log::debug!("candidate_roots: not a directory, skip");
                continue;
            }

            let canonical = canonicalize_display(&candidate);
            if seen.insert(canonical.clone()) {
                log::info!(
                    "candidate_roots: found home global skill directory: {}",
                    canonical.display()
                );
                roots.push(canonical);
            } else {
                log::debug!("candidate_roots: already seen, skip");
            }
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
    let bytes = fs::read(path).map_err(|e| {
        log::debug!("parse_skill_file: read error for {}: {}", path.display(), e);
    })?;
    let raw_content = tidev_utils::encoding::decode_text(&bytes)
        .map(|document| document.into_text())
        .map_err(|e| {
            log::debug!(
                "parse_skill_file: decode error for {}: {}",
                path.display(),
                e
            );
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
    let body = body.trim().to_string();

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
        document: normalized_content,
        content: body,
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
    let bytes = fs::read(&resolved).ok()?;
    let content = tidev_utils::encoding::decode_text(&bytes)
        .ok()
        .map(|document| document.into_text())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(root: &Path, name: &str) {
        fs::create_dir_all(root.join(name)).unwrap();
        fs::write(
            root.join(name).join(SKILL_FILE_NAME),
            format!("---\nname: {name}\ndescription: test skill {name}\n---\nBody of {name}.\n"),
        )
        .unwrap();
    }

    #[test]
    fn discovers_home_global_roots_outside_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("work");
        fs::create_dir_all(&workspace).unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let config_dir = home.join(".config").join("tidev");
        fs::create_dir_all(&config_dir).unwrap();

        write_skill(&home.join(".agents").join("skills"), "agents-skill");
        write_skill(&home.join(".claude").join("skills"), "claude-skill");

        let catalog = discover_inner(&workspace, &config_dir, Some(&home), &[], None);
        let names: Vec<&str> = catalog.skills.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"agents-skill"),
            "expected agents-skill in {names:?}"
        );
        assert!(
            names.contains(&"claude-skill"),
            "expected claude-skill in {names:?}"
        );
    }

    #[test]
    fn home_global_roots_deduplicated_when_workspace_is_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let config_dir = home.join(".config").join("tidev");
        fs::create_dir_all(&config_dir).unwrap();

        // Workspace is the home directory itself: the ancestor walk already
        // finds .agents/skills and .claude/skills, so the explicit home roots
        // must dedup rather than double-load.
        let workspace = home.clone();
        write_skill(&home.join(".agents").join("skills"), "agents-skill");
        write_skill(&home.join(".claude").join("skills"), "claude-skill");

        let catalog = discover_inner(&workspace, &config_dir, Some(&home), &[], None);
        let agents: Vec<&SkillInfo> = catalog
            .skills
            .iter()
            .filter(|s| s.name == "agents-skill")
            .collect();
        let claude: Vec<&SkillInfo> = catalog
            .skills
            .iter()
            .filter(|s| s.name == "claude-skill")
            .collect();
        assert_eq!(agents.len(), 1);
        assert_eq!(claude.len(), 1);
    }

    fn skill_info(name: &str, dir: &Path) -> SkillInfo {
        SkillInfo {
            name: name.to_string(),
            description: format!("test skill {name}"),
            directory: dir.to_path_buf(),
            location: dir.join(SKILL_FILE_NAME),
            document: format!(
                "---\nname: {name}\ndescription: test skill {name}\n---\n\nBody of {name}."
            ),
            content: format!("Body of {name}."),
            companion_files: Vec::new(),
        }
    }

    fn catalog_from_skills(skills: Vec<SkillInfo>) -> SkillCatalog {
        SkillCatalog {
            inner: Arc::new(SkillCatalogInner { skills }),
        }
    }

    #[test]
    fn catalog_section_lists_skills_or_reports_none() {
        assert_eq!(
            SkillCatalog::default().catalog_section(),
            "Available skills: none discovered."
        );

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        let catalog =
            catalog_from_skills(vec![skill_info("demo", &dir), skill_info("alpha", &dir)]);
        let section = catalog.catalog_section();
        assert_eq!(
            section,
            "Available skills:\n- demo: test skill demo\n- alpha: test skill alpha"
        );
    }

    #[test]
    fn list_skills_paginates_the_live_catalog() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        let skills: Vec<SkillInfo> = (0..5)
            .map(|i| skill_info(&format!("skill-{i}"), &dir))
            .collect();
        let catalog = catalog_from_skills(skills);

        let page1 = catalog.list_skills(1, 2).unwrap();
        assert!(page1.contains("showing 1-2 of 5"));
        assert!(page1.contains("- skill-0: test skill skill-0"));
        assert!(page1.contains("- skill-1:"));
        assert!(page1.contains("offset=3"));

        let page2 = catalog.list_skills(3, 2).unwrap();
        assert!(page2.contains("showing 3-4 of 5"));

        let page3 = catalog.list_skills(5, 2).unwrap();
        assert!(page3.contains("showing 5-5 of 5"));
        assert!(!page3.contains("offset="));

        let beyond = catalog.list_skills(99, 2).unwrap();
        assert!(beyond.contains("no skills at offset 99"));
    }

    #[test]
    fn read_skill_file_reads_inside_the_skill_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        fs::create_dir_all(dir.join("docs")).unwrap();
        fs::write(dir.join("docs").join("guide.md"), "# Guide\n\nHello").unwrap();
        let catalog = catalog_from_skills(vec![skill_info("demo", &dir)]);

        let content = catalog
            .read_skill_file("demo", "docs/guide.md", 1024)
            .unwrap();
        assert_eq!(content, "# Guide\n\nHello");

        let err = catalog.read_skill_file("unknown", "docs/guide.md", 1024);
        assert!(err.is_err());
    }

    #[test]
    fn read_skill_file_reads_the_main_document_for_virtual_skills() {
        let skill = crate::bundled_skills::load()
            .into_iter()
            .find(|skill| skill.name == "git-workflow")
            .unwrap();
        let catalog = catalog_from_skills(vec![skill]);

        let document = catalog
            .read_skill_file("git-workflow", "SKILL.md", 1024 * 1024)
            .unwrap();
        assert!(document.starts_with("---\n"));
        assert!(document.contains("# Git Workflow"));

        let rendered = catalog.render_skill("git-workflow").unwrap();
        assert!(rendered.starts_with("# Skill: git-workflow\n\nLocation: "));
        assert!(rendered.contains("# Git Workflow"));
        assert!(!rendered.contains("---\nname: git-workflow"));

        let listing = catalog
            .read_skill_file("git-workflow", ".", 1024 * 1024)
            .unwrap();
        assert_eq!(listing, "./\nSKILL.md");
    }

    #[test]
    fn read_skill_file_lists_skill_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        fs::create_dir_all(dir.join("docs")).unwrap();
        fs::create_dir_all(dir.join("scripts")).unwrap();
        fs::write(dir.join("docs").join("guide.md"), "x").unwrap();
        fs::write(dir.join("scripts").join("run.sh"), "#!/bin/sh\n").unwrap();
        let catalog = catalog_from_skills(vec![skill_info("demo", &dir)]);

        let listing = catalog.read_skill_file("demo", ".", 1024).unwrap();
        assert!(listing.contains("docs/"), "listing was: {listing}");
        assert!(listing.contains("scripts/"), "listing was: {listing}");

        let docs = catalog.read_skill_file("demo", "docs", 1024).unwrap();
        assert!(docs.contains("docs/\n"), "listing was: {docs}");
        assert!(docs.contains("guide.md"), "listing was: {docs}");
    }

    #[test]
    fn read_skill_file_rejects_escaping_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        fs::create_dir_all(&dir).unwrap();
        let catalog = catalog_from_skills(vec![skill_info("demo", &dir)]);

        assert!(catalog.read_skill_file("demo", "", 1024).is_err());
        assert!(catalog.read_skill_file("demo", "..", 1024).is_err());
        assert!(
            catalog
                .read_skill_file("demo", "../secret.txt", 1024)
                .is_err()
        );
        assert!(catalog.read_skill_file("demo", "a/../../b", 1024).is_err());

        let absolute = dir.join("SKILL.md");
        assert!(
            catalog
                .read_skill_file("demo", absolute.to_str().unwrap(), 1024)
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_skill_file_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside.txt");
        fs::write(&outside, "secret").unwrap();
        let dir = tmp.path().join("demo");
        fs::create_dir_all(&dir).unwrap();
        symlink(&outside, dir.join("link.txt")).unwrap();
        let catalog = catalog_from_skills(vec![skill_info("demo", &dir)]);

        let err = catalog.read_skill_file("demo", "link.txt", 1024);
        assert!(err.is_err(), "symlink escape must be rejected");
    }

    #[test]
    fn discover_creates_a_catalog_for_each_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace_one = tmp.path().join("workspace-one");
        let workspace_two = tmp.path().join("workspace-two");
        let config_one = tmp.path().join("config-one");
        let config_two = tmp.path().join("config-two");
        fs::create_dir_all(&workspace_one).unwrap();
        fs::create_dir_all(&workspace_two).unwrap();
        fs::create_dir_all(&config_one).unwrap();
        fs::create_dir_all(&config_two).unwrap();
        write_skill(
            &workspace_one.join(".agents").join("skills"),
            "workspace-one-skill",
        );
        write_skill(
            &workspace_two.join(".agents").join("skills"),
            "workspace-two-skill",
        );

        let catalog_one = SkillCatalog::discover(&workspace_one, &config_one, &[], None);
        let catalog_two = SkillCatalog::discover(&workspace_two, &config_two, &[], None);

        assert!(catalog_one.get("workspace-one-skill").is_some());
        assert!(catalog_two.get("workspace-two-skill").is_some());
    }
}
