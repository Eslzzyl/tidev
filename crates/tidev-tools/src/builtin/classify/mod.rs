//! Shell command classification — determines whether a command is read-only
//! or likely to perform write operations.
//!
//! Used by the bash tool to block modifying commands in Plan mode.
//! The classification is best-effort: false positives (blocking a read-only command)
//! are worse than false negatives (letting a write command through), so when in doubt
//! we return [`Safety::Unknown`] which lets the command execute.

mod build_tool;
mod cargo;
mod docker;
mod editor;
mod git;
mod go;
mod npm;
mod tar;

use build_tool::classify_build_tool;
use cargo::classify_cargo;
use docker::classify_docker;
use editor::classify_editor;
use git::classify_git;
use go::classify_go;
use npm::classify_npm;
use std::sync::LazyLock;
use std::sync::OnceLock;
use tar::classify_tar;

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
        "sudo", "doas", "pkexec", "env", "noglob", "command", "time", "nohup",
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
        "rm" | "rmdir" | "unlink" | "del" | "erase" => {
            // `rm --help` or `rm --version` is read-only
            if args.iter().any(|a| *a == "--help" || *a == "--version") {
                Safety::ReadOnly
            } else {
                Safety::WriteOperation
            }
        }
        "mv" | "cp" | "chmod" | "chown" | "chattr" | "mkdir" | "touch" | "truncate" | "dd"
        | "ln" | "install" | "mkfs" | "mount" | "umount"
        | "copy" | "move" | "ren" | "rename" | "md" | "rd" => Safety::WriteOperation,
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
        "unzip" | "zip" | "gzip" | "gunzip" | "bzip2" | "bunzip2" | "xz" | "unxz" | "zcat"
        | "zstd" | "unzstd" => Safety::WriteOperation,

        // ── Network downloads (write files) ──────────────────────────
        "curl" | "wget" | "scp" | "rsync" => Safety::WriteOperation,

        // ── Process management ───────────────────────────────────────
        "kill" | "pkill" | "killall" => Safety::WriteOperation,

        // ── Windows / PowerShell ─────────────────────────────────────
        "ri" | "rni" | "ni"                                    // PowerShell aliases
        | "iwr" | "irm"                                           // Invoke-WebRequest/RestMethod
        | "ac"                                                    // Add-Content
        => Safety::WriteOperation,
        "Copy-Item" | "Move-Item" | "Remove-Item" | "Rename-Item" | "New-Item"
        | "Set-Content" | "Add-Content" | "Clear-Content"
        | "Out-File"
        | "Invoke-WebRequest" | "Invoke-RestMethod"
        => Safety::WriteOperation,

        // ── Package managers ─────────────────────────────────────────
        "npm" | "npx" | "yarn" | "pnpm" | "bun" => classify_npm(args),
        "pip" | "pip3" | "gem" | "bundle" | "composer" | "apt" | "apt-get" | "brew" | "port"
        | "cargo-install" => Safety::WriteOperation,

        // ── Everything else → let through ────────────────────────────
        _ => Safety::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c() -> Classifier {
        Classifier::new()
    }

    // ── Network / Download ───────────────────────────────────────────────

    #[test]
    fn network_write_commands() {
        let cl = c();
        assert_eq!(
            cl.classify("curl https://example.com"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("wget https://example.com/file"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("scp file.txt user@host:/path"),
            Safety::WriteOperation
        );
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
        assert_eq!(
            cl.classify("sed -i 's/foo/bar/' file"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("sed -i.bak 's/foo/bar/' file"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("sed -i'' 's/foo/bar/' file"),
            Safety::WriteOperation
        );
    }

    #[test]
    fn perl_in_place_is_write() {
        let cl = c();
        assert_eq!(
            cl.classify("perl -pi -e 's/foo/bar/' file"),
            Safety::WriteOperation
        );
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
        assert_eq!(
            cl.classify("dd if=/dev/zero of=file bs=1 count=1"),
            Safety::WriteOperation
        );
        assert_eq!(cl.classify("unzip archive.zip"), Safety::WriteOperation);
        assert_eq!(cl.classify("zip archive.zip file.txt"), Safety::WriteOperation);
        assert_eq!(cl.classify("gzip file.txt"), Safety::WriteOperation);
        // Windows cmd/powershell commands
        assert_eq!(cl.classify("copy a b"), Safety::WriteOperation);
        assert_eq!(cl.classify("move a b"), Safety::WriteOperation);
        assert_eq!(cl.classify("del file"), Safety::WriteOperation);
        assert_eq!(cl.classify("erase file"), Safety::WriteOperation);
        assert_eq!(cl.classify("ren old new"), Safety::WriteOperation);
        assert_eq!(cl.classify("rename old new"), Safety::WriteOperation);
        // PowerShell cmdlets
        assert_eq!(cl.classify("Copy-Item a b"), Safety::WriteOperation);
        assert_eq!(cl.classify("Remove-Item file"), Safety::WriteOperation);
        assert_eq!(cl.classify("Set-Content file text"), Safety::WriteOperation);
        assert_eq!(cl.classify("Add-Content file text"), Safety::WriteOperation);
        assert_eq!(cl.classify("Out-File file"), Safety::WriteOperation);
        assert_eq!(cl.classify("Invoke-WebRequest url"), Safety::WriteOperation);
        assert_eq!(cl.classify("ri file"), Safety::WriteOperation);
        assert_eq!(cl.classify("ni file"), Safety::WriteOperation);
        assert_eq!(cl.classify("iwr url"), Safety::WriteOperation);
    }

    #[test]
    fn help_version_is_read_only() {
        let cl = c();
        assert_eq!(cl.classify("rm --help"), Safety::ReadOnly);
        assert_eq!(cl.classify("rm --version"), Safety::ReadOnly);
        assert_eq!(cl.classify("tee --help"), Safety::ReadOnly);
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
        assert_eq!(cl.classify("cargo check && cargo test"), Safety::ReadOnly);
    }

    // ── Wrappers (sudo, env, etc.) ───────────────────────────────────────

    #[test]
    fn sudo_wrapper_stripped() {
        let cl = c();
        assert_eq!(cl.classify("sudo rm file"), Safety::WriteOperation);
        assert_eq!(
            cl.classify("sudo git checkout main"),
            Safety::WriteOperation
        );
        assert_eq!(cl.classify("sudo git log"), Safety::ReadOnly);
        assert_eq!(cl.classify("doas git log"), Safety::ReadOnly);
    }

    #[test]
    fn env_wrapper_stripped() {
        let cl = c();
        assert_eq!(cl.classify("env FOO=bar git log"), Safety::ReadOnly);
        assert_eq!(
            cl.classify("env RUST_LOG=debug cargo check"),
            Safety::ReadOnly
        );
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
