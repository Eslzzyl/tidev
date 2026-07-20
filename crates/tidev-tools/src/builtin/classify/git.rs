use super::*;
use std::sync::LazyLock;

/// Git sub-commands that are always read-only (no sub-subcommand nuance).
static GIT_READ_SUBCOMMANDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        // ── Log / history ──────────────────────────────────────────────
        "log",
        "shortlog",
        "whatchanged",
        "reflog",
        // ── Diff / comparison ──────────────────────────────────────────
        "diff",
        "range-diff",
        "diff-files",
        "diff-index",
        "diff-tree",
        "cherry",
        "merge-base",
        "name-rev",
        "describe",
        // ── Show / inspect ─────────────────────────────────────────────
        "show",
        "blame",
        "annotate",
        "grep",
        "status",
        "help",
        "version",
        // ── List / ls ──────────────────────────────────────────────────
        "ls-files",
        "ls-tree",
        "ls-remote",
        "for-each-ref",
        "for-each-repo",    // iterates over repos, doesn't modify
        "count-objects",
        // ── Ref / revision ─────────────────────────────────────────────
        "rev-parse",
        "rev-list",
        "cat-file",
        // ── Verify / fsck ──────────────────────────────────────────────
        "fsck",
        "verify-commit",
        "verify-tag",
        "verify-pack",
        // ── Config (reading) ───────────────────────────────────────────
        "config",            // reading config is fine; writes handled separately
        // ── Branch/tag list ────────────────────────────────────────────
        "branch",            // list mode is read-only; create/delete handled separately
        "tag",               // list mode is read-only; create/delete handled separately
        // ── Stash list ─────────────────────────────────────────────────
        "stash",             // `stash show` / `stash list` are read-only
        // ── Archive ────────────────────────────────────────────────────
        "archive",           // outputs to stdout; file redirect is caught separately
        // ── Fetch (does NOT modify working tree) ───────────────────────
        "fetch",
        // ── Request / fmt ──────────────────────────────────────────────
        "request-pull",
        "stripspace",        // stdin→stdout
        // ── Check-* ────────────────────────────────────────────────────
        "check-attr",
        "check-ignore",
        "check-mailmap",
        "check-ref-format",
        "checkout-index",    // copies from index to working tree (write-like, but used for read)
        // ── Miscellaneous read-only ────────────────────────────────────
        "column",
        "interpret-trailers",// can be write with --in-place, but redirect catches that
        "hash-object",       // outputs hash; `-w` writes to object store (not checked here)
        "symbolic-ref",      // with `--query` is read; default is write — see classify_git
        "var",
        "get-tar-commit-id",
        "mailinfo",
        "mailsplit",
        "fmt-merge-msg",
        "merge-file",
        "merge-one-file",
        "merge-index",
        "notes",             // `notes list` / `notes show` are read-only
        "worktree",          // `worktree list` is read-only
        "submodule",         // `submodule status` / `submodule summary` are read-only
        "remote",            // `remote -v` / `remote show` are read-only
        "maintenance",       // `maintenance start` is write, but `maintenance run` could be either
    ]
});

/// Classify a git command by its subcommand.
pub(super) fn classify_git(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown;
    };

    // ── branch / tag: list vs. create/delete ──────────────────────────────
    if sub == "branch" || sub == "tag" {
        return classify_branch_tag(args);
    }

    // ── stash: list/show vs. push/pop/drop/apply ─────────────────────────
    if sub == "stash" {
        return classify_stash(args);
    }

    // ── config: reading vs. setting ───────────────────────────────────────
    if sub == "config" {
        return classify_config(args);
    }

    // ── remote: show/list vs. add/remove/set-url ─────────────────────────
    if sub == "remote" {
        return classify_remote(args);
    }

    // ── submodule: status/summary vs. add/update/foreach ─────────────────
    if sub == "submodule" {
        return classify_submodule(args);
    }

    // ── worktree: list vs. add/remove/lock/unlock ────────────────────────
    if sub == "worktree" {
        return classify_worktree(args);
    }

    // ── notes: list/show vs. add/append/edit/remove/... ──────────────────
    if sub == "notes" {
        return classify_notes(args);
    }

    // ── maintenance: run vs. start/stop/register/unregister ──────────────
    if sub == "maintenance" {
        return classify_maintenance(args);
    }

    // ── hash-object: without -w is read, with -w is write ────────────────
    if sub == "hash-object" {
        if args.contains(&"-w") || args.contains(&"--stdin") {
            return Safety::WriteOperation;
        }
        return Safety::ReadOnly;
    }

    // ── symbolic-ref: without args is write (creates/updates), with --query is read
    if sub == "symbolic-ref" {
        if args.contains(&"--query") || args.contains(&"--short") || args.len() == 1 {
            return Safety::ReadOnly;
        }
        return Safety::WriteOperation;
    }

    // ── interpret-trailers: --in-place is write ──────────────────────────
    if sub == "interpret-trailers" {
        if args.contains(&"--in-place") {
            return Safety::WriteOperation;
        }
        return Safety::ReadOnly;
    }

    // ── Explicit write subcommands (caught by default, but listing for clarity)
    // checkout, switch, restore, reset, revert, merge, rebase, pull,
    // push, commit, add, clean, rm, mv, cherry-pick, bisect, gc, prune,
    // update-ref, update-index, write-tree, read-tree, mktag, mktree,
    // commit-tree, replace, filter-branch, worktree add/remove,
    // submodule add/update, notes add/remove, maintenance start/stop,
    // etc. — all fall through to the default WriteOperation below.

    if GIT_READ_SUBCOMMANDS.contains(&sub) {
        Safety::ReadOnly
    } else {
        // Unknown subcommand → assume write (safer to block)
        Safety::WriteOperation
    }
}

