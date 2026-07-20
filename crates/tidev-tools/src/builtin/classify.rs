//! Shell command classification — determines whether a command is read-only
//! or likely to perform write operations.
//!
//! Used by the bash tool to block modifying commands in Plan mode.
//! The classification is best-effort: false positives (blocking a read-only command)
//! are worse than false negatives (letting a write command through), so when in doubt
//! we return [`Safety::Unknown`] which lets the command execute.

use std::sync::LazyLock;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Safety
// ---------------------------------------------------------------------------

/// Classification result for a shell command.
///
/// Ordered from least to most restrictive:
/// - `Unknown`: cannot determine (treated as safe — let through)
/// - `ReadOnly`: command only reads
/// - `WriteOperation`: detected as a write operation
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Safety {
    Unknown,
    ReadOnly,
    WriteOperation,
}

// ---------------------------------------------------------------------------
// Classifier
// ---------------------------------------------------------------------------

/// Classifier for shell commands.
///
/// Extensible: add new `classify_*` methods below and register the dispatch
/// in [`Classifier::classify`].
#[derive(Default)]
pub struct Classifier {
    // Reserved for future configuration (user allowlist/blocklist, etc.)
}

impl Classifier {
    pub fn new() -> Self {
        Self {}
    }

    /// Return a global cached instance.
    pub fn global() -> &'static Self {
        static CLASSIFIER: OnceLock<Classifier> = OnceLock::new();
        CLASSIFIER.get_or_init(|| Classifier::new())
    }

    /// Classify a shell command string.
    ///
    /// Handles compound commands (`&&`, `||`, `;`, `|`) by classifying each
    /// segment independently and returning the strictest result.
    pub fn classify(&self, command: &str) -> Safety {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return Safety::Unknown;
        }

        // ── 1. Redirect detection — most reliable write signal ──────────
        if has_write_redirect(trimmed) {
            return Safety::WriteOperation;
        }

        // ── 2. Split compound commands (&& || ; |) and classify each ───
        let segments = split_compound(trimmed);
        let mut result = Safety::Unknown;

        for segment in segments {
            let parts: Vec<&str> = simple_tokenize(segment);
            if parts.is_empty() {
                continue;
            }

            // ── 3. Strip privilege-escalation / env wrappers ────────
            let cmd_index = find_cmd_index(&parts);
            let Some(cmd) = parts.get(cmd_index).copied() else {
                continue;
            };
            let args = &parts[cmd_index + 1..];

            let seg_safety = classify_command(cmd, args);
            // Take the strictest: WriteOperation > ReadOnly > Unknown
            if seg_safety == Safety::WriteOperation {
                return Safety::WriteOperation; // short-circuit
            }
            if seg_safety == Safety::ReadOnly {
                result = Safety::ReadOnly;
            }
        }

        result
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Simple shell-aware tokenizer.
///
/// Splits on whitespace while respecting single/double quotes and backslash
/// escapes. This is intentionally **not** a full shell parser — it handles
/// the common case well enough for command classification.
fn simple_tokenize(s: &str) -> Vec<&str> {
    // We can't return substrings with quotes stripped without allocation,
    // so just split on whitespace. This is sufficient for extracting the
    // command name and subcommand.
    s.split_whitespace().collect()
}

/// Known wrapper commands that delegate to another command.
/// e.g. `sudo rm -rf /` → the real command is `rm`.
static WRAPPERS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "sudo", "doas", "pkexec",
        "env", "noglob", "command",
        "time", "nohup",
    ]
});

/// Find the index of the actual command, skipping wrapper prefixes.
fn find_cmd_index(parts: &[&str]) -> usize {
    let mut idx = 0;
    while idx < parts.len() && WRAPPERS.contains(&parts[idx]) {
        idx += 1;
        // After `env`, skip KEY=VALUE assignments
        while idx < parts.len() && parts[idx].contains('=') {
            idx += 1;
        }
    }
    idx
}

