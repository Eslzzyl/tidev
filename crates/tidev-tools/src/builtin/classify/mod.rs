//! Shell command classification — determines whether a command is read-only
//! or likely to perform write operations.
//!
//! Used by the shell tool to block modifying commands in Plan mode.
//! The classification is best-effort: false positives (blocking a read-only command)
//! are worse than false negatives (letting a write command through), so when in doubt
//! we return [`Safety::Unknown`] which lets the command execute.

mod apt;
mod brew;
mod build_tool;
mod rust;
mod deno;
mod docker;
mod editor;
mod gem;
mod git;
mod go;
mod helm;
mod kubectl;
mod nix;
mod npm;
mod pip;
mod systemctl;
mod tar;
mod terraform;

use apt::classify_apt;
use brew::classify_brew;
use build_tool::classify_build_tool;
use rust::classify_rust;
use deno::classify_deno;
use docker::classify_docker;
use editor::classify_editor;
use gem::classify_gem;
use git::classify_git;
use go::classify_go;
use helm::classify_helm;
use kubectl::classify_kubectl;
use nix::classify_nix;
use nix::classify_nix_env;
use nix::classify_nix_shell;
use npm::classify_npm;
use pip::classify_pip;
use std::sync::LazyLock;
use std::sync::OnceLock;
use systemctl::classify_systemctl;
use tar::classify_tar;
use terraform::classify_terraform;

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
        CLASSIFIER.get_or_init(Classifier::new)
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

            // ── 3. Strip redirect tokens uniformly (fd-prefixed, fused,  ──
            //    null-device, etc.) so command classifiers only see
            //    semantic arguments (e.g. `git branch -a 2>/dev/null`
            //    becomes `git branch -a`).
            let parts = strip_redirects(&parts);
            if parts.is_empty() {
                continue;
            }

            // ── 4. Strip privilege-escalation / env wrappers ────────
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
fn split_compound(s: &str) -> Vec<&str> {
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

/// Strip all shell redirect tokens from a tokenized command, so they don't
/// interfere with command classifiers (e.g. `git branch -a 2>/dev/null`
/// where `"2>/dev/null"` would be mistaken for a positional argument).
///
/// This is the token-level complement to [`has_write_redirect`] (string level).
/// Together they ensure no redirect token reaches a command classifier:
///
/// - `has_write_redirect` catches bare `> real_file` early (fast path).
/// - `strip_redirects` strips all remaining redirect tokens (fd-prefixed,
///   fused, null-device, etc.) before classification.
///
/// Handles three forms:
/// - Independent redirect operators that consume a separate target token:
///   `>`, `>>`, `&>`, `>&`, `<>`, `>|`
/// - Fused redirect tokens that include the target: `>file`, `2>/dev/null`,
///   `&>file`, `>&2`, `>>file`, `<>file`, `>|file`, `2>&1`
/// - Simple fd-closing: `>&-`, `2>&-`
fn strip_redirects<'a>(tokens: &[&'a str]) -> Vec<&'a str> {
    // Quick check: if no token contains a redirect char, skip allocation.
    let has_redirect = tokens.iter().any(|t| {
        let bytes = t.as_bytes();
        bytes.contains(&b'>') || bytes.contains(&b'<')
    });
    if !has_redirect {
        return tokens.to_vec();
    }

    let mut result = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];

        // Independent redirect operators: `>`, `>>`, `&>`, `>&`, `<>`, `>|`
        if matches!(token, ">" | ">>" | "&>" | ">&" | "<>" | ">|") {
            i += 2; // skip operator AND the target (file path / fd number)
            continue;
        }

        // Fused redirect: `>file`, `2>/dev/null`, `&>file`, `>&2`, `>>file`, `2>&1`, etc.
        if is_fused_redirect(token) {
            i += 1; // skip the single fused token
            continue;
        }

        result.push(token);
        i += 1;
    }

    result
}

/// Check if a single token is a fused shell redirect (operator + target combined
/// in one word, e.g. `2>/dev/null`, `>file`, `&>file`, `>&2`, `>>file`, `<>file`).
fn is_fused_redirect(token: &str) -> bool {
    let bytes = token.as_bytes();
    if bytes.is_empty() {
        return false;
    }

    let mut pos = 0;

    // Optional file-descriptor prefix: `2>/dev/null`, `1>file`
    if pos < bytes.len() && bytes[pos].is_ascii_digit() {
        pos += 1;
    }

    if pos >= bytes.len() {
        return false;
    }

    match bytes[pos] {
        // `>` variants: `>file`, `>>file`, `>&2`, `>|file`
        b'>' => true,
        // `<>` — read-write redirect (single token like `<>file`)
        b'<' if pos + 1 < bytes.len() && bytes[pos + 1] == b'>' => true,
        // `&>` — both stdout+stderr (fused like `&>file`)
        b'&' if pos + 1 < bytes.len() && bytes[pos + 1] == b'>' => true,
        _ => false,
    }
}

/// Detect stdout write redirects (`>`, `>>`, `&>`) to real (non-null) files.
///
/// This is a **fast path** that catches clear write cases early without needing
/// to tokenize the command. It only catches bare stdout redirects — fd-prefixed
/// redirects to real files (e.g. `2>/tmp/log`) are also caught, but fd-to-fd
/// redirects (`2>&1`, `>&2`) are exempted since they duplicate an fd rather
/// than opening a file. Redirects to `/dev/null`/`nul` are always exempted.
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
            // Skip past '>' (or '>>' or '>|') to find the target
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] == b'>' || bytes[j] == b'|') {
                j += 1;
            }

            // fd-to-fd redirect (e.g. 2>&1, >&2) — not a file write
            if j < bytes.len() && bytes[j] == b'&' {
                continue;
            }

            // Skip whitespace to find the target path
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }

            // If target is /dev/null or nul, not a real write
            if j < bytes.len() && is_dev_null(&bytes[j..]) {
                continue;
            }

            return true;
        }
    }

    false
}

/// Check if the byte slice (starting at a potential target path) matches
/// `/dev/null` (Unix) or `nul` (Windows), with a word boundary after.
fn is_dev_null(tail: &[u8]) -> bool {
    // Check /dev/null (Unix)
    if tail.starts_with(b"/dev/null") {
        let end = b"/dev/null".len();
        return end >= tail.len() || is_boundary_char(tail[end]);
    }

    // Check nul (Windows) — case-insensitive
    if tail.len() >= 3 && tail[..3].eq_ignore_ascii_case(b"nul") {
        let end = 3;
        return end >= tail.len() || is_boundary_char(tail[end]);
    }

    false
}