// ---------------------------------------------------------------------------
// Sub-classifiers
// ---------------------------------------------------------------------------

/// `git branch` / `git tag`: list mode vs. create/delete.
fn classify_branch_tag(args: &[&str]) -> Safety {
    if args.contains(&"-d") || args.contains(&"-D") || args.contains(&"--delete") {
        return Safety::WriteOperation;
    }
    // `git branch` (list) or `git tag` (list) is read-only
    if args.len() == 1 {
        return Safety::ReadOnly;
    }
    // If all extra args start with `-`, it's list mode with flags
    let has_positional = args.iter().skip(1).any(|a| !a.starts_with('-'));
    if !has_positional {
        return Safety::ReadOnly;
    }
    // `git branch <name>` / `git tag <name>` creates — write
    Safety::WriteOperation
}

/// `git stash`: list/show are read-only; everything else is write.
fn classify_stash(args: &[&str]) -> Safety {
    let action = args.get(1).copied().unwrap_or("list");
    if matches!(action, "list" | "show") {
        Safety::ReadOnly
    } else {
        Safety::WriteOperation
    }
}

/// `git config`: reading vs. setting.
fn classify_config(args: &[&str]) -> Safety {
    let has_set = args.contains(&"--set")
        || args.contains(&"--unset")
        || args.contains(&"--add")
        || args.contains(&"--unset-all")
        || args.contains(&"--replace-all");
    if has_set {
        return Safety::WriteOperation;
    }
    if args.len() >= 3 {
        // `git config <key> <value>` is writing
        Safety::WriteOperation
    } else {
        Safety::ReadOnly
    }
}

/// `git remote`: read-only subcommands vs. write.
fn classify_remote(args: &[&str]) -> Safety {
    let Some(action) = args.get(1).copied() else {
        // `git remote` — list remotes
        return Safety::ReadOnly;
    };

    // `git remote -v` / `git remote --verbose` — list with URLs
    if action.starts_with('-') {
        return Safety::ReadOnly;
    }

    match action {
        "show" | "get-url" | "get-head" => Safety::ReadOnly,
        _ => Safety::WriteOperation, // add, rename, remove, set-url, set-head, prune, update
    }
}

/// `git submodule`: read-only vs. write.
fn classify_submodule(args: &[&str]) -> Safety {
    let Some(action) = args.get(1).copied() else {
        // `git submodule` — show status (default action)
        return Safety::ReadOnly;
    };
    match action {
        "status" | "summary" | "foreach" => Safety::ReadOnly,
        _ => Safety::WriteOperation, // add, update, init, deinit, absorbgitdirs, sync
    }
}

/// `git worktree`: list is read-only; add/remove/lock/unlock are write.
fn classify_worktree(args: &[&str]) -> Safety {
    let Some(action) = args.get(1).copied() else {
        // `git worktree` without args — list
        return Safety::ReadOnly;
    };
    match action {
        "list" | "prune" | "lock" | "unlock" => Safety::ReadOnly,
        _ => Safety::WriteOperation, // add, remove, move, repair
    }
}

/// `git notes`: list/show are read-only; add/append/edit/remove/... are write.
fn classify_notes(args: &[&str]) -> Safety {
    let Some(action) = args.get(1).copied() else {
        // `git notes` — list notes
        return Safety::ReadOnly;
    };
    match action {
        "list" | "show" | "get-ref" => Safety::ReadOnly,
        _ => Safety::WriteOperation, // add, append, edit, remove, prune, merge, copy
    }
}

