---
name: skill-creator
description: Create or improve AgentSkills for this coding assistant. Use when asked to create a new skill, package a skill, edit an existing skill, or set up skill infrastructure. Guides designing SKILL.md with proper YAML frontmatter, writing effective prompts, organizing companion files, and testing skills.
---

# Skill Creator

Guide for creating and maintaining AgentSkills.

## Skill Format

Each skill is a directory containing a `SKILL.md` file (required) and optional companion files:

```
.agent/skills/<name>/
├── SKILL.md                  # Required: YAML frontmatter + markdown instructions
├── references/               # Optional: reference docs, examples, guides
│   ├── guide.md
│   └── api-notes.md
└── scripts/                  # Optional: executable scripts
    └── helper.py
```

## SKILL.md Structure

### YAML Frontmatter

```yaml
---
name: my-skill                    # Required: lowercase, hyphens, max 64 chars
description: What this skill does # Required: concise one-liner for the skill tool
---
```

Rules for `name`:
- Lowercase ASCII letters, digits, and hyphens only
- Must not start or end with a hyphen
- Must not contain consecutive hyphens
- Max 64 characters
- Directory name must match skill name exactly

### Body

After the frontmatter, write clear markdown instructions for what the model should do when this skill is invoked. Include:

- **Purpose and trigger conditions** — when should the model use this skill
- **Step-by-step instructions** — what to do
- **Language/framework guidance** — project-specific conventions
- **Examples** — concrete inputs and outputs
- **References to companion files** — where to find detailed guides

Keep instructions actionable and specific. Avoid generic advice that applies to any project.

## Namespace

Bundled skills (built into the binary) use the `tidev-` namespace prefix.
User-created skills should NOT use the `tidev-` prefix to avoid naming conflicts with future built-in skills.

When creating a skill, suggest a name that:
1. Is specific enough to describe its purpose
2. Does not conflict with existing skill names
3. Follows the `kebab-case` convention

## Companion Files

When a skill references companion files, the `skill` tool lists them after the skill content so the model knows they are available. You can also reference companion files explicitly in your instructions.

## Testing Skills

After creating or editing a skill:
1. Verify the SKILL.md is valid (YAML frontmatter parses, `name` follows rules)
2. The directory name matches the skill name exactly
3. Run `/skills` in the TUI or check the skill tool output
4. Invoke the skill with the `skill` tool and verify the prompt looks correct