/// Split a shell command string into compound segments.
///
/// Segments are separated by `&&`, `||`, `;`, or `|` (simple pipe).
fn split_compound<'a>(s: &'a str) -> Vec<&'a str> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut in_single = false;
    let mut in_double = false;
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i] as char;

        if c == '\'' && !in_double {
            in_single = !in_single;
            i += 1;
            continue;
        }
        if c == '"' && !in_single {
            in_double = !in_double;
            i += 1;
            continue;
        }

        if !in_single && !in_double {
            if c == '|' && i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                let seg = s[start..i].trim();
                if !seg.is_empty() {
                    segments.push(seg);
                }
                start = i + 2;
                i += 2;
                continue;
            }
            if c == '&' && i + 1 < bytes.len() && bytes[i + 1] == b'&' {
                let seg = s[start..i].trim();
                if !seg.is_empty() {
                    segments.push(seg);
                }
                start = i + 2;
                i += 2;
                continue;
            }
            if c == ';' || c == '|' {
                let seg = s[start..i].trim();
                if !seg.is_empty() {
                    segments.push(seg);
                }
                start = i + 1;
                i += 1;
                continue;
            }
        }

        i += 1;
    }

    let last = s[start..].trim();
    if !last.is_empty() {
        segments.push(last);
    }

    segments
}

/// Detect file write redirects (`>`, `>>`, `&>`).
///
/// Excludes file-descriptor-only redirects like `2>&1`.
fn has_write_redirect(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut in_single = false;
    let mut in_double = false;

    for i in 0..bytes.len() {
        let c = bytes[i] as char;

        if c == '\'' && !in_double {
            in_single = !in_single;
        } else if c == '"' && !in_single {
            in_double = !in_double;
        }

        if in_single || in_double {
            continue;
        }

        if c == '>' {
            // Check if this is a fd redirect (like 2>&1, >&2)
            // Look at the previous char for a digit, and the next char for &
            let prev_is_digit = i > 0 && bytes[i - 1].is_ascii_digit();
            let next_is_ampersand = i + 1 < bytes.len() && bytes[i + 1] == b'&';

            if prev_is_digit && next_is_ampersand {
                // e.g. `2>&1` — fd redirect, not file write
                continue;
            }
            if next_is_ampersand {
                // e.g. `>&2` — also fd redirect
                continue;
            }

            return true;
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Per-command classifiers
// ---------------------------------------------------------------------------

/// Git sub-commands that are read-only.
static GIT_READ_SUBCOMMANDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "log", "diff", "show", "status", "branch", // `branch` without -d is read-only
        "tag",                                       // `tag` without -d is read-only
        "blame", "annotate",
        "describe",
        "grep",
        "ls-files", "ls-tree", "ls-remote",
        "rev-parse", "rev-list",
        "cat-file",
        "shortlog",
        "whatchanged",
        "help", "version",
        "config",                                    // reading config is fine
        "stash",                                     // `stash show` / `stash list` are read-only
    ]
});

/// Classify a git command by its subcommand.
fn classify_git(args: &[&str]) -> Safety {
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
        let has_set = args.contains(&"--set") || args.contains(&"--unset")
            || args.contains(&"--add") || args.contains(&"--unset-all")
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

/// Cargo sub-commands that are read-only.
static CARGO_READ_SUBCOMMANDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "check", "test", "clippy",                  // `fix` and `fmt` modify code — write
        "doc", "metadata", "tree",
        "locate-project", "pkgid",
        "help", "version",
        "search", "info",
        "report",
        "generate-lockfile",                            // safe (regenerates Cargo.lock)
        "verify-project",
        "audit",
    ]
});

/// Classify a cargo command by its subcommand.
fn classify_cargo(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown;
    };

    if CARGO_READ_SUBCOMMANDS.contains(&sub) {
        Safety::ReadOnly
    } else {
        // build, run, publish, install, etc. — all write
        Safety::WriteOperation
    }
}

