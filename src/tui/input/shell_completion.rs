use std::process::Command;

#[derive(Clone, Debug, Default)]
pub struct ShellCompletionState {
    pub visible: bool,
    pub prefix: String,
    pub candidates: Vec<String>,
    pub selected_index: usize,
    /// Cached list of all available commands (refreshed periodically).
    command_cache: Vec<String>,
    /// Whether the cache has been initialized.
    cache_initialized: bool,
}

impl ShellCompletionState {
    pub fn clear(&mut self) {
        self.visible = false;
        self.prefix.clear();
        self.candidates.clear();
        self.selected_index = 0;
    }

    /// Get command completions for the given prefix.
    pub fn fetch_completions(&mut self, prefix: &str) {
        self.prefix = prefix.to_string();
        self.candidates.clear();
        self.selected_index = 0;

        if prefix.is_empty() {
            self.visible = false;
            return;
        }

        // Ensure command cache is populated
        if !self.cache_initialized {
            self.refresh_cache();
        }

        // Filter cached commands by prefix
        let lower_prefix = prefix.to_lowercase();
        for cmd in &self.command_cache {
            if cmd.to_lowercase().starts_with(&lower_prefix) {
                self.candidates.push(cmd.clone());
            }
        }

        // Sort for consistent ordering
        self.candidates.sort();
        // Deduplicate
        self.candidates.dedup();

        self.visible = !self.candidates.is_empty();
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.candidates.is_empty() {
            return;
        }
        let len = self.candidates.len() as isize;
        let current = self.selected_index as isize;
        self.selected_index = ((current + delta + len) % len) as usize;
    }

    pub fn selected(&self) -> Option<&str> {
        if self.selected_index < self.candidates.len() {
            Some(&self.candidates[self.selected_index])
        } else {
            None
        }
    }

    fn refresh_cache(&mut self) {
        self.command_cache.clear();
        let (shell, args, flag) = if cfg!(windows) {
            // On Windows, use PowerShell to get commands
            (
                "powershell",
                "-Command",
                "Get-Command | Select-Object -ExpandProperty Name",
            )
        } else {
            // On Unix, use bash compgen
            (
                "sh",
                "-c",
                "compgen -c 2>/dev/null || echo 'cd ls mkdir rm cp mv cat echo grep find git cargo make python node npm docker'",
            )
        };

        if let Ok(output) = Command::new(shell).arg(args).arg(flag).output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    self.command_cache.push(trimmed.to_string());
                }
            }
        }

        self.cache_initialized = true;
    }

    /// Accept the current completion and return the replacement text.
    pub fn accept(&mut self) -> Option<String> {
        let selected = self.selected()?.to_string();
        self.clear();
        Some(selected)
    }
}
