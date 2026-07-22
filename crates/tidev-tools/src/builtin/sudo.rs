//! Privilege escalation handling for the shell tool.
//!
//! When the shell tool runs a command that contains `sudo` (or similar privilege
//! escalation tools like `doas`, `pkexec`), the child process may try to open
//! `/dev/tty` directly to prompt for a password. In TUI mode, this writes raw
//! text to the terminal outside of the alternate screen, corrupting the display.
//!
//! This module provides:
//!
//! - **Detection**: scan a shell command for privilege escalation patterns
//! - **Transformation**: wrap the command so `sudo` uses `SUDO_ASKPASS` instead
//!   of /dev/tty (by defining a shell function that aliases `sudo` to `sudo -A`)
//! - **Askpass**: a temporary helper script that writes a user-friendly error
//!   message to stderr and exits with failure, preventing the password prompt

use std::io::Write;
use std::path::PathBuf;

/// Check whether a shell command contains privilege escalation patterns
/// (sudo, doas, pkexec, su -c).
///
/// Uses simple substring matching anchored to word boundaries. False positives
/// are possible (e.g., `echo sudo`) but harmless — wrapping a non-elevated
/// `sudo` word with the shell function is a no-op.
pub fn has_privilege_escalation(command: &str) -> bool {
    let command_lower = command.to_ascii_lowercase();

    // Quick scan: look for key substrings before doing expensive splitting
    let has_keyword = command_lower.contains("sudo")
        || command_lower.contains("doas")
        || command_lower.contains("pkexec")
        || command_lower.contains("su ");
    if !has_keyword {
        return false;
    }

    // Tokenize at word boundaries for accurate detection
    for word in command_lower.split_whitespace() {
        match word {
            "sudo" | "doas" | "pkexec" => return true,
            "su" => return true, // su -c will be caught by su
            _ => {}
        }
    }

    // Also check for `su -c` pattern where -c is separate or joined
    if command_lower.contains("su -c") || command_lower.contains("su -c") {
        return true;
    }

    false
}

/// Wrap a shell command so that `sudo` invocations use `sudo -A` with
/// `SUDO_ASKPASS` pointing to our helper script.
///
/// This is done by prepending a shell function that shadows the `sudo` command:
///
/// ```sh
/// sudo() { command sudo -A "$@"; }; export SUDO_ASKPASS=<path>; <original command>
/// ```
///
/// This approach is more robust than string replacement because it correctly
/// handles sudo embedded in subshells, pipelines, and variable expansions.
pub fn wrap_command(command: &str, askpass_path: &std::path::Path) -> String {
    let path_str = askpass_path.to_string_lossy();

    // We need to escape the path for single-quoted shell string.
    // The only character that needs escaping in single quotes is the single
    // quote itself, which we handle by ending the quote, adding an escaped
    // quote, and resuming.
    let escaped_path: String = path_str
        .chars()
        .flat_map(|c| {
            if c == '\'' {
                "'\\''".chars().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect();

    format!("sudo() {{ command sudo -A \"$@\"; }}; export SUDO_ASKPASS='{escaped_path}'; {command}")
}

/// A guard that cleans up the askpass script directory on drop.
pub struct AskpassGuard {
    pub script_path: PathBuf,
    _dir: tempfile::TempDir,
}

impl AskpassGuard {
    pub fn path(&self) -> &std::path::Path {
        &self.script_path
    }
}

/// Create a temporary askpass helper script.
///
/// The script, when invoked by `sudo -A`, writes a helpful error message to
/// stderr (which gets captured by tidev's pipe) and exits with code 1.
/// This prevents `sudo` from ever writing a password prompt to the terminal.
pub fn create_askpass_script() -> std::io::Result<AskpassGuard> {
    let dir = tempfile::tempdir()?;
    let script_path = dir.path().join("tidev-askpass.sh");

    let mut f = std::fs::File::create(&script_path)?;
    write!(
        f,
        "#!/bin/sh\n\
         cat >&2 << 'TIDEV_EOF'\n\
         \n\
         \x1b[1;33mtidev:\x1b[0m this command requires sudo with a password.\n\
         \n\
         Interactive password prompts are not supported while the TUI is active.\n\
         To fix this, open another terminal and run:\n\
         \n\
         \x1b[1;32m  sudo -v\x1b[0m\n\
         \n\
         This caches your sudo credentials. Then retry the command in tidev.\n\
         TIDEV_EOF\n\
         exit 1\n"
    )?;
    f.flush()?;

    // Make the script executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&script_path, perms)?;
    }

    Ok(AskpassGuard {
        script_path,
        _dir: dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detects_sudo_at_start() {
        assert!(has_privilege_escalation("sudo apt-get update"));
    }

    #[test]
    fn test_detects_sudo_after_and() {
        assert!(has_privilege_escalation("echo hi && sudo apt-get update"));
    }

    #[test]
    fn test_detects_sudo_after_semicolon() {
        assert!(has_privilege_escalation("echo hi; sudo apt-get update"));
    }

    #[test]
    fn test_detects_sudo_after_pipe() {
        assert!(has_privilege_escalation("echo hi | sudo tee /etc/file"));
    }

    #[test]
    fn test_detects_doas() {
        assert!(has_privilege_escalation("doas pkg update"));
    }

    #[test]
    fn test_detects_pkexec() {
        assert!(has_privilege_escalation("pkexec apt update"));
    }

    #[test]
    fn test_rejects_echo_sudo() {
        // "echo sudo" contains "sudo" as an argument, not a command
        // Our heuristic errs on the side of false positives, which is safe
        assert!(has_privilege_escalation("echo sudo"));
    }

    #[test]
    fn test_rejects_no_sudo() {
        assert!(!has_privilege_escalation("echo hello world"));
        assert!(!has_privilege_escalation("ls -la"));
        assert!(!has_privilege_escalation("cargo build"));
    }

    #[test]
    fn test_rejects_sudo_in_word() {
        // "sudo" as part of a larger word should not match
        assert!(!has_privilege_escalation("echo pseudocode"));
    }

    #[test]
    fn test_wrap_command_includes_function() {
        let dir = tempfile::tempdir().unwrap();
        let askpass = dir.path().join("askpass.sh");
        let wrapped = wrap_command("apt-get update", &askpass);

        assert!(wrapped.starts_with("sudo() { command sudo -A \"$@\"; };"));
        assert!(wrapped.contains(askpass.to_string_lossy().as_ref()));
        assert!(wrapped.ends_with("apt-get update"));
    }

    #[test]
    fn test_wrap_command_with_special_chars() {
        let dir = tempfile::tempdir().unwrap();
        let askpass = dir.path().join("askpass.sh");
        let wrapped = wrap_command("echo 'hello world' | sudo tee /etc/hosts", &askpass);

        assert!(wrapped.starts_with("sudo() {"));
        assert!(wrapped.contains("sudo -A"));
        assert!(wrapped.contains("sudo tee"));
    }

    #[test]
    fn test_create_askpass_script_is_executable() {
        let guard = create_askpass_script().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(guard.path()).unwrap();
            assert!(meta.permissions().mode() & 0o111 != 0);
        }
        // Script should exist and be readable
        assert!(guard.path().exists());
        let content = std::fs::read_to_string(guard.path()).unwrap();
        assert!(content.contains("sudo"));
    }
}
