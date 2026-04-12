use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandAction {
    Help,
    Connect,
    Model,
    Mode,
    Clear,
    Theme,
    Quit,
}

#[derive(Clone, Debug)]
pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub usage: &'static str,
    pub action: CommandAction,
}

impl CommandSpec {
    pub fn label(&self) -> String {
        format!("/{}", self.name)
    }
}

#[derive(Clone, Debug)]
pub struct CommandSuggestion {
    pub spec: &'static CommandSpec,
    pub score: i32,
}

#[derive(Clone, Debug, Default)]
pub struct CommandRegistry {
    usage_counts: HashMap<&'static str, usize>,
    usage_order: HashMap<&'static str, usize>,
    usage_counter: usize,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list(&self) -> &'static [CommandSpec] {
        COMMANDS
    }

    pub fn mark_used(&mut self, name: &'static str) {
        *self.usage_counts.entry(name).or_insert(0) += 1;
        self.usage_counter = self.usage_counter.wrapping_add(1);
        self.usage_order.insert(name, self.usage_counter);
    }

    pub fn command(&self, name: &str) -> Option<&'static CommandSpec> {
        COMMANDS
            .iter()
            .find(|spec| spec.name == name || spec.aliases.contains(&name))
    }

    pub fn parse_invocation(&self, line: &str) -> Option<(String, Vec<String>)> {
        let trimmed = line.trim();
        let stripped = trimmed.strip_prefix('/')?.trim();
        if stripped.is_empty() {
            return None;
        }

        let mut parts = stripped.split_whitespace().map(str::to_string);
        let name = parts.next()?;
        let args = parts.collect::<Vec<_>>();
        Some((name, args))
    }

    pub fn suggestions(&self, query: &str) -> Vec<CommandSuggestion> {
        let normalized = query.trim().trim_start_matches('/').to_ascii_lowercase();
        let mut candidates = COMMANDS
            .iter()
            .filter_map(|spec| {
                self.score(spec, &normalized)
                    .map(|score| CommandSuggestion { spec, score })
            })
            .collect::<Vec<_>>();

        candidates.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.spec.name.cmp(right.spec.name))
        });

        candidates
    }

    fn score(&self, spec: &'static CommandSpec, query: &str) -> Option<i32> {
        let name = spec.name.to_ascii_lowercase();
        let alias_matches = spec
            .aliases
            .iter()
            .map(|alias| alias.to_ascii_lowercase())
            .collect::<Vec<_>>();

        let mut score = if query.is_empty() {
            1_000
        } else if name == query {
            10_000
        } else if name.starts_with(query) {
            8_000 - ((name.len() - query.len()) as i32 * 10)
        } else if alias_matches.iter().any(|alias| alias == query) {
            9_500
        } else if alias_matches.iter().any(|alias| alias.starts_with(query)) {
            7_500
        } else if let Some(position) = name.find(query) {
            4_500 - (position as i32 * 20)
        } else if alias_matches.iter().any(|alias| alias.contains(query)) {
            3_500
        } else {
            return None;
        };

        score += self.usage_counts.get(&spec.name).copied().unwrap_or(0) as i32 * 40;
        if let Some(order) = self.usage_order.get(&spec.name) {
            score += *order as i32;
        }

        Some(score)
    }
}

#[derive(Clone, Debug, Default)]
pub struct CommandPaletteState {
    pub visible: bool,
    pub query: String,
    pub selected_index: usize,
    pub suggestions: Vec<CommandSuggestion>,
}

impl CommandPaletteState {
    pub fn sync(&mut self, input: &str, registry: &CommandRegistry) {
        let Some(fragment) = command_fragment(input) else {
            self.clear();
            return;
        };

        self.visible = true;
        self.query = fragment.to_string();
        let previous = self.selected_command_name();
        self.suggestions = registry.suggestions(fragment);

        if self.suggestions.is_empty() {
            self.selected_index = 0;
            return;
        }

        if let Some(previous) = previous {
            if let Some(index) = self
                .suggestions
                .iter()
                .position(|item| item.spec.name == previous)
            {
                self.selected_index = index;
                return;
            }
        }

        self.selected_index = self
            .selected_index
            .min(self.suggestions.len().saturating_sub(1));
    }

    pub fn clear(&mut self) {
        self.visible = false;
        self.query.clear();
        self.selected_index = 0;
        self.suggestions.clear();
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.suggestions.is_empty() {
            return;
        }

        let len = self.suggestions.len() as isize;
        let current = self.selected_index as isize;
        let next = (current + delta).rem_euclid(len);
        self.selected_index = next as usize;
    }

    pub fn selected(&self) -> Option<&CommandSuggestion> {
        self.suggestions.get(self.selected_index)
    }

    pub fn selected_command_name(&self) -> Option<&'static str> {
        self.selected().map(|suggestion| suggestion.spec.name)
    }

    pub fn completion(&self) -> Option<String> {
        self.selected()
            .map(|suggestion| format!("/{} ", suggestion.spec.name))
    }

    pub fn active_query(&self) -> Option<&str> {
        if self.visible {
            Some(self.query.as_str())
        } else {
            None
        }
    }
}

fn command_fragment(input: &str) -> Option<&str> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with('/') {
        return None;
    }

    let body = &trimmed[1..];
    if body.chars().any(char::is_whitespace) {
        return None;
    }

    let fragment = body;
    Some(fragment)
}

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "connect",
        aliases: &["login"],
        description: "Connect, update, or add a provider",
        usage: "/connect [provider|new]",
        action: CommandAction::Connect,
    },
    CommandSpec {
        name: "model",
        aliases: &["models"],
        description: "List or switch models",
        usage: "/model [provider:model]",
        action: CommandAction::Model,
    },
    CommandSpec {
        name: "mode",
        aliases: &["plan", "build"],
        description: "Switch between plan and build modes",
        usage: "/mode [plan|build]",
        action: CommandAction::Mode,
    },
    CommandSpec {
        name: "theme",
        aliases: &["appearance"],
        description: "Switch between light and dark themes",
        usage: "/theme [light|dark]",
        action: CommandAction::Theme,
    },
    CommandSpec {
        name: "clear",
        aliases: &["new"],
        description: "Start a new conversation",
        usage: "/clear",
        action: CommandAction::Clear,
    },
    CommandSpec {
        name: "help",
        aliases: &["?"],
        description: "Show command and key help",
        usage: "/help",
        action: CommandAction::Help,
    },
    CommandSpec {
        name: "quit",
        aliases: &["exit", "q"],
        description: "Exit TiDev",
        usage: "/quit",
        action: CommandAction::Quit,
    },
];