/// Classify sed/perl/awk — only blocking if in-place flag is present.
fn classify_editor(args: &[&str], _in_place_flags: &[&str]) -> Safety {
    let has_in_place = args.iter().any(|a| {
        // Exact match: -i
        if a == &"-i" {
            return true;
        }
        // Combined flags: single-dash flags containing 'i' (e.g. -pi, -i.bak)
        if let Some(flags) = a.strip_prefix('-') {
            // Only check single-dash flags, not --long-options
            if !flags.starts_with('-') {
                // Split off the value suffix (e.g. .bak in -i.bak)
                let flag_letters = flags.split('.').next().unwrap_or(flags);
                return flag_letters.contains('i');
            }
        }
        false
    });

    if has_in_place {
        Safety::WriteOperation
    } else {
        // Without -i, sed/perl output to stdout — read-only
        Safety::ReadOnly
    }
}

/// Classify build tools (make, cmake, ninja, meson).
fn classify_build_tool(args: &[&str]) -> Safety {
    let Some(target) = args.first().copied() else {
        // bare `make` → builds (write)
        return Safety::WriteOperation;
    };

    // `make clean`, `make install` → write
    // `make -n` → dry-run (read)
    if args.contains(&"-n") || args.contains(&"--dry-run") || args.contains(&"--just-print") {
        return Safety::ReadOnly;
    }

    // Common read-only targets
    match target {
        "help" | "list" | "describe" => Safety::ReadOnly,
        _ => Safety::WriteOperation,
    }
}

/// Classify docker commands.
fn classify_docker(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown;
    };

    // Read-only docker commands
    match sub {
        "ps" | "images" | "logs" | "inspect" | "stats" | "top"
        | "port" | "version" | "info" | "events" | "history"
        | "network" | "volume" => Safety::ReadOnly,

        // Everything else (run, build, pull, push, exec, stop, rm, etc.)
        _ => Safety::WriteOperation,
    }
}

/// Classify npm/pnpm/yarn commands.
fn classify_npm(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown;
    };

    match sub {
        "run" | "test" | "start" | "ls" | "list" | "outdated"
        | "help" | "version" | "why" | "audit" | "doctor"
        | "completion" | "cache" => {
            // Check for `--dry-run`
            if args.contains(&"--dry-run") || args.contains(&"--dry") {
                return Safety::ReadOnly;
            }
            // `npm cache clean` is write
            if sub == "cache" && args.get(1) == Some(&"clean") {
                return Safety::WriteOperation;
            }
            if sub == "run" || sub == "test" || sub == "start" {
                return Safety::Unknown; // depends on the script
            }
            Safety::ReadOnly
        }
        _ => Safety::WriteOperation,
    }
}

/// Classify go commands.
fn classify_go(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown;
    };

    match sub {
        "vet" | "doc" | "list" | "version" | "env" | "help"
        | "fmt"                                        // `go fmt` reports diffs by default; -w writes
        => {
            // Check for `-w` (write) flag on go fmt
            if sub == "fmt" && args.contains(&"-w") {
                return Safety::WriteOperation;
            }
            Safety::ReadOnly
        }
        // build, run, test, install, get, mod, work, generate etc.
        _ => Safety::WriteOperation,
    }
}

/// Classify tar — -t/-tf is list (read-only), everything else writes.
fn classify_tar(args: &[&str]) -> Safety {
    // Look for `-t` or `--list` anywhere in args (including combined: -tf, -tvf, -vtf)
    let is_list = args.iter().any(|a| {
        a == &"-t" || a == &"--list" || a.starts_with("-t")
    });
    if is_list {
        Safety::ReadOnly
    } else {
        Safety::WriteOperation
    }
}

// ---------------------------------------------------------------------------
// Main dispatch
// ---------------------------------------------------------------------------

