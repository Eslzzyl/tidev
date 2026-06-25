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
    for shell in &["/bin/bash", "/bin/zsh", "/bin/sh"] {
        if Path::new(shell).exists() {
            return ResolvedShell {
                program: shell.to_string(),
                arg: "-c".to_string(),
            };
        }
    }
    ResolvedShell {
        program: "/bin/sh".to_string(),
        arg: "-c".to_string(),
    }
}

/// Initialize shell configuration (stub for now).
pub fn init(_config_shell: Option<String>, _paths: Option<&std::path::Path>) {
    // Shell auto-detection happens at first use
}
