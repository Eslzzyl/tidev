/// Shell detection utilities.
use std::path::Path;

/// Resolved shell configuration.
#[derive(Clone, Debug)]
pub struct ResolvedShell {
    pub program: String,
    pub arg: String,
}

/// Get the default system shell.
pub fn get() -> ResolvedShell {
    detect_default_shell()
}

/// Detect the default system shell.
pub fn detect_default_shell() -> ResolvedShell {
    // Try common shells in order of preference
    for shell in &["/bin/bash", "/bin/zsh", "/bin/sh"] {
        if Path::new(shell).exists() {
            return ResolvedShell {
                program: shell.to_string(),
                arg: "-c".to_string(),
            };
        }
    }
    // Fallback
    ResolvedShell {
        program: "/bin/sh".to_string(),
        arg: "-c".to_string(),
    }
}
