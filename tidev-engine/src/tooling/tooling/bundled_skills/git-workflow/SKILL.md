---
name: git-workflow
description: Standard git workflow for everyday development — branching, committing, rebasing, pull requests, reviewing, merging, and changelog generation. Use when asked to perform git operations, prepare a PR, resolve merge conflicts, or generate release notes.
---

# Git Workflow

Standard git operations for any project.

## Branching

- **Feature branches**: `feat/<short-description>` or `<username>/<feature-name>`
- **Bug fixes**: `fix/<issue-number>-<description>`
- **Chores/refactoring**: `chore/<description>`
- Base feature/fix branches off the default branch (`main`, `master`, `dev`)

## Committing

Write clear, conventional commit messages:

```
<type>: <short summary>                 (subject, max 72 chars)

<body>                                   (optional, wrap at 72 chars)
```

Common types: `feat`, `fix`, `chore`, `refactor`, `docs`, `test`, `style`, `perf`, `ci`

Guidelines:
- One logical change per commit
- The subject line should complete "This commit will..." (e.g., "fix: handle empty input gracefully")
- Use imperative mood ("add" not "added" or "adds")
- Reference issue numbers when applicable: `fix #123: ...`

## Pull Requests

### Before Opening

- Rebase onto the latest target branch: `git rebase origin/main`
- Resolve any merge conflicts locally
- Run tests: build, lint, typecheck, test
- Review your own diff before submitting
- Ensure each commit compiles and passes tests (bisect-friendly)

### PR Description

```
## Summary
What this change does and why.

## Changes
- Bullet list of key changes
- Focus on what is notable, not every file touched

## Testing
How the change was tested: specific commands, scenarios, edge cases.

## Breaking Changes
List any breaking changes and migration steps, or "None."
```

## Code Review

- Respond to review comments promptly
- Each comment should get a reply or a fix
- After addressing feedback, re-request review
- Keep conversations focused on the code

## Merging

### Strategies

- **Squash merge**: For feature branches with many small/pending commits. Produces one clean commit.
- **Rebase merge**: When you want to preserve individual commit history with linear history.
- **Merge commit**: For collaborative branches where multiple people committed.

### After Merge

- Delete the feature branch (remote and local)
- Update any related issues

## Changelog Generation

To generate changelog from git history between two references:

```
git log --oneline <from-tag>..HEAD --no-decorate
```

Categorize changes by type (feat, fix, chore, etc.) using the commit message prefixes. Group breaking changes separately with a "⚠️ Breaking Changes" header.

## Handling Merge Conflicts

1. `git merge <branch>` or `git rebase <branch>` will stop at conflicts
2. Use `git status` to find conflicted files
3. Open each file and resolve conflict markers (`<<<<<<<`, `=======`, `>>>>>>>`)
4. `git add <resolved-file>` to mark as resolved
5. `git rebase --continue` or `git merge --continue`
6. Verify the build still works after resolution
