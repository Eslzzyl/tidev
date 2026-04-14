use anyhow::{Context, Result};
use ignore::WalkBuilder;
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use reqwest::blocking::Client;
use std::time::Duration;

const SKILL_FILE_NAME: &str = "SKILL.md";
const SKILL_ROOTS: &[&str] = &[".opencode/skills", ".claude/skills", ".agents/skills"];
const MAX_COMPANION_FILES: usize = 10;

#[derive(Clone, Debug)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub directory: PathBuf,
    pub location: PathBuf,
    pub content: String,
    pub companion_files: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct SkillCatalog {
    skills: Vec<SkillInfo>,
}

#[derive(Clone, Debug, Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
}

impl SkillCatalog {
    pub fn discover(workspace_root: &Path, config_dir: &Path, extra_sources: &[String]) -> Self {
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

        Self { skills }
    }

    pub fn all(&self) -> &[SkillInfo] {
        &self.skills
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&SkillInfo> {
        self.skills.iter().find(|skill| skill.name == name)
    }

    pub fn tool_description(&self) -> String {
        if self.skills.is_empty() {
            return String::from("Load a reusable skill by name. No skills were discovered.");
        }

        let mut description = String::from("Load a reusable skill by name. Available skills:\n");
        for skill in &self.skills {
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
    parse_skill_content(path.to_path_buf(), Some(path.parent().ok_or(())?.to_path_buf()), raw_content)
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
    parse_skill_content(resolved.clone(), resolved.parent().map(Path::to_path_buf), content).ok()
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::fs;
    use uuid::Uuid;

    fn temp_dir() -> Result<PathBuf> {
        let path = std::env::temp_dir().join(format!("tidev-skills-{}", Uuid::new_v4()));
        fs::create_dir_all(&path)?;
        Ok(path)
    }

    fn write_skill(root: &Path, name: &str, description: &str, body: &str) -> Result<PathBuf> {
        let dir = root.join(name);
        fs::create_dir_all(&dir)?;
        let path = dir.join(SKILL_FILE_NAME);
        fs::write(
            &path,
            format!("---\nname: {name}\ndescription: {description}\n---\n{body}\n"),
        )?;
        Ok(path)
    }

    #[test]
    fn discover_loads_project_and_global_skills() -> Result<()> {
        let workspace = temp_dir()?;
        let config = temp_dir()?;

        let project_root = workspace.join(".opencode").join("skills");
        let global_root = config.join("skills");
        fs::create_dir_all(&project_root)?;
        fs::create_dir_all(&global_root)?;

        write_skill(&project_root, "git-release", "Release helper", "# project")?;
        write_skill(&global_root, "lint-fix", "Lint helper", "# global")?;

        let catalog = SkillCatalog::discover(&workspace, &config, &[]);
        assert_eq!(catalog.all().len(), 2);
        assert_eq!(catalog.all()[0].name, "git-release");
        assert_eq!(catalog.all()[1].name, "lint-fix");
        assert!(catalog.tool_description().contains("git-release"));
        assert!(catalog.tool_description().contains("lint-fix"));

        Ok(())
    }

    #[test]
    fn discover_prefers_first_duplicate_name() -> Result<()> {
        let workspace = temp_dir()?;
        let config = temp_dir()?;

        let project_root = workspace.join(".opencode").join("skills");
        let global_root = config.join("skills");
        fs::create_dir_all(&project_root)?;
        fs::create_dir_all(&global_root)?;

        write_skill(&project_root, "shared", "Project copy", "# project")?;
        write_skill(&global_root, "shared", "Global copy", "# global")?;

        let catalog = SkillCatalog::discover(&workspace, &config, &[]);
        assert_eq!(catalog.all().len(), 1);
        assert_eq!(catalog.all()[0].description, "Project copy");

        Ok(())
    }

    #[test]
    fn render_skill_includes_companion_files() -> Result<()> {
        let workspace = temp_dir()?;
        let config = temp_dir()?;
        let project_root = workspace.join(".opencode").join("skills");
        fs::create_dir_all(&project_root)?;

        let skill_dir = project_root.join("docs-helper");
        fs::create_dir_all(skill_dir.join("snippets"))?;
        fs::write(
            skill_dir.join(SKILL_FILE_NAME),
            "---\nname: docs-helper\ndescription: Docs helper\n---\n# Skill body\n",
        )?;
        fs::write(skill_dir.join("README.md"), "notes")?;
        fs::write(skill_dir.join("snippets").join("one.txt"), "hello")?;

        let catalog = SkillCatalog::discover(&workspace, &config, &[]);
        let rendered = catalog.render_skill("docs-helper")?;

        assert!(rendered.contains("# Skill: docs-helper"));
        assert!(rendered.contains("# Skill body"));
        assert!(rendered.contains("README.md") || rendered.contains("snippets/one.txt"));

        Ok(())
    }

    #[test]
    fn discover_loads_remote_skill_source() -> Result<()> {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let workspace = temp_dir()?;
        let config = temp_dir()?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;

        let server = thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };

            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);

            let body = "---\nname: remote-skill\ndescription: Remote helper\n---\n# Remote body\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        });

        let url = format!("http://{}/skill.md", address);
        let catalog = SkillCatalog::discover(&workspace, &config, &[url]);
        assert_eq!(catalog.all().len(), 1);
        assert_eq!(catalog.all()[0].name, "remote-skill");
        assert!(catalog.all()[0].content.contains("# Remote body"));

        let _ = server.join();
        Ok(())
    }
}