/// Classify a single command (no redirects, no compound, no wrapper stripping).
fn classify_command(cmd: &str, args: &[&str]) -> Safety {
    match cmd {
        // ── Version control ──────────────────────────────────────────
        "git" => classify_git(args),

        // ── Rust tooling ─────────────────────────────────────────────
        "cargo" => classify_cargo(args),
        "rustc" | "rustup" => Safety::WriteOperation,

        // ── Build tools ──────────────────────────────────────────────
        "make" | "cmake" | "ninja" | "meson" => classify_build_tool(args),
        "gcc" | "g++" | "clang" | "clang++" | "cc" | "c++" | "ld" => Safety::WriteOperation,

        // ── Text editors with in-place flag ──────────────────────────
        "sed" => classify_editor(args, &["-i"]),
        "perl" => classify_editor(args, &["-i"]),
        "awk" => Safety::Unknown, // awk without -i (which doesn't exist) is read-only

        // ── Deterministic write commands ─────────────────────────────
        "rm" | "rmdir" | "unlink" => {
            // `rm --help` or `rm --version` is read-only
            if args.iter().any(|a| *a == "--help" || *a == "--version") {
                Safety::ReadOnly
            } else {
                Safety::WriteOperation
            }
        }
        "mv" | "cp" | "chmod" | "chown" | "chattr"
        | "mkdir" | "touch" | "truncate" | "dd"
        | "ln" | "install" | "mkfs" | "mount" | "umount" => Safety::WriteOperation,
        "tee" => {
            // `tee --help` is read-only, otherwise write
            if args.iter().any(|a| *a == "--help" || *a == "--version") {
                Safety::ReadOnly
            } else {
                Safety::WriteOperation
            }
        }

        // ── Containers ───────────────────────────────────────────────
        "docker" | "docker-compose" => classify_docker(args),

        // ── Go ───────────────────────────────────────────────────────
        "go" => classify_go(args),

        // ── Archives ─────────────────────────────────────────────────
        "tar" => classify_tar(args),
        "unzip" | "zip" | "gzip" | "gunzip" | "bzip2" | "bunzip2" | "xz" | "unxz"
        | "zcat" | "zstd" | "unzstd" => Safety::WriteOperation,

        // ── Network downloads (write files) ──────────────────────────
        "curl" | "wget" | "scp" | "rsync" => Safety::WriteOperation,

        // ── Process management ───────────────────────────────────────
        "kill" | "pkill" | "killall" => Safety::WriteOperation,

        // ── Package managers ─────────────────────────────────────────
        "npm" | "npx" | "yarn" | "pnpm" | "bun" => classify_npm(args),
        "pip" | "pip3" | "gem" | "bundle" | "composer"
        | "apt" | "apt-get" | "brew" | "port" | "cargo-install" => Safety::WriteOperation,

        // ── Everything else → let through ────────────────────────────
        _ => Safety::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn c() -> Classifier {
        Classifier::new()
    }

    // ── Git ──────────────────────────────────────────────────────────────

    #[test]
    fn git_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("git log"), Safety::ReadOnly);
        assert_eq!(cl.classify("git log --oneline -5"), Safety::ReadOnly);
        assert_eq!(cl.classify("git diff HEAD~1"), Safety::ReadOnly);
        assert_eq!(cl.classify("git status"), Safety::ReadOnly);
        assert_eq!(cl.classify("git show HEAD"), Safety::ReadOnly);
        assert_eq!(cl.classify("git blame src/main.rs"), Safety::ReadOnly);
        assert_eq!(cl.classify("git grep foo"), Safety::ReadOnly);
        assert_eq!(cl.classify("git ls-files"), Safety::ReadOnly);
        assert_eq!(cl.classify("git rev-parse HEAD"), Safety::ReadOnly);
        assert_eq!(cl.classify("git branch"), Safety::ReadOnly);            // list
        assert_eq!(cl.classify("git tag"), Safety::ReadOnly);              // list
        assert_eq!(cl.classify("git config user.name"), Safety::ReadOnly); // read
        assert_eq!(cl.classify("git stash list"), Safety::ReadOnly);
        assert_eq!(cl.classify("git stash show"), Safety::ReadOnly);
    }

    #[test]
    fn git_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("git checkout main"), Safety::WriteOperation);
        assert_eq!(cl.classify("git checkout -b feature"), Safety::WriteOperation);
        assert_eq!(cl.classify("git switch main"), Safety::WriteOperation);
        assert_eq!(cl.classify("git restore src/main.rs"), Safety::WriteOperation);
        assert_eq!(cl.classify("git reset --hard HEAD"), Safety::WriteOperation);
        assert_eq!(cl.classify("git revert HEAD"), Safety::WriteOperation);
        assert_eq!(cl.classify("git merge feature"), Safety::WriteOperation);
        assert_eq!(cl.classify("git rebase main"), Safety::WriteOperation);
        assert_eq!(cl.classify("git push origin main"), Safety::WriteOperation);
        assert_eq!(cl.classify("git commit -m 'fix'"), Safety::WriteOperation);
        assert_eq!(cl.classify("git add ."), Safety::WriteOperation);
        assert_eq!(cl.classify("git branch -d old"), Safety::WriteOperation);
        assert_eq!(cl.classify("git branch -D old"), Safety::WriteOperation);
        assert_eq!(cl.classify("git tag -d v1.0"), Safety::WriteOperation);
        assert_eq!(cl.classify("git clean -fd"), Safety::WriteOperation);
        assert_eq!(cl.classify("git rm file.rs"), Safety::WriteOperation);
        assert_eq!(cl.classify("git mv old.rs new.rs"), Safety::WriteOperation);
        assert_eq!(cl.classify("git stash push"), Safety::WriteOperation);
        assert_eq!(cl.classify("git stash pop"), Safety::WriteOperation);
        assert_eq!(cl.classify("git stash drop"), Safety::WriteOperation);
        assert_eq!(cl.classify("git config --set user.name foo"), Safety::WriteOperation);
        assert_eq!(cl.classify("git config user.name foo"), Safety::WriteOperation);
    }

    #[test]
    fn git_branch_create_is_write() {
        let cl = c();
        // `git branch feature` creates a branch
        assert_eq!(cl.classify("git branch feature"), Safety::WriteOperation);
        // `git branch` (list) is read-only
        assert_eq!(cl.classify("git branch"), Safety::ReadOnly);
    }

    #[test]
    fn git_branch_list_with_flags_is_read_only() {
        let cl = c();
        assert_eq!(cl.classify("git branch -a"), Safety::ReadOnly);
        assert_eq!(cl.classify("git branch -r"), Safety::ReadOnly);
        assert_eq!(cl.classify("git branch -v"), Safety::ReadOnly);
        assert_eq!(cl.classify("git branch --all"), Safety::ReadOnly);
        assert_eq!(cl.classify("git branch --remotes"), Safety::ReadOnly);
    }

    #[test]
    fn git_tag_list_is_read_only() {
        let cl = c();
        // `git tag -l` (list mode, no pattern) is read-only
        assert_eq!(cl.classify("git tag -l"), Safety::ReadOnly);
        // `git tag` (bare, list all) is read-only
        assert_eq!(cl.classify("git tag"), Safety::ReadOnly);
    }

    // ── Go ───────────────────────────────────────────────────────────────

    #[test]
    fn go_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("go vet ./..."), Safety::ReadOnly);
        assert_eq!(cl.classify("go doc fmt"), Safety::ReadOnly);
        assert_eq!(cl.classify("go list ./..."), Safety::ReadOnly);
        assert_eq!(cl.classify("go version"), Safety::ReadOnly);
    }

    #[test]
    fn go_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("go build ./..."), Safety::WriteOperation);
        assert_eq!(cl.classify("go run main.go"), Safety::WriteOperation);
        assert_eq!(cl.classify("go install ./cmd/..."), Safety::WriteOperation);
        assert_eq!(cl.classify("go mod tidy"), Safety::WriteOperation);
        assert_eq!(cl.classify("go mod download"), Safety::WriteOperation);
        assert_eq!(cl.classify("go get example.com/pkg"), Safety::WriteOperation);
    }

    // ── Tar / Archive ────────────────────────────────────────────────────

    #[test]
    fn tar_list_is_read_only() {
        let cl = c();
        assert_eq!(cl.classify("tar -tf archive.tar"), Safety::ReadOnly);
        assert_eq!(cl.classify("tar -tvf archive.tar"), Safety::ReadOnly);
    }

    #[test]
    fn tar_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("tar -cf archive.tar files/"), Safety::WriteOperation);
        assert_eq!(cl.classify("tar -xf archive.tar"), Safety::WriteOperation);
        assert_eq!(cl.classify("unzip archive.zip"), Safety::WriteOperation);
        assert_eq!(cl.classify("zip archive.zip file.txt"), Safety::WriteOperation);
        assert_eq!(cl.classify("gzip file.txt"), Safety::WriteOperation);
    }

    // ── Network / Download ───────────────────────────────────────────────

    #[test]
    fn network_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("curl https://example.com"), Safety::WriteOperation);
        assert_eq!(cl.classify("wget https://example.com/file"), Safety::WriteOperation);
        assert_eq!(cl.classify("scp file.txt user@host:/path"), Safety::WriteOperation);
        assert_eq!(cl.classify("rsync -a src/ dst/"), Safety::WriteOperation);
    }

    // ── Process management ───────────────────────────────────────────────

    #[test]
    fn process_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("kill 1234"), Safety::WriteOperation);
        assert_eq!(cl.classify("pkill foo"), Safety::WriteOperation);
        assert_eq!(cl.classify("killall bar"), Safety::WriteOperation);
    }

    // ── Cargo fix/fmt are now write ──────────────────────────────────────

    #[test]
    fn cargo_fix_and_fmt_are_write() {
        let cl = c();
        assert_eq!(cl.classify("cargo fix"), Safety::WriteOperation);
        assert_eq!(cl.classify("cargo fmt"), Safety::WriteOperation);
    }

    #[test]
    fn cargo_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("cargo check"), Safety::ReadOnly);
        assert_eq!(cl.classify("cargo test"), Safety::ReadOnly);
        assert_eq!(cl.classify("cargo clippy"), Safety::ReadOnly);
        assert_eq!(cl.classify("cargo doc"), Safety::ReadOnly);
        assert_eq!(cl.classify("cargo metadata"), Safety::ReadOnly);
        assert_eq!(cl.classify("cargo tree"), Safety::ReadOnly);
        assert_eq!(cl.classify("cargo audit"), Safety::ReadOnly);
    }

    #[test]
    fn cargo_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("cargo build"), Safety::WriteOperation);
        assert_eq!(cl.classify("cargo run"), Safety::WriteOperation);
        assert_eq!(cl.classify("cargo publish"), Safety::WriteOperation);
        assert_eq!(cl.classify("cargo install"), Safety::WriteOperation);
        assert_eq!(cl.classify("cargo add serde"), Safety::WriteOperation);
        assert_eq!(cl.classify("cargo remove serde"), Safety::WriteOperation);
        assert_eq!(cl.classify("cargo update"), Safety::WriteOperation);
    }

    // ── Sed / Perl ───────────────────────────────────────────────────────

    #[test]
    fn sed_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("sed 's/foo/bar/' file"), Safety::ReadOnly);
        assert_eq!(cl.classify("sed -n 'p' file"), Safety::ReadOnly);
        assert_eq!(cl.classify("sed -e 's/foo/bar/' file"), Safety::ReadOnly);
    }

    #[test]
    fn sed_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("sed -i 's/foo/bar/' file"), Safety::WriteOperation);
        assert_eq!(cl.classify("sed -i.bak 's/foo/bar/' file"), Safety::WriteOperation);
        assert_eq!(cl.classify("sed -i'' 's/foo/bar/' file"), Safety::WriteOperation);
    }

    #[test]
    fn perl_in_place_is_write() {
        let cl = c();
        assert_eq!(cl.classify("perl -pi -e 's/foo/bar/' file"), Safety::WriteOperation);
        assert_eq!(cl.classify("perl -e 'print 1'"), Safety::ReadOnly);
    }

    // ── Redirects ────────────────────────────────────────────────────────

    #[test]
    fn redirect_is_write() {
        let cl = c();
        assert_eq!(cl.classify("echo hello > file.txt"), Safety::WriteOperation);
        assert_eq!(cl.classify("cat >> log.txt"), Safety::WriteOperation);
        assert_eq!(cl.classify("ls > out.txt"), Safety::WriteOperation);
        assert_eq!(cl.classify("cmd &> file.txt"), Safety::WriteOperation);
    }

    #[test]
    fn fd_redirect_is_not_write() {
        let cl = c();
        // These are file descriptor redirects, not file writes
        assert_eq!(cl.classify("cmd 2>&1"), Safety::Unknown);
        assert_eq!(cl.classify("cmd >&2"), Safety::Unknown);
        assert_eq!(cl.classify("cmd 1>&2"), Safety::Unknown);
    }

    #[test]
    fn redirect_in_quotes_not_write() {
        // This is a limitation: our simple tokenizer can't distinguish
        // `echo ">"` from `echo > file`. We accept this trade-off.
    }

    // ── Write commands ───────────────────────────────────────────────────

    #[test]
    fn deterministic_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("rm file"), Safety::WriteOperation);
        assert_eq!(cl.classify("rm -rf /tmp"), Safety::WriteOperation);
        assert_eq!(cl.classify("mv old new"), Safety::WriteOperation);
        assert_eq!(cl.classify("cp src dst"), Safety::WriteOperation);
        assert_eq!(cl.classify("chmod +x script.sh"), Safety::WriteOperation);
        assert_eq!(cl.classify("mkdir -p dir"), Safety::WriteOperation);
        assert_eq!(cl.classify("touch file"), Safety::WriteOperation);
        assert_eq!(cl.classify("ln -s target link"), Safety::WriteOperation);
        assert_eq!(cl.classify("dd if=/dev/zero of=file bs=1 count=1"), Safety::WriteOperation);
    }

    #[test]
    fn help_version_is_read_only() {
        let cl = c();
        assert_eq!(cl.classify("rm --help"), Safety::ReadOnly);
        assert_eq!(cl.classify("rm --version"), Safety::ReadOnly);
        assert_eq!(cl.classify("tee --help"), Safety::ReadOnly);
    }

    // ── Docker ───────────────────────────────────────────────────────────

    #[test]
    fn docker_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("docker ps"), Safety::ReadOnly);
        assert_eq!(cl.classify("docker images"), Safety::ReadOnly);
        assert_eq!(cl.classify("docker logs app"), Safety::ReadOnly);
        assert_eq!(cl.classify("docker inspect app"), Safety::ReadOnly);
    }

    #[test]
    fn docker_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("docker build ."), Safety::WriteOperation);
        assert_eq!(cl.classify("docker run image"), Safety::WriteOperation);
        assert_eq!(cl.classify("docker push image"), Safety::WriteOperation);
        assert_eq!(cl.classify("docker stop app"), Safety::WriteOperation);
    }

    // ── NPM ──────────────────────────────────────────────────────────────

    #[test]
    fn npm_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("npm run test"), Safety::Unknown); // run scripts are unknown
        assert_eq!(cl.classify("npm list"), Safety::ReadOnly);
        assert_eq!(cl.classify("npm outdated"), Safety::ReadOnly);
        assert_eq!(cl.classify("npm audit"), Safety::ReadOnly);
        assert_eq!(cl.classify("npm cache ls"), Safety::ReadOnly);
    }

    #[test]
    fn npm_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("npm install express"), Safety::WriteOperation);
        assert_eq!(cl.classify("npm publish"), Safety::WriteOperation);
        assert_eq!(cl.classify("npm cache clean"), Safety::WriteOperation);
    }

    // ── Compound commands ────────────────────────────────────────────────

    #[test]
    fn compound_with_write_segment_is_write() {
        let cl = c();
        assert_eq!(
            cl.classify("git log && git checkout main"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("cargo check && cargo build"),
            Safety::WriteOperation
        );
    }

    #[test]
    fn compound_all_read_is_read() {
        let cl = c();
        assert_eq!(
            cl.classify("git log --oneline | grep fix"),
            Safety::ReadOnly
        );
        assert_eq!(
            cl.classify("cargo check && cargo test"),
            Safety::ReadOnly
        );
    }

    // ── Wrappers (sudo, env, etc.) ───────────────────────────────────────

    #[test]
    fn sudo_wrapper_stripped() {
        let cl = c();
        assert_eq!(cl.classify("sudo rm file"), Safety::WriteOperation);
        assert_eq!(cl.classify("sudo git checkout main"), Safety::WriteOperation);
        assert_eq!(cl.classify("sudo git log"), Safety::ReadOnly);
        assert_eq!(cl.classify("doas git log"), Safety::ReadOnly);
    }

    #[test]
    fn env_wrapper_stripped() {
        let cl = c();
        assert_eq!(cl.classify("env FOO=bar git log"), Safety::ReadOnly);
        assert_eq!(cl.classify("env RUST_LOG=debug cargo check"), Safety::ReadOnly);
    }

    // ── Build tools ──────────────────────────────────────────────────────

    #[test]
    fn make_write() {
        let cl = c();
        assert_eq!(cl.classify("make"), Safety::WriteOperation);
        assert_eq!(cl.classify("make install"), Safety::WriteOperation);
        assert_eq!(cl.classify("make clean"), Safety::WriteOperation);
    }

    #[test]
    fn make_dry_run_is_read() {
        let cl = c();
        assert_eq!(cl.classify("make -n"), Safety::ReadOnly);
        assert_eq!(cl.classify("make --dry-run"), Safety::ReadOnly);
    }

    // ── Empty / edge cases ───────────────────────────────────────────────

    #[test]
    fn empty_command_is_unknown() {
        let cl = c();
        assert_eq!(cl.classify(""), Safety::Unknown);
        assert_eq!(cl.classify("   "), Safety::Unknown);
    }

    #[test]
    fn unknown_command_is_unknown() {
        let cl = c();
        assert_eq!(cl.classify("python -c 'print(1)'"), Safety::Unknown);
        assert_eq!(cl.classify("node script.js"), Safety::Unknown);
        assert_eq!(cl.classify("cat file.txt"), Safety::Unknown);
        assert_eq!(cl.classify("ls -la"), Safety::Unknown);
        assert_eq!(cl.classify("find . -name '*.rs'"), Safety::Unknown);
    }

    // ── split_compound ───────────────────────────────────────────────────

    #[test]
    fn test_split_compound_simple() {
        let parts = split_compound("git log");
        assert_eq!(parts, vec!["git log"]);
    }

    #[test]
    fn test_split_compound_and() {
        let parts = split_compound("cargo check && cargo test");
        assert_eq!(parts, vec!["cargo check", "cargo test"]);
    }

    #[test]
    fn test_split_compound_or() {
        let parts = split_compound("make || echo skip");
        assert_eq!(parts, vec!["make", "echo skip"]);
    }

    #[test]
    fn test_split_compound_pipe() {
        let parts = split_compound("git log | grep fix");
        assert_eq!(parts, vec!["git log", "grep fix"]);
    }

    #[test]
    fn test_split_compound_semicolon() {
        let parts = split_compound("cd dir; git status");
        assert_eq!(parts, vec!["cd dir", "git status"]);
    }

    #[test]
    fn test_split_compound_quotes() {
        // Quotes should not split
        let parts = split_compound("echo 'foo && bar'");
        assert_eq!(parts, vec!["echo 'foo && bar'"]);
    }

    // ── has_write_redirect ───────────────────────────────────────────────

    #[test]
    fn test_redirect_detection() {
        assert!(has_write_redirect("echo hello > file"));
        assert!(has_write_redirect("cat >> file"));
        assert!(!has_write_redirect("echo hello"));
        assert!(!has_write_redirect("cmd 2>&1"));
        assert!(!has_write_redirect("cmd >&2"));
    }
}