/// `git maintenance`: run is read-only (safe), start/stop are write.
fn classify_maintenance(args: &[&str]) -> Safety {
    let Some(action) = args.get(1).copied() else {
        // `git maintenance` — show help
        return Safety::Unknown;
    };
    match action {
        // `run` performs maintenance tasks but doesn't change config
        "run" => Safety::ReadOnly,
        _ => Safety::WriteOperation, // start, stop, register, unregister
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Read-only git commands (general) ──────────────────────────────────

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
    fn git_new_read_subcommands() {
        // Fetch is safe — does not modify working tree
        assert_eq!(classify_git(&["fetch"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["fetch", "origin"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["fetch", "--all"]), Safety::ReadOnly);
        // Archive outputs to stdout by default
        assert_eq!(classify_git(&["archive", "HEAD"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["archive", "--format=zip", "HEAD"]), Safety::ReadOnly);
        // Query subcommands
        assert_eq!(classify_git(&["merge-base", "HEAD", "HEAD~1"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["cherry", "main"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["name-rev", "HEAD"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["reflog"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["count-objects"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["fsck"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["verify-commit", "HEAD"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["verify-tag", "v1.0"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["shortlog"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["whatchanged"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["describe"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["range-diff", "HEAD~1", "HEAD"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["for-each-ref"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["request-pull", "url", "HEAD"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["stripspace"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["check-ignore", "file"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["verify-pack", "pack.idx"]), Safety::ReadOnly);
    }

    // ── Write git commands ────────────────────────────────────────────────

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
        assert_eq!(classify_git(&["pull", "origin", "main"]), Safety::WriteOperation);
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
    fn git_writes_new() {
        assert_eq!(classify_git(&["cherry-pick", "abc123"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["bisect", "start"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["gc"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["prune"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["update-ref", "HEAD", "abc"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["replace", "abc", "def"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["notes", "add", "-m", "note"]), Safety::WriteOperation);
    }

    // ── Branch/tag ───────────────────────────────────────────────────────

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

    // ── Remote ───────────────────────────────────────────────────────────

    #[test]
    fn git_remote_read_commands() {
        assert_eq!(classify_git(&["remote"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["remote", "-v"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["remote", "show", "origin"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["remote", "get-url", "origin"]), Safety::ReadOnly);
    }

    #[test]
    fn git_remote_write_commands() {
        assert_eq!(classify_git(&["remote", "add", "origin", "url"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["remote", "remove", "origin"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["remote", "rename", "old", "new"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["remote", "set-url", "origin", "url"]), Safety::WriteOperation);
    }

    // ── Submodule ────────────────────────────────────────────────────────

    #[test]
    fn git_submodule_read_commands() {
        assert_eq!(classify_git(&["submodule"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["submodule", "status"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["submodule", "summary"]), Safety::ReadOnly);
    }

    #[test]
    fn git_submodule_write_commands() {
        assert_eq!(classify_git(&["submodule", "add", "url"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["submodule", "update"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["submodule", "deinit", "."]), Safety::WriteOperation);
    }

    // ── Worktree ─────────────────────────────────────────────────────────

    #[test]
    fn git_worktree_read_commands() {
        assert_eq!(classify_git(&["worktree"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["worktree", "list"]), Safety::ReadOnly);
    }

    #[test]
    fn git_worktree_write_commands() {
        assert_eq!(classify_git(&["worktree", "add", "../path"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["worktree", "remove", "name"]), Safety::WriteOperation);
    }

    // ── Notes ────────────────────────────────────────────────────────────

    #[test]
    fn git_notes_read_commands() {
        assert_eq!(classify_git(&["notes"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["notes", "list"]), Safety::ReadOnly);
        assert_eq!(classify_git(&["notes", "show", "HEAD"]), Safety::ReadOnly);
    }

    #[test]
    fn git_notes_write_commands() {
        assert_eq!(classify_git(&["notes", "add", "-m", "note"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["notes", "remove"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["notes", "prune"]), Safety::WriteOperation);
    }

    // ── Maintenance ──────────────────────────────────────────────────────

    #[test]
    fn git_maintenance_run_is_read() {
        assert_eq!(classify_git(&["maintenance", "run"]), Safety::ReadOnly);
    }

    #[test]
    fn git_maintenance_start_stop_is_write() {
        assert_eq!(classify_git(&["maintenance", "start"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["maintenance", "stop"]), Safety::WriteOperation);
        assert_eq!(classify_git(&["maintenance", "register"]), Safety::WriteOperation);
    }

    // ── hash-object ──────────────────────────────────────────────────────

    #[test]
    fn git_hash_object_without_w_is_read() {
        assert_eq!(classify_git(&["hash-object", "file.txt"]), Safety::ReadOnly);
    }

    #[test]
    fn git_hash_object_with_w_is_write() {
        assert_eq!(classify_git(&["hash-object", "-w", "file.txt"]), Safety::WriteOperation);
    }
}
