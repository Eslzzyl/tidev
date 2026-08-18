//! Bundled skills that ship with the tidev binary.
//!
//! These SKILL.md files are compiled into the binary at build time via
//! `include_str!` and registered into the skill catalog at startup.
//! They are always available without requiring any filesystem setup.

use std::path::PathBuf;

use crate::skills::SkillInfo;

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
        skill_from_str(include_str!("powershell/SKILL.md"), "powershell"),
    ]
}

fn skill_from_str(content: &'static str, dir_name: &str) -> SkillInfo {
    // Normalize CRLF to LF to handle Windows line endings (git may convert
    // to CRLF on checkout).  This mirrors what parse_skill_content does.
    let content = content.replace("\r\n", "\n");
    let (name, description, body) = crate::skills::parse_frontmatter(&content)
        .expect("bundled SKILL.md must have valid YAML frontmatter");
    let body = body.trim().to_string();

    SkillInfo {
        name,
        description,
        directory: PathBuf::from(format!("__builtin__/{}", dir_name)),
        location: PathBuf::from(format!("__builtin__/{}/SKILL.md", dir_name)),
        document: content,
        content: body,
        companion_files: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills;

    #[test]
    fn test_bundled_skills_load() {
        let skills = load();
        assert_eq!(skills.len(), 5, "expected 5 bundled skills");

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
                skill.document.starts_with("---\n"),
                "document should start with frontmatter"
            );
            assert!(
                !skill.content.starts_with("---\n"),
                "content should contain only the document body"
            );
        }
    }

    #[test]
    fn test_parse_frontmatter_valid() {
        let content = "---\nname: test-skill\ndescription: A test skill\n---\n\nBody text";
        let (name, desc, body) = skills::parse_frontmatter(content).expect("valid frontmatter");
        assert_eq!(name, "test-skill");
        assert_eq!(desc, "A test skill");
        assert_eq!(body.trim(), "Body text");
    }

    #[test]
    fn test_parse_frontmatter_missing() {
        assert!(skills::parse_frontmatter("no frontmatter").is_err());
        assert!(skills::parse_frontmatter("---\nname: only\n---\nno description").is_err());
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