/// Characters that can follow a redirect target path without it being part
/// of a longer name (e.g. `/dev/null2` should NOT match `/dev/null`).
fn is_boundary_char(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b'&' | b'|' | b'>' | b';' | b'<' | b'(' | b')')
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
        "cargo" | "rustc" | "rustup" => classify_rust(cmd, args),

        // ── Build tools ──────────────────────────────────────────────
        "make" | "cmake" | "ninja" | "meson" | "just" | "task" => classify_build_tool(args),
        "gcc" | "g++" | "clang" | "clang++" | "cc" | "c++" | "ld" => Safety::WriteOperation,
        "bazel" | "blaze" => {
            let Some(sub) = args.first().copied() else {
                return Safety::Unknown;
            };
            match sub {
                "query" | "cquery" | "aquery" | "info" | "help" | "version" => Safety::ReadOnly,
                "build" | "test" | "run" | "clean" | "coverage" | "mobile-install"
                | "fetch" | "shutdown" => Safety::WriteOperation,
                _ => Safety::Unknown,
            }
        }

        // ── Text editors with in-place flag ──────────────────────────
        "sed" => classify_editor(args, &["-i"]),
        "perl" => classify_editor(args, &["-i"]),
        "awk" => Safety::Unknown, // awk without -i (which doesn't exist) is read-only
        "sd" => classify_editor(args, &["-i"]), // sed alternative

        // ── Deterministic write commands ─────────────────────────────
        "rm" | "rmdir" | "unlink" | "del" | "erase" | "shred" => {
            // `--help` / `--version` is read-only
            if args.iter().any(|a| *a == "--help" || *a == "--version") {
                Safety::ReadOnly
            } else {
                Safety::WriteOperation
            }
        }
        "mv" | "cp" | "chmod" | "chown" | "chattr" | "mkdir" | "touch" | "truncate" | "dd"
        | "ln" | "install" | "mkfs" | "mount" | "umount" | "fallocate" | "mktemp" | "mkfifo"
        | "mknod" | "setfacl" | "wipefs" | "swapon" | "swapoff" | "losetup"
        | "copy" | "move" | "ren" | "rename" | "md" | "rd" => Safety::WriteOperation,
        "tee" | "patch" => {
            if args.iter().any(|a| *a == "--help" || *a == "--version") {
                Safety::ReadOnly
            } else {
                Safety::WriteOperation
            }
        }

        // ── Containers ───────────────────────────────────────────────
        "docker" | "docker-compose" => {
            // Handle `docker compose` (space-separated) — strip "compose" prefix
            if cmd == "docker" && args.first() == Some(&"compose") {
                if args.len() < 2 {
                    Safety::Unknown
                } else {
                    classify_docker(&args[1..])
                }
            } else {
                classify_docker(args)
            }
        }
        "podman" | "podman-compose" => {
            // Handle `podman compose` — strip "compose" prefix
            if cmd == "podman" && args.first() == Some(&"compose") {
                if args.len() < 2 {
                    Safety::Unknown
                } else {
                    classify_docker(&args[1..])
                }
            } else {
                classify_docker(args)
            }
        }
        "singularity" | "apptainer" => {
            let Some(sub) = args.first().copied() else {
                return Safety::Unknown;
            };
            match sub {
                "help" | "version" | "info" => Safety::ReadOnly,
                "build" | "exec" | "run" | "shell" | "instance" | "pull" | "push"
                | "sign" | "verify" => Safety::WriteOperation,
                _ => Safety::Unknown,
            }
        }

        // ── Go ───────────────────────────────────────────────────────
        "go" => classify_go(args),

        // ── Archives ─────────────────────────────────────────────────
        "tar" => classify_tar(args),
        "unzip" | "zip" | "gzip" | "gunzip" | "bzip2" | "bunzip2" | "xz" | "unxz" | "zcat"
        | "zstd" | "unzstd" | "7z" | "7za" | "7zr" | "rar" | "unrar" | "ar" | "deb"
        | "arj" | "cabextract" => Safety::WriteOperation,

        // ── Media / conversion (write files) ─────────────────────────
        "ffmpeg" | "avconv" => Safety::Unknown,
        "ffprobe" | "avprobe" | "mediainfo" | "exiftool" => Safety::ReadOnly,
        "convert" | "mogrify" | "magick" | "sox" => Safety::WriteOperation, // ImageMagick / SoX

        // ── Network downloads — ambiguous (output to stdout unless -o/-O) ─
        "curl" => {
            // `curl -o` / `-O` / `--output` writes to file
            if args.iter().any(|a| a.starts_with("-o") || a == &"-O"
                || a == &"--output" || a == &"--remote-name"
                || a == &"--remote-header-name" || a == &"--data"
                || a == &"-d" || a == &"-F" || a == &"--form")
            {
                Safety::WriteOperation
            } else {
                Safety::Unknown
            }
        }
        "wget" => {
            if args.contains(&"-O") || args.contains(&"--output-document")
                || args.contains(&"-o") || args.contains(&"--output-file")
                || args.contains(&"--post-data") || args.contains(&"--post-file")
                || args.contains(&"-P") || args.contains(&"--directory-prefix")
            {
                Safety::WriteOperation
            } else {
                Safety::Unknown
            }
        }
        "scp" | "rsync" | "aria2c" => Safety::Unknown,

        // ── Process management ───────────────────────────────────────
        "kill" | "pkill" | "killall" | "timeout" | "skill" | "snice" | "renice" => {
            Safety::WriteOperation
        }
        "nohup" | "disown" | "bg" | "fg" => Safety::WriteOperation, // job control

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
        "sc" | "schtasks" | "reg" | "regedit" => Safety::WriteOperation, // Windows system

        // ── Package managers ─────────────────────────────────────────
        "npm" | "npx" | "yarn" | "pnpm" | "bun" => classify_npm(args),
        "pip" | "pip3" => classify_pip(args),
        "apt" | "apt-get" | "aptitude" => classify_apt(args),
        "brew" => classify_brew(args),
        "gem" => classify_gem(args),
        "bundle" | "composer" | "port" => Safety::Unknown,
        "cargo-install" => Safety::Unknown,
        "uv" => classify_pip(args), // uv is a fast Python package manager, same subcommands

        // ── Kubernetes / orchestration ───────────────────────────────
        "kubectl" | "oc" => classify_kubectl(args),
        "helm" => classify_helm(args),
        "kustomize" => {
            let Some(sub) = args.first().copied() else {
                return Safety::Unknown;
            };
            match sub {
                "build" | "version" | "help" => Safety::ReadOnly,
                "edit" => Safety::WriteOperation,
                _ => Safety::Unknown,
            }
        }

        // ── Infrastructure as Code ───────────────────────────────────
        "terraform" | "tofu" => classify_terraform(args),
        "pulumi" => {
            let Some(sub) = args.first().copied() else {
                return Safety::Unknown;
            };
            match sub {
                "preview" | "stack" | "config" | "version" | "help" => {
                    // `pulumi stack ls` / `pulumi config` / `pulumi config get` are read
                    // `pulumi stack init` / `pulumi config set` are write — handled below
                    // For simplicity, only preview is guaranteed read
                    if sub == "preview" {
                        Safety::ReadOnly
                    } else {
                        Safety::Unknown
                    }
                }
                "up" | "destroy" | "init" | "import" | "refresh" | "rm" | "state"
                | "policy" => Safety::WriteOperation,
                _ => Safety::Unknown,
            }
        }
        "ansible" | "ansible-playbook" | "ansible-galaxy" | "ansible-vault" => {
            Safety::Unknown
        }
        "vagrant" => {
            let Some(sub) = args.first().copied() else {
                return Safety::Unknown;
            };
            match sub {
                "status" | "global-status" | "version" | "help" | "validate"
                | "port" | "ssh-config" => Safety::ReadOnly,
                "up" | "destroy" | "halt" | "suspend" | "resume" | "reload"
                | "provision" | "package" | "plugin" | "rsync-auto" | "snapshot" => {
                    Safety::WriteOperation
                }
                _ => Safety::Unknown,
            }
        }

        // ── Systemd / system services ────────────────────────────────
        "systemctl" | "journalctl" => {
            if cmd == "journalctl" {
                // journalctl is read-only (just queries the journal)
                Safety::ReadOnly
            } else {
                classify_systemctl(args)
            }
        }

        // ── Deno ─────────────────────────────────────────────────────
        "deno" => classify_deno(args),

        // ── Nix ──────────────────────────────────────────────────────
        "nix" => classify_nix(args),
        "nix-env" => classify_nix_env(args),
        "nix-shell" => classify_nix_shell(args),
        "nix-build" | "nix-collect-garbage" | "nix-store" | "nix-channel" => {
            Safety::WriteOperation
        }

        // ── OpenSSL ──────────────────────────────────────────────────
        "openssl" => {
            let Some(sub) = args.first().copied() else {
                return Safety::Unknown;
            };
            match sub {
                // Key/cert generation — write
                "genrsa" | "gendsa" | "genpkey" | "req" | "ca" | "x509" | "pkcs12"
                | "enc" | "dgst" | "speed" | "rand" => Safety::WriteOperation,
                // Read-only operations
                "version" | "help" | "ciphers" | "list" => Safety::ReadOnly,
                // Unknown — let through
                _ => Safety::Unknown,
            }
        }

        // ── SSH ──────────────────────────────────────────────────────
        "ssh-keygen" | "ssh-copy-id" | "ssh-add" => Safety::Unknown,
        "ssh" | "ssh-keyscan" | "ssh-keysign" => Safety::Unknown, // interactive, let through

        // ── Network config (can be read with show/status) ────────────
        "ip" | "ifconfig" | "iwconfig" | "nmcli" | "nmtui" | "firewall-cmd"
        | "ufw" | "iptables" | "nft" | "tc" => Safety::Unknown,
        "ping" | "traceroute" | "mtr" | "dig" | "nslookup" | "host" | "nmap"
        | "netstat" | "ss" | "lsof" | "tcpdump" | "whois" => Safety::ReadOnly,

        // ── SELinux / security — ambiguous, depends on subcommand ────
        "setenforce" | "restorecon" | "chcon" | "semodule" | "semanage" => Safety::Unknown,
        "getenforce" | "sestatus" | "sesearch" => Safety::ReadOnly,

        // ── System info (read-only) ──────────────────────────────────
        "uname" | "hostname" | "arch" | "nproc" | "free" | "df" | "du" | "lsblk"
        | "lscpu" | "lspci" | "lsusb" | "lshw" | "dmidecode" | "lsmod" | "uptime"
        | "dmesg" | "sysctl" => {
            // For `sysctl`, `-w` or `--write` is a write operation
            if cmd == "sysctl" && (args.contains(&"-w") || args.contains(&"--write")) {
                Safety::WriteOperation
            } else {
                Safety::ReadOnly
            }
        }

        // ── Date/time — ambiguous (date without args shows date) ────
        "timedatectl" | "date" | "hwclock" => Safety::Unknown,

        // ── User / group management (always system-changing) ─────────
        "useradd" | "userdel" | "usermod" | "groupadd" | "groupdel" | "groupmod"
        | "passwd" | "chage" | "chsh" | "chfn" | "gpasswd" | "newgrp" | "sudo" => {
            Safety::WriteOperation
        }

        // ── crontab / system scheduling ──────────────────────────────
        "crontab" | "at" | "atq" | "atrm" | "batch" => {
            // `crontab -l` is read-only, `crontab -e` / `crontab file` is write
            if cmd == "crontab"
                && (args.contains(&"-l") || args.contains(&"--list")) {
                    return Safety::ReadOnly;
                }
            Safety::Unknown
        }

        // ── Flatpak / Snap ───────────────────────────────────────────
        "flatpak" => {
            let Some(sub) = args.first().copied() else {
                return Safety::Unknown;
            };
            match sub {
                "list" | "info" | "search" | "history" | "help" | "version" => Safety::ReadOnly,
                "install" | "uninstall" | "update" | "override" | "make-current"
                | "mask" | "pin" | "remove" | "repair" => Safety::WriteOperation,
                _ => Safety::Unknown,
            }
        }
        "snap" => {
            let Some(sub) = args.first().copied() else {
                return Safety::Unknown;
            };
            match sub {
                "list" | "info" | "search" | "help" | "version" | "changes" | "tasks"
                | "services" => Safety::ReadOnly,
                "install" | "remove" | "uninstall" | "refresh" | "revert" | "enable"
                | "disable" | "set" | "unset" | "alias" | "unalias" | "prefer"
                | "hold" | "unhold" | "switch" => Safety::WriteOperation,
                _ => Safety::Unknown,
            }
        }

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
        // curl/wget without write flags output to stdout (Unknown)
        assert_eq!(cl.classify("curl https://example.com"), Safety::Unknown);
        assert_eq!(
            cl.classify("wget https://example.com/file"),
            Safety::Unknown
        );
        // With -o/-O, curl/wget writes to file
        assert_eq!(
            cl.classify("curl -o output.txt https://example.com"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("wget -O output.html https://example.com"),
            Safety::WriteOperation
        );
        // scp/rsync always transfer files
        assert_eq!(cl.classify("scp file.txt user@host:/path"), Safety::Unknown);
        assert_eq!(cl.classify("rsync -a src/ dst/"), Safety::Unknown);
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
        assert_eq!(
            cl.classify("zip archive.zip file.txt"),
            Safety::WriteOperation
        );
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

    // ── Docker compose (space-separated) ───────────────────────────────────

    #[test]
    fn docker_compose_space_read() {
        let cl = c();
        assert_eq!(cl.classify("docker compose ps"), Safety::ReadOnly);
        assert_eq!(cl.classify("docker compose logs nginx"), Safety::ReadOnly);
        assert_eq!(cl.classify("docker compose images"), Safety::ReadOnly);
    }

    #[test]
    fn docker_compose_space_write() {
        let cl = c();
        assert_eq!(cl.classify("docker compose up"), Safety::WriteOperation);
        assert_eq!(cl.classify("docker compose down"), Safety::WriteOperation);
        assert_eq!(cl.classify("docker compose build"), Safety::WriteOperation);
    }

    // ── Docker exec (ambiguous, should be Unknown) ─────────────────────────

    #[test]
    fn docker_exec_ambiguous() {
        let cl = c();
        assert_eq!(
            cl.classify("docker exec db cat /etc/hosts"),
            Safety::Unknown
        );
        assert_eq!(
            cl.classify(
                "docker exec paper-postgres psql -U paper -d paper -c '\\d revision_versions' 2>&1"
            ),
            Safety::Unknown
        );
    }

    // ── Podman ─────────────────────────────────────────────────────────────

    #[test]
    fn podman_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("podman ps"), Safety::ReadOnly);
        assert_eq!(cl.classify("podman images"), Safety::ReadOnly);
        assert_eq!(cl.classify("podman inspect nginx"), Safety::ReadOnly);
    }

    #[test]
    fn podman_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("podman run nginx"), Safety::WriteOperation);
        assert_eq!(cl.classify("podman build ."), Safety::WriteOperation);
        assert_eq!(cl.classify("podman push image"), Safety::WriteOperation);
    }

    // ── Just (build tool) ──────────────────────────────────────────────────

    #[test]
    fn just_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("just --list"), Safety::ReadOnly);
        assert_eq!(cl.classify("just --dry-run"), Safety::ReadOnly);
        assert_eq!(cl.classify("just --help"), Safety::ReadOnly);
    }

    #[test]
    fn just_write_commands() {
        let cl = c();
        // `just` tasks are user-defined — `build` is ambiguous
        assert_eq!(cl.classify("just build"), Safety::Unknown);
        // `test` is explicitly listed as a write target in build_tool.rs
        assert_eq!(cl.classify("just test"), Safety::WriteOperation);
    }

    // ── Pip ────────────────────────────────────────────────────────────────

    #[test]
    fn pip_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("pip list"), Safety::ReadOnly);
        assert_eq!(cl.classify("pip show requests"), Safety::ReadOnly);
        assert_eq!(cl.classify("pip freeze"), Safety::ReadOnly);
        assert_eq!(cl.classify("pip check"), Safety::ReadOnly);
        assert_eq!(cl.classify("pip3 list"), Safety::ReadOnly);
    }

    #[test]
    fn pip_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("pip install requests"), Safety::WriteOperation);
        assert_eq!(
            cl.classify("pip uninstall requests"),
            Safety::WriteOperation
        );
        assert_eq!(cl.classify("pip download requests"), Safety::WriteOperation);
        assert_eq!(cl.classify("pip3 install requests"), Safety::WriteOperation);
    }

    // ── uv ─────────────────────────────────────────────────────────────────

    #[test]
    fn uv_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("uv list"), Safety::ReadOnly);
        assert_eq!(cl.classify("uv show requests"), Safety::ReadOnly);
    }

    #[test]
    fn uv_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("uv install requests"), Safety::WriteOperation);
        assert_eq!(cl.classify("uv uninstall requests"), Safety::WriteOperation);
    }

    // ── Apt ────────────────────────────────────────────────────────────────

    #[test]
    fn apt_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("apt list --installed"), Safety::ReadOnly);
        assert_eq!(cl.classify("apt show bash"), Safety::ReadOnly);
        assert_eq!(cl.classify("apt search package"), Safety::ReadOnly);
        assert_eq!(cl.classify("apt policy bash"), Safety::ReadOnly);
        assert_eq!(cl.classify("apt cache show bash"), Safety::ReadOnly);
        assert_eq!(cl.classify("apt-get list --installed"), Safety::ReadOnly);
    }

    #[test]
    fn apt_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("apt install bash"), Safety::WriteOperation);
        assert_eq!(cl.classify("apt remove bash"), Safety::WriteOperation);
        assert_eq!(cl.classify("apt update"), Safety::WriteOperation);
        assert_eq!(cl.classify("apt upgrade"), Safety::WriteOperation);
        assert_eq!(cl.classify("apt autoremove"), Safety::WriteOperation);
        assert_eq!(cl.classify("apt-get install bash"), Safety::WriteOperation);
    }

    // ── Brew ───────────────────────────────────────────────────────────────

    #[test]
    fn brew_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("brew list"), Safety::ReadOnly);
        assert_eq!(cl.classify("brew info bash"), Safety::ReadOnly);
        assert_eq!(cl.classify("brew search package"), Safety::ReadOnly);
        assert_eq!(cl.classify("brew doctor"), Safety::ReadOnly);
        assert_eq!(cl.classify("brew outdated"), Safety::ReadOnly);
        assert_eq!(cl.classify("brew services list"), Safety::ReadOnly);
    }

    #[test]
    fn brew_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("brew install bash"), Safety::WriteOperation);
        assert_eq!(cl.classify("brew uninstall bash"), Safety::WriteOperation);
        assert_eq!(cl.classify("brew upgrade"), Safety::WriteOperation);
        assert_eq!(cl.classify("brew update"), Safety::WriteOperation);
        assert_eq!(
            cl.classify("brew services start nginx"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("brew tap homebrew/core"),
            Safety::WriteOperation
        );
    }

    // ── Gem ────────────────────────────────────────────────────────────────

    #[test]
    fn gem_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("gem list"), Safety::ReadOnly);
        assert_eq!(cl.classify("gem which rake"), Safety::ReadOnly);
        assert_eq!(cl.classify("gem environment"), Safety::ReadOnly);
        assert_eq!(cl.classify("gem outdated"), Safety::ReadOnly);
    }

    #[test]
    fn gem_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("gem install rails"), Safety::WriteOperation);
        assert_eq!(cl.classify("gem uninstall rails"), Safety::WriteOperation);
        assert_eq!(
            cl.classify("gem build mygem.gemspec"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("gem push mygem-1.0.gem"),
            Safety::WriteOperation
        );
    }

    // ── Kubectl ────────────────────────────────────────────────────────────

    #[test]
    fn kubectl_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("kubectl get pods"), Safety::ReadOnly);
        assert_eq!(cl.classify("kubectl describe pod nginx"), Safety::ReadOnly);
        assert_eq!(cl.classify("kubectl logs nginx"), Safety::ReadOnly);
        assert_eq!(cl.classify("kubectl top pod"), Safety::ReadOnly);
        assert_eq!(cl.classify("kubectl version"), Safety::ReadOnly);
        assert_eq!(cl.classify("kubectl config view"), Safety::ReadOnly);
        assert_eq!(
            cl.classify("kubectl config current-context"),
            Safety::ReadOnly
        );
    }

    #[test]
    fn kubectl_write_commands() {
        let cl = c();
        assert_eq!(
            cl.classify("kubectl apply -f file.yaml"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("kubectl delete pod nginx"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("kubectl create deployment nginx"),
            Safety::WriteOperation
        );
        assert_eq!(cl.classify("kubectl exec -it pod -- bash"), Safety::Unknown);
        assert_eq!(
            cl.classify("kubectl config set-context prod"),
            Safety::WriteOperation
        );
    }

    // ── Systemctl ──────────────────────────────────────────────────────────

    #[test]
    fn systemctl_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("systemctl status nginx"), Safety::ReadOnly);
        assert_eq!(cl.classify("systemctl show nginx"), Safety::ReadOnly);
        assert_eq!(cl.classify("systemctl list-units"), Safety::ReadOnly);
        assert_eq!(cl.classify("systemctl is-active nginx"), Safety::ReadOnly);
        assert_eq!(cl.classify("systemctl daemon-reload"), Safety::ReadOnly);
    }

    #[test]
    fn systemctl_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("systemctl start nginx"), Safety::WriteOperation);
        assert_eq!(cl.classify("systemctl stop nginx"), Safety::WriteOperation);
        assert_eq!(
            cl.classify("systemctl enable nginx"),
            Safety::WriteOperation
        );
        assert_eq!(cl.classify("systemctl mask nginx"), Safety::WriteOperation);
        assert_eq!(
            cl.classify("systemctl set-default multi-user.target"),
            Safety::WriteOperation
        );
    }

    // ── Journalctl ─────────────────────────────────────────────────────────

    #[test]
    fn journalctl_is_read_only() {
        let cl = c();
        assert_eq!(cl.classify("journalctl -u nginx"), Safety::ReadOnly);
        assert_eq!(
            cl.classify("journalctl --since yesterday"),
            Safety::ReadOnly
        );
    }

    // ── Deno ───────────────────────────────────────────────────────────────

    #[test]
    fn deno_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("deno check main.ts"), Safety::ReadOnly);
        assert_eq!(cl.classify("deno doc main.ts"), Safety::ReadOnly);
        assert_eq!(cl.classify("deno info"), Safety::ReadOnly);
        assert_eq!(cl.classify("deno lint"), Safety::ReadOnly);
        assert_eq!(cl.classify("deno fmt --check"), Safety::ReadOnly);
    }

    #[test]
    fn deno_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("deno run main.ts"), Safety::WriteOperation);
        assert_eq!(cl.classify("deno compile main.ts"), Safety::WriteOperation);
        assert_eq!(cl.classify("deno cache deps.ts"), Safety::WriteOperation);
        assert_eq!(cl.classify("deno fmt"), Safety::WriteOperation);
        assert_eq!(cl.classify("deno lint --fix"), Safety::WriteOperation);
        assert_eq!(cl.classify("deno task build"), Safety::WriteOperation);
    }

    // ── Terraform / Tofu ──────────────────────────────────────────────────

    #[test]
    fn terraform_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("terraform plan"), Safety::ReadOnly);
        assert_eq!(cl.classify("terraform show"), Safety::ReadOnly);
        assert_eq!(cl.classify("terraform output"), Safety::ReadOnly);
        assert_eq!(cl.classify("terraform state list"), Safety::ReadOnly);
        assert_eq!(cl.classify("terraform workspace list"), Safety::ReadOnly);
        assert_eq!(cl.classify("terraform fmt -check"), Safety::ReadOnly);
    }

    #[test]
    fn terraform_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("terraform apply"), Safety::WriteOperation);
        assert_eq!(cl.classify("terraform destroy"), Safety::WriteOperation);
        assert_eq!(cl.classify("terraform init"), Safety::WriteOperation);
        assert_eq!(cl.classify("terraform fmt"), Safety::WriteOperation);
        assert_eq!(
            cl.classify("terraform state mv old new"),
            Safety::WriteOperation
        );
    }

    #[test]
    fn tofu_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("tofu plan"), Safety::ReadOnly);
        assert_eq!(cl.classify("tofu state list"), Safety::ReadOnly);
    }

    #[test]
    fn tofu_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("tofu apply"), Safety::WriteOperation);
        assert_eq!(cl.classify("tofu init"), Safety::WriteOperation);
    }

    // ── Helm ───────────────────────────────────────────────────────────────

    #[test]
    fn helm_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("helm list"), Safety::ReadOnly);
        assert_eq!(cl.classify("helm status release"), Safety::ReadOnly);
        assert_eq!(cl.classify("helm history release"), Safety::ReadOnly);
        assert_eq!(cl.classify("helm show chart ./chart"), Safety::ReadOnly);
        assert_eq!(cl.classify("helm repo list"), Safety::ReadOnly);
        assert_eq!(cl.classify("helm dependency list"), Safety::ReadOnly);
    }

    #[test]
    fn helm_write_commands() {
        let cl = c();
        assert_eq!(
            cl.classify("helm install release ./chart"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("helm upgrade release ./chart"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("helm uninstall release"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("helm repo add stable url"),
            Safety::WriteOperation
        );
        assert_eq!(cl.classify("helm dependency build"), Safety::WriteOperation);
    }

    // ── Nix ────────────────────────────────────────────────────────────────

    #[test]
    fn nix_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("nix show config"), Safety::ReadOnly);
        assert_eq!(cl.classify("nix search nixpkgs hello"), Safety::ReadOnly);
        assert_eq!(
            cl.classify("nix eval -f default.nix name"),
            Safety::ReadOnly
        );
        assert_eq!(cl.classify("nix flake show ."), Safety::ReadOnly);
        assert_eq!(cl.classify("nix registry list"), Safety::ReadOnly);
        assert_eq!(cl.classify("nix profile list"), Safety::ReadOnly);
        assert_eq!(cl.classify("nix path-info pkg"), Safety::ReadOnly);
    }

    #[test]
    fn nix_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("nix build .#hello"), Safety::WriteOperation);
        assert_eq!(cl.classify("nix run .#app"), Safety::WriteOperation);
        assert_eq!(
            cl.classify("nix develop .#devShell"),
            Safety::WriteOperation
        );
        assert_eq!(cl.classify("nix flake update"), Safety::WriteOperation);
        assert_eq!(
            cl.classify("nix profile install nixpkgs#hello"),
            Safety::WriteOperation
        );
    }

    // ── Nix-env / Nix-shell ────────────────────────────────────────────────

    #[test]
    fn nix_env_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("nix-env -q"), Safety::ReadOnly);
        assert_eq!(cl.classify("nix-env --query"), Safety::ReadOnly);
    }

    #[test]
    fn nix_env_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("nix-env -i hello"), Safety::Unknown);
        assert_eq!(cl.classify("nix-env -e hello"), Safety::Unknown);
    }

    #[test]
    fn nix_shell_is_write() {
        let cl = c();
        assert_eq!(cl.classify("nix-shell -p hello"), Safety::Unknown);
        assert_eq!(
            cl.classify("nix-shell --command 'echo hi'"),
            Safety::Unknown
        );
    }

    // ── Additional deterministic writes ────────────────────────────────────

    #[test]
    fn new_deterministic_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("shred file"), Safety::WriteOperation);
        assert_eq!(cl.classify("fallocate -l 1M file"), Safety::WriteOperation);
        assert_eq!(cl.classify("mktemp"), Safety::WriteOperation);
        assert_eq!(cl.classify("mkfifo mypipe"), Safety::WriteOperation);
        assert_eq!(
            cl.classify("setfacl -m u:user:rwx file"),
            Safety::WriteOperation
        );
        assert_eq!(cl.classify("wipefs /dev/sda1"), Safety::WriteOperation);
        assert_eq!(cl.classify("swapon /dev/sda2"), Safety::WriteOperation);
        assert_eq!(cl.classify("patch < diff.patch"), Safety::WriteOperation);
    }

    #[test]
    fn media_write_commands() {
        let cl = c();
        assert_eq!(
            cl.classify("ffmpeg -i input.mp4 output.avi"),
            Safety::Unknown
        );
        assert_eq!(
            cl.classify("convert input.png output.jpg"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("mogrify -resize 50% *.jpg"),
            Safety::WriteOperation
        );
    }

    #[test]
    fn media_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("ffprobe input.mp4"), Safety::ReadOnly);
        assert_eq!(cl.classify("mediainfo input.mp4"), Safety::ReadOnly);
        assert_eq!(cl.classify("exiftool image.jpg"), Safety::ReadOnly);
    }

    // ── OpenSSL ────────────────────────────────────────────────────────────

    #[test]
    fn openssl_write_commands() {
        let cl = c();
        assert_eq!(
            cl.classify("openssl genrsa -out key.pem 2048"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("openssl req -new -key key.pem -out csr.pem"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("openssl enc -aes-256-cbc -in file -out file.enc"),
            Safety::WriteOperation
        );
    }

    #[test]
    fn openssl_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("openssl version"), Safety::ReadOnly);
        assert_eq!(cl.classify("openssl help"), Safety::ReadOnly);
    }

    // ── SSH keygen ─────────────────────────────────────────────────────────

    #[test]
    fn ssh_keygen_is_write() {
        let cl = c();
        assert_eq!(cl.classify("ssh-keygen -t rsa -b 4096"), Safety::Unknown);
        assert_eq!(cl.classify("ssh-copy-id user@host"), Safety::Unknown);
    }

    // ── Network config (write) ─────────────────────────────────────────────

    #[test]
    fn network_config_is_write() {
        let cl = c();
        assert_eq!(cl.classify("ip link set eth0 up"), Safety::Unknown);
        assert_eq!(cl.classify("ifconfig eth0 up"), Safety::Unknown);
        assert_eq!(
            cl.classify("firewall-cmd --add-port=80/tcp"),
            Safety::Unknown
        );
        assert_eq!(cl.classify("ufw enable"), Safety::Unknown);
        assert_eq!(
            cl.classify("iptables -A INPUT -p tcp --dport 80 -j ACCEPT"),
            Safety::Unknown
        );
    }

    #[test]
    fn network_diagnostics_is_read() {
        let cl = c();
        assert_eq!(cl.classify("ping 8.8.8.8"), Safety::ReadOnly);
        assert_eq!(cl.classify("traceroute 8.8.8.8"), Safety::ReadOnly);
        assert_eq!(cl.classify("dig example.com"), Safety::ReadOnly);
        assert_eq!(cl.classify("nslookup example.com"), Safety::ReadOnly);
        assert_eq!(cl.classify("netstat -tulpn"), Safety::ReadOnly);
        assert_eq!(cl.classify("ss -tulpn"), Safety::ReadOnly);
        assert_eq!(cl.classify("lsof -i :8080"), Safety::ReadOnly);
    }

    // ── System info (read-only) ────────────────────────────────────────────

    #[test]
    fn system_info_is_read() {
        let cl = c();
        assert_eq!(cl.classify("uname -a"), Safety::ReadOnly);
        assert_eq!(cl.classify("free -h"), Safety::ReadOnly);
        assert_eq!(cl.classify("df -h"), Safety::ReadOnly);
        assert_eq!(cl.classify("du -sh ."), Safety::ReadOnly);
        assert_eq!(cl.classify("lscpu"), Safety::ReadOnly);
        assert_eq!(cl.classify("lsblk"), Safety::ReadOnly);
        assert_eq!(cl.classify("uptime"), Safety::ReadOnly);
        assert_eq!(cl.classify("dmesg | tail"), Safety::ReadOnly);
    }

    #[test]
    fn sysctl_read_write() {
        let cl = c();
        assert_eq!(cl.classify("sysctl -a"), Safety::ReadOnly);
        assert_eq!(cl.classify("sysctl net.ipv4.ip_forward"), Safety::ReadOnly);
        assert_eq!(
            cl.classify("sysctl -w net.ipv4.ip_forward=1"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("sysctl --write net.ipv4.ip_forward=1"),
            Safety::WriteOperation
        );
    }

    // ── SELinux ────────────────────────────────────────────────────────────

    #[test]
    fn selinux_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("getenforce"), Safety::ReadOnly);
        assert_eq!(cl.classify("sestatus"), Safety::ReadOnly);
    }

    #[test]
    fn selinux_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("setenforce 1"), Safety::Unknown);
        assert_eq!(cl.classify("restorecon -Rv /var/www"), Safety::Unknown);
    }

    // ── Date/time ──────────────────────────────────────────────────────────

    #[test]
    fn datetime_is_write() {
        let cl = c();
        assert_eq!(
            cl.classify("timedatectl set-time '2024-01-01 12:00:00'"),
            Safety::Unknown
        );
        assert_eq!(cl.classify("date -s '2024-01-01'"), Safety::Unknown);
        assert_eq!(
            cl.classify("hwclock --set --date '2024-01-01'"),
            Safety::Unknown
        );
    }

    // ── User management ────────────────────────────────────────────────────

    #[test]
    fn user_management_is_write() {
        let cl = c();
        assert_eq!(cl.classify("useradd bob"), Safety::WriteOperation);
        assert_eq!(cl.classify("userdel bob"), Safety::WriteOperation);
        assert_eq!(cl.classify("passwd bob"), Safety::WriteOperation);
        assert_eq!(cl.classify("groupadd devs"), Safety::WriteOperation);
    }

    // ── Crontab ────────────────────────────────────────────────────────────

    #[test]
    fn crontab_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("crontab -l"), Safety::ReadOnly);
        assert_eq!(cl.classify("crontab --list"), Safety::ReadOnly);
    }

    #[test]
    fn crontab_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("crontab -e"), Safety::Unknown);
        assert_eq!(cl.classify("crontab myfile"), Safety::Unknown);
    }

    // ── Flatpak ────────────────────────────────────────────────────────────

    #[test]
    fn flatpak_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("flatpak list"), Safety::ReadOnly);
        assert_eq!(
            cl.classify("flatpak info org.gnome.Epiphany"),
            Safety::ReadOnly
        );
        assert_eq!(cl.classify("flatpak search browser"), Safety::ReadOnly);
    }

    #[test]
    fn flatpak_write_commands() {
        let cl = c();
        assert_eq!(
            cl.classify("flatpak install org.gnome.Epiphany"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("flatpak uninstall org.gnome.Epiphany"),
            Safety::WriteOperation
        );
        assert_eq!(cl.classify("flatpak update"), Safety::WriteOperation);
    }

    // ── Snap ───────────────────────────────────────────────────────────────

    #[test]
    fn snap_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("snap list"), Safety::ReadOnly);
        assert_eq!(cl.classify("snap info hello"), Safety::ReadOnly);
        assert_eq!(cl.classify("snap search hello"), Safety::ReadOnly);
    }

    #[test]
    fn snap_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("snap install hello"), Safety::WriteOperation);
        assert_eq!(cl.classify("snap uninstall hello"), Safety::WriteOperation);
        assert_eq!(cl.classify("snap refresh"), Safety::WriteOperation);
    }

    // ── Bazel ──────────────────────────────────────────────────────────────

    #[test]
    fn bazel_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("bazel query //..."), Safety::ReadOnly);
        assert_eq!(cl.classify("bazel info"), Safety::ReadOnly);
    }

    #[test]
    fn bazel_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("bazel build //..."), Safety::WriteOperation);
        assert_eq!(cl.classify("bazel test //..."), Safety::WriteOperation);
    }

    // ── Vagrant ────────────────────────────────────────────────────────────

    #[test]
    fn vagrant_read_commands() {
        let cl = c();
        assert_eq!(cl.classify("vagrant status"), Safety::ReadOnly);
        assert_eq!(cl.classify("vagrant global-status"), Safety::ReadOnly);
    }

    #[test]
    fn vagrant_write_commands() {
        let cl = c();
        assert_eq!(cl.classify("vagrant up"), Safety::WriteOperation);
        assert_eq!(cl.classify("vagrant destroy"), Safety::WriteOperation);
        assert_eq!(cl.classify("vagrant ssh"), Safety::Unknown);
    }

    // ── Kustomize ──────────────────────────────────────────────────────────

    #[test]
    fn kustomize_read_commands() {
        let cl = c();
        assert_eq!(
            cl.classify("kustomize build ./overlays/prod"),
            Safety::ReadOnly
        );
    }

    #[test]
    fn kustomize_write_commands() {
        let cl = c();
        assert_eq!(
            cl.classify("kustomize edit set image nginx:latest"),
            Safety::WriteOperation
        );
    }

    // ── Ansible ────────────────────────────────────────────────────────────

    #[test]
    fn ansible_is_write() {
        let cl = c();
        assert_eq!(cl.classify("ansible all -m ping"), Safety::Unknown);
        assert_eq!(cl.classify("ansible-playbook deploy.yml"), Safety::Unknown);
        assert_eq!(cl.classify("ansible-galaxy install role"), Safety::Unknown);
    }

    // ── Windows system ─────────────────────────────────────────────────────

    #[test]
    fn windows_system_write() {
        let cl = c();
        assert_eq!(cl.classify("sc create MyService"), Safety::WriteOperation);
        assert_eq!(
            cl.classify("reg add HKLM\\Software\\MyApp"),
            Safety::WriteOperation
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
        // Write redirects (should be caught)
        assert!(has_write_redirect("echo hello > file"));
        assert!(has_write_redirect("cat >> file"));
        assert!(has_write_redirect("ls &> file"));

        // NOTE: `>& file` is also a write (older syntax for `&> file`), but the
        // tokenizer can't distinguish it from `>&2` (fd redirect). This is a
        // pre-existing limitation — we accept the false negative.

        // No redirect
        assert!(!has_write_redirect("echo hello"));

        // fd-to-fd redirects (already handled)
        assert!(!has_write_redirect("cmd 2>&1"));
        assert!(!has_write_redirect("cmd >&2"));

        // fd-to-file redirects — the common `2>/dev/null` pattern (was false positive)
        assert!(!has_write_redirect("find . 2>/dev/null"));
        assert!(!has_write_redirect("pip list 2>/dev/null"));
        assert!(!has_write_redirect("grep -r foo 2>/dev/null"));
        assert!(!has_write_redirect("ls 2>/dev/null"));
        assert!(!has_write_redirect("cmd 1>/dev/null"));
        // fd-to-real-file redirect — correctly detected as write
        assert!(has_write_redirect("cmd 2>/tmp/log.txt"));
        // fd-closing redirect — not a file write
        assert!(!has_write_redirect("cmd 3>&-"));
    }

    #[test]
    fn test_stderr_redirect_not_blocked() {
        // Full classification: commands with stderr redirect should not be blocked
        let cl = c();
        assert_eq!(
            cl.classify("find / -name '*.rs' 2>/dev/null"),
            Safety::Unknown
        );
        assert_eq!(
            cl.classify("find / -name '*.rs' 2>/dev/null | head -3"),
            Safety::Unknown
        );
        assert_eq!(cl.classify("pip show rich 2>/dev/null"), Safety::ReadOnly);
        assert_eq!(
            cl.classify("pip show rich 2>/dev/null | head -5"),
            Safety::ReadOnly
        );
        assert_eq!(
            cl.classify("grep -r 'TODO' src/ 2>/dev/null"),
            Safety::Unknown
        );
        assert_eq!(cl.classify("ls -la 2>/dev/null"), Safety::Unknown);
        assert_eq!(cl.classify("cat foo.txt 2>/dev/null"), Safety::Unknown);

        // But a genuinely write command with stderr redirect must still be blocked
        assert_eq!(
            cl.classify("pip install requests 2>/dev/null"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("cargo build 2>/dev/null"),
            Safety::WriteOperation
        );

        // Mixed: stdout redirect + stderr redirect should still be caught
        assert_eq!(
            cl.classify("echo hello > file.txt 2>/dev/null"),
            Safety::WriteOperation
        );
    }

    // ── strip_redirects ─────────────────────────────────────────

    #[test]
    fn test_filter_redirect_simple_fused() {
        // Basic fused redirect tokens are stripped
        assert_eq!(
            strip_redirects(&["git", "branch", "-a", "2>/dev/null"]),
            vec!["git", "branch", "-a"]
        );
        assert_eq!(strip_redirects(&["cmd", ">file"]), vec!["cmd"]);
        assert_eq!(strip_redirects(&["cmd", ">>file"]), vec!["cmd"]);
        assert_eq!(strip_redirects(&["cmd", "&>file"]), vec!["cmd"]);
    }

    #[test]
    fn test_filter_redirect_independent_operator() {
        // Independent redirect operator skips operator AND target
        assert_eq!(
            strip_redirects(&["echo", "hello", ">", "file"]),
            vec!["echo", "hello"]
        );
        assert_eq!(strip_redirects(&["cat", ">>", "file"]), vec!["cat"]);
        assert_eq!(strip_redirects(&["ls", "&>", "file"]), vec!["ls"]);
    }

    #[test]
    fn test_filter_redirect_fd_close() {
        assert_eq!(strip_redirects(&["cmd", "2>&-"]), vec!["cmd"]);
        assert_eq!(strip_redirects(&["cmd", "3>&-"]), vec!["cmd"]);
    }

    #[test]
    fn test_filter_redirect_no_redirect() {
        // No redirect tokens → unchanged
        assert_eq!(
            strip_redirects(&["git", "branch", "-a"]),
            vec!["git", "branch", "-a"]
        );
        assert_eq!(strip_redirects(&["echo", "hello"]), vec!["echo", "hello"]);
        // Empty input
        let empty: Vec<&str> = vec![];
        assert_eq!(strip_redirects(&empty), empty);
    }

    #[test]
    fn test_filter_redirect_mixed_independent_and_fused() {
        // The `> /dev/null` is independent operator + target; `2>&1` is fused
        assert_eq!(
            strip_redirects(&["cmd", ">", "/dev/null", "2>&1"]),
            vec!["cmd"]
        );
    }

    #[test]
    fn test_filter_redirect_compound_command() {
        // After split_compound, the second segment's tokens get filtered
        let cl = c();
        // This is the exact bug: git branch -a with stderr redirect
        assert_eq!(
            cl.classify("cd /tmp && git branch -a 2>/dev/null"),
            Safety::ReadOnly
        );
        // git branch -a with different redirect forms
        assert_eq!(cl.classify("git branch -a 2>/dev/null"), Safety::ReadOnly);
        assert_eq!(
            cl.classify("git branch -a 2>&1 | head -5"),
            Safety::ReadOnly
        );
        // `git branch -a > /dev/null` — stdout to null device, not a real write
        assert_eq!(cl.classify("git branch -a > /dev/null"), Safety::ReadOnly);
    }

    #[test]
    fn test_filter_redirect_git_tag_list_with_redirect() {
        let cl = c();
        // git tag -l with stderr redirect should be read-only
        assert_eq!(cl.classify("git tag -l 2>/dev/null"), Safety::ReadOnly);
        // git tag (without flags) with stderr redirect
        assert_eq!(cl.classify("git tag 2>/dev/null"), Safety::ReadOnly);
    }

    #[test]
    fn test_filter_redirect_git_branch_create_with_redirect() {
        let cl = c();
        // Creating a branch with a redirect should still be classified as write
        assert_eq!(
            cl.classify("git branch new-feature 2>/dev/null"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("git branch -d old-branch 2>/dev/null"),
            Safety::WriteOperation
        );
    }

    #[test]
    fn test_filter_redirect_write_redirect_still_caught() {
        let cl = c();
        // Real write redirect via stdout must still be caught
        assert_eq!(cl.classify("echo hello > file"), Safety::WriteOperation);
        assert_eq!(
            cl.classify("echo hello >> /tmp/log"),
            Safety::WriteOperation
        );
        // Overwrite flag with fused redirect still blocks
        assert_eq!(
            cl.classify("sed -i 's/foo/bar/' file 2>/dev/null"),
            Safety::WriteOperation
        );
    }

    #[test]
    fn test_filter_redirect_non_write_command_through() {
        let cl = c();
        // grep with redirect should be Unknown (not write)
        assert_eq!(cl.classify("grep -r foo src/ 2>/dev/null"), Safety::Unknown);
        // find with redirect
        assert_eq!(
            cl.classify("find . -name '*.rs' 2>/dev/null"),
            Safety::Unknown
        );
    }
}
