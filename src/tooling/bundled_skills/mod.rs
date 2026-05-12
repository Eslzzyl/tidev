//! Bundled skills that ship with the tidev binary.
//!
//! These SKILL.md files are compiled into the binary at build time via
//! `include_str!` and registered into the skill catalog at startup.
//! They are always available without requiring any filesystem setup.

use std::path::PathBuf;

use crate::tooling::skills::SkillInfo;

/// Load all bundled skills.
///
/// Each skill's SKILL.md is embedded at compile time. The `name` and
/// `description` are extracted from the YAML frontmatter.  A special
/// `__builtin__` prefix is used for the location and directory fields
/// so it is clear these are compiled-in skills, not filesystem ones.
pub fn load() -> Vec<SkillInfo> {
    vec![
        skill_from_str(include_str!("skill-creator/SKILL.md"), "skill-creator"),
        skill_from_str(include_str!("code-review/SKILL.md"), "code-review"),
        skill_from_str(include_str!("debug/SKILL.md"), "debug"),
        skill_from_str(include_str!("git-workflow/SKILL.md"), "git-workflow"),
    ]
}

fn skill_from_str(content: &'static str, dir_name: &str) -> SkillInfo {
    let (name, description) =
        parse_frontmatter(content).expect("bundled SKILL.md must have valid YAML frontmatter");

    SkillInfo {
        name,
        description,
        directory: PathBuf::from(format!("__builtin__/{}", dir_name)),
        location: PathBuf::from(format!("__builtin__/{}/SKILL.md", dir_name)),
        content: content.to_string(),
        companion_files: Vec::new(),
    }
}

/// Minimal YAML frontmatter parser.
///
/// Extracts only the `name` and `description` fields from the `---` delimited
/// block at the start of a SKILL.md file.  This avoids a full YAML dependency
/// for bundled skills while keeping the data in a human-readable file.
fn parse_frontmatter(content: &str) -> Option<(String, String)> {
    let body = content.strip_prefix("---\n")?;
    let (frontmatter, _rest) = body.split_once("\n---\n")?;

    let mut name: Option<String> = None;
    let mut description: Option<String> = None;

    for line in frontmatter.lines() {
        if let Some(stripped) = line.strip_prefix("name: ") {
            name = Some(stripped.trim().to_string());
        } else if let Some(stripped) = line.strip_prefix("description: ") {
            description = Some(stripped.trim().to_string());
        }
    }

    Some((name?, description?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tooling::skills;

    #[test]
    fn test_bundled_skills_load() {
        let skills = load();
        assert_eq!(skills.len(), 4, "expected 4 bundled skills");

        for skill in &skills {
            assert!(!skill.name.is_empty(), "name must not be empty");
            assert!(
                !skill.description.is_empty(),
                "description must not be empty"
            );
            assert!(
                skill.location.starts_with("__builtin__"),
                "location should start with __builtin__"
            );
            assert!(!skill.content.is_empty(), "content must not be empty");
            assert!(
                skill.content.starts_with("---\n"),
                "content should start with frontmatter"
            );
        }
    }

    #[test]
    fn test_parse_frontmatter_valid() {
        let content = "---\nname: test-skill\ndescription: A test skill\n---\n\nBody text";
        let (name, desc) = parse_frontmatter(content).expect("valid frontmatter");
        assert_eq!(name, "test-skill");
        assert_eq!(desc, "A test skill");
    }

    #[test]
    fn test_parse_frontmatter_missing() {
        assert!(parse_frontmatter("no frontmatter").is_none());
        assert!(parse_frontmatter("---\nname: only\n---\nno description").is_none());
    }

    #[test]
    fn test_bundled_skill_names_are_valid() {
        let skills = load();
        for skill in &skills {
            assert!(
                skills::is_valid_skill_name(&skill.name),
                "bundled skill name '{}' is not valid",
                skill.name
            );
        }
    }
}
