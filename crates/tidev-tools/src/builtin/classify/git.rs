use super::*;
use std::sync::LazyLock;

/// Git sub-commands that are read-only.
static GIT_READ_SUBCOMMANDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "log",
        "diff",
        "show",
        "status",
        "branch", // `branch` without -d is read-only
        "tag",    // `tag` without -d is read-only
        "blame",
        "annotate",
        "describe",
        "grep",
        "ls-files",
        "ls-tree",
        "ls-remote",
        "rev-parse",
        "rev-list",
        "cat-file",
        "shortlog",
        "whatchanged",
        "help",
        "version",
        "config", // reading config is fine
        "stash",  // `stash show` / `stash list` are read-only
    ]
});

/// Classify a git command by its subcommand.
pub(super) fn classify_git(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        // bare `git` — probably `git status` equivalent?
        // allow through; shell will show help/error
        return Safety::Unknown;
    };

    // `git branch -d` / `git tag -d` are write operations
    if sub == "branch" || sub == "tag" {
        if args.contains(&"-d") || args.contains(&"-D") || args.contains(&"--delete") {
            return Safety::WriteOperation;
        }
        // `git branch` (list) or `git tag` (list) is read-only
        if args.len() == 1 {
            return Safety::ReadOnly;
        }
        // If all extra args start with `-`, it's list mode with flags (e.g. `-a`, `-r`, `-v`)
        // (`git branch -a`, `git tag -l 'v*'`). Otherwise it creates/deletes.
        let has_positional = args.iter().skip(1).any(|a| !a.starts_with('-'));
        if !has_positional {
            return Safety::ReadOnly;
        }
        // `git branch <name>` / `git tag <name>` creates — write
        return Safety::WriteOperation;
    }

    if sub == "stash" {
        let action = args.get(1).copied().unwrap_or("list");
        return if matches!(action, "list" | "show") {
            Safety::ReadOnly
        } else {
            Safety::WriteOperation
        };
    }

    // `git config` — reading is read-only, writing is write
    if sub == "config" {
        let has_set = args.contains(&"--set")
            || args.contains(&"--unset")
            || args.contains(&"--add")
            || args.contains(&"--unset-all")
            || args.contains(&"--replace-all");
        return if has_set {
            Safety::WriteOperation
        } else if args.len() >= 3 {
            // `git config <key> <value>` is writing
            Safety::WriteOperation
        } else {
            Safety::ReadOnly
        };
    }

    if GIT_READ_SUBCOMMANDS.contains(&sub) {
        Safety::ReadOnly
    } else {
        // Unknown subcommand → assume write (safer to block)
        Safety::WriteOperation
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_read_commands() {
        assert_eq!(classify_git(&["log"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["log", "--oneline", "-5"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["diff", "HEAD~1"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["status"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["show", "HEAD"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["blame", "src/main.rs"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["grep", "foo"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["ls-files"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["rev-parse", "HEAD"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["branch"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["tag"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["config", "user.name"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["stash", "list"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["stash", "show"]), Safety::ReadOnly);
    }

    #[test]
    fn git_write_commands() {
        assert_eq!(classify_git(&["checkout", "main"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["checkout", "-b", "feature"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["switch", "main"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["restore", "src/main.rs"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["reset", "--hard", "HEAD"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["revert", "HEAD"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["merge", "feature"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["rebase", "main"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["push", "origin", "main"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["commit", "-m", "fix"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["add", "."]), Safety::WriteOperation);
        assert_eq!(classify_git(&["branch", "-d", "old"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["branch", "-D", "old"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["tag", "-d", "v1.0"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["clean", "-fd"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["rm", "file.rs"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["mv", "old.rs", "new.rs"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["stash", "push"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["stash", "pop"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["stash", "drop"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["config", "--set", "user.name", "foo"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["config", "user.name", "foo"]), Safety::WriteOperation);
    }

    #[test]
    fn git_branch_create_is_write() {
        assert_eq!(classify_git(&["branch", "feature"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["branch"]), Safety::ReadOnly);
    }

    #[test]
    fn git_branch_list_with_flags_is_read_only() {
        assert_eq!(classify_git(&["branch", "-a"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["branch", "-r"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["branch", "-v"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["branch", "--all"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["branch", "--remotes"]), Safety::ReadOnly);
    }

    #[test]
    fn git_tag_list_is_read_only() {
        assert_eq!(classify_git(&["tag", "-l"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["tag"]), Safety::ReadOnly);
    }
}
