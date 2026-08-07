//! CommandPalette — inline `/command` suggestion popup for the Composer.
//!
//! When the user types `/command_name`, this module provides a fuzzy-matched
//! suggestion list rendered as a popup above the input area.
//!
//! Heuristics (matching old `commands.rs` behaviour, minus usage tracking):
//!
//!   10_000  exact match
//!    9_500  alias exact match
//!    8_000  name starts_with query
//!    7_500  alias starts_with query
//!    4_500  name contains query at position
//!    3_500  alias contains query
//!    1_000  empty query (show all)

use crate::action::{Action, ChatAction, OverlayAction, OverlayKind, SessionAction, ThemeAction};
use crate::theme::ThemeName;

// ---------------------------------------------------------------------------
// CommandAction
// ---------------------------------------------------------------------------

/// Identifier for every built-in command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CommandAction {
    Connect,
    Model,
    Search,
    Session,
    Rename,
    Undo,
    Redo,
    Theme,
    Settings,
    Quit,
    Message,
    Agents,
    Skills,
    /// Start a new conversation.
    Clear,
    /// Backend features not yet available in the new architecture.
    Compact,
    Init,
    /// Expand every thinking block in the current session.
    ExpandThinking,
    /// Collapse every thinking block in the current session.
    CollapseThinking,
}

// ---------------------------------------------------------------------------
// CommandSpec
// ---------------------------------------------------------------------------

/// Static command metadata.
#[derive(Clone, Debug)]
pub(crate) struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub action: CommandAction,
}

impl CommandSpec {
    pub fn label(&self) -> String {
        format!("/{}", self.name)
    }
}

// ---------------------------------------------------------------------------
// CommandSuggestion
// ---------------------------------------------------------------------------

/// A scored suggestion returned by [`CommandRegistry`].
#[derive(Clone, Debug)]
pub(crate) struct CommandSuggestion {
    pub spec: &'static CommandSpec,
    pub score: i32,
}

// ---------------------------------------------------------------------------
// COMMANDS — static registry
// ---------------------------------------------------------------------------

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "connect",
        aliases: &["login"],
        description: "Open the provider picker",
        action: CommandAction::Connect,
    },
    CommandSpec {
        name: "model",
        aliases: &["models"],
        description: "Open the model panel",
        action: CommandAction::Model,
    },
    CommandSpec {
        name: "search",
        aliases: &["websearch"],
        description: "Open the search provider panel",
        action: CommandAction::Search,
    },
    CommandSpec {
        name: "session",
        aliases: &["sessions", "resume"],
        description: "Open the session panel",
        action: CommandAction::Session,
    },
    CommandSpec {
        name: "message",
        aliases: &["msg", "timeline", "history"],
        description: "Search current session user messages",
        action: CommandAction::Message,
    },
    CommandSpec {
        name: "rename",
        aliases: &["title"],
        description: "Rename the current session",
        action: CommandAction::Rename,
    },
    CommandSpec {
        name: "theme",
        aliases: &["appearance"],
        description: "Switch between built-in themes",
        action: CommandAction::Theme,
    },
    CommandSpec {
        name: "undo",
        aliases: &[],
        description: "Revert the previous user message",
        action: CommandAction::Undo,
    },
    CommandSpec {
        name: "redo",
        aliases: &[],
        description: "Move one step forward in undo history",
        action: CommandAction::Redo,
    },
    CommandSpec {
        name: "settings",
        aliases: &["config"],
        description: "Open settings panel",
        action: CommandAction::Settings,
    },
    CommandSpec {
        name: "new",
        aliases: &["clear"],
        description: "Start a new conversation",
        action: CommandAction::Clear,
    },
    CommandSpec {
        name: "agents",
        aliases: &[],
        description: "List all available sub-agent types",
        action: CommandAction::Agents,
    },
    CommandSpec {
        name: "skills",
        aliases: &[],
        description: "Browse and preview available skills",
        action: CommandAction::Skills,
    },
    CommandSpec {
        name: "exit",
        aliases: &["quit", "q"],
        description: "Exit tidev",
        action: CommandAction::Quit,
    },
    CommandSpec {
        name: "compact",
        aliases: &[],
        description: "Compact the current session context to free space",
        action: CommandAction::Compact,
    },
    CommandSpec {
        name: "init",
        aliases: &[],
        description: "Analyze project and create AGENTS.md",
        action: CommandAction::Init,
    },
    CommandSpec {
        name: "expand-thinking",
        aliases: &["expand", "show-thinking"],
        description: "Expand all thinking blocks in the current session",
        action: CommandAction::ExpandThinking,
    },
    CommandSpec {
        name: "collapse-thinking",
        aliases: &["collapse", "hide-thinking"],
        description: "Collapse all thinking blocks in the current session",
        action: CommandAction::CollapseThinking,
    },
];

// ---------------------------------------------------------------------------
// CommandRegistry
// ---------------------------------------------------------------------------

/// Command registry with fuzzy matching.
#[derive(Clone, Debug, Default)]
pub(crate) struct CommandRegistry;

impl CommandRegistry {
    pub fn new() -> Self {
        Self
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
        let mut candidates: Vec<CommandSuggestion> = COMMANDS
            .iter()
            .filter_map(|spec| {
                self.score(spec, &normalized)
                    .map(|score| CommandSuggestion { spec, score })
            })
            .collect();

        candidates.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.spec.name.cmp(b.spec.name))
        });

        candidates
    }

    fn score(&self, spec: &'static CommandSpec, query: &str) -> Option<i32> {
        if query.is_empty() {
            return Some(1_000);
        }

        let name = spec.name.to_ascii_lowercase();
        let alias_matches: Vec<String> = spec
            .aliases
            .iter()
            .map(|a| a.to_ascii_lowercase())
            .collect();

        if name == query {
            return Some(10_000);
        }
        if alias_matches.iter().any(|a| a == query) {
            return Some(9_500);
        }
        if name.starts_with(query) {
            return Some(8_000 - ((name.len() - query.len()) as i32 * 10));
        }
        if alias_matches.iter().any(|a| a.starts_with(query)) {
            return Some(7_500);
        }
        if let Some(position) = name.find(query) {
            return Some(4_500 - (position as i32 * 20));
        }
        if alias_matches.iter().any(|a| a.contains(query)) {
            return Some(3_500);
        }

        None
    }
}

// ---------------------------------------------------------------------------
// CommandPaletteState
// ---------------------------------------------------------------------------

/// State of the command palette popup.
#[derive(Clone, Debug)]
pub(crate) struct CommandPaletteState {
    pub visible: bool,
    pub query: String,
    pub selected_index: usize,
    pub suggestions: Vec<CommandSuggestion>,
    /// Whether the user has manually navigated the selection (↑/↓).
    ///
    /// While `false`, `sync` keeps the top-ranked suggestion selected so the
    /// selection always tracks the best match for the typed query. Once the
    /// user navigates, the selection is preserved by name across query
    /// changes, and reset only when the query is shortened again.
    user_moved: bool,
}

impl CommandPaletteState {
    pub fn new() -> Self {
        Self {
            visible: false,
            query: String::new(),
            selected_index: 0,
            suggestions: Vec::new(),
            user_moved: false,
        }
    }

    pub fn clear(&mut self) {
        self.visible = false;
        self.query.clear();
        self.selected_index = 0;
        self.suggestions.clear();
        self.user_moved = false;
    }

    pub fn sync(&mut self, input: &str, registry: &CommandRegistry) {
        let Some(fragment) = command_fragment(input) else {
            self.clear();
            return;
        };

        self.visible = true;
        // A shortened query means the user is restarting their search:
        // drop manual navigation so the top-ranked suggestion is selected.
        if fragment.len() < self.query.len() {
            self.user_moved = false;
        }
        let previous = self.selected_command_name();
        self.query = fragment.to_string();
        self.suggestions = registry.suggestions(fragment);

        if self.suggestions.is_empty() {
            self.selected_index = 0;
            return;
        }

        if !self.user_moved {
            // No manual navigation: always follow the best match.
            self.selected_index = 0;
            return;
        }

        // Preserve selection if the previously selected command still exists.
        if let Some(prev) = previous
            && let Some(index) = self
                .suggestions
                .iter()
                .position(|item| item.spec.name == prev)
        {
            self.selected_index = index;
            return;
        }

        self.selected_index = self
            .selected_index
            .min(self.suggestions.len().saturating_sub(1));
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.suggestions.is_empty() {
            return;
        }
        self.user_moved = true;
        let len = self.suggestions.len() as isize;
        let current = self.selected_index as isize;
        let next = (current + delta).rem_euclid(len);
        self.selected_index = next as usize;
    }

    pub fn selected(&self) -> Option<&CommandSuggestion> {
        self.suggestions.get(self.selected_index)
    }

    pub fn selected_command_name(&self) -> Option<&'static str> {
        self.selected().map(|s| s.spec.name)
    }

    pub fn completion(&self) -> Option<String> {
        self.selected().map(|s| format!("/{} ", s.spec.name))
    }

    /// Total height of the popup in terminal rows (0 if hidden).
    pub fn popup_height(&self) -> u16 {
        if !self.visible || self.suggestions.is_empty() {
            return 0;
        }
        // 1 border + up to 6 suggestion rows + 1 gap.
        (self.suggestions.len() as u16).min(6).saturating_add(2)
    }
}

/// Extract the `/fragment` part from the input text.
fn command_fragment(input: &str) -> Option<&str> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with('/') {
        return None;
    }
    let body = &trimmed[1..];
    if body.chars().any(char::is_whitespace) {
        return None;
    }
    Some(body)
}

// ---------------------------------------------------------------------------
// execute_command — translate a command invocation into Actions
// ---------------------------------------------------------------------------

/// Translate a command invocation into one or more Actions.
///
/// Called by the App when a `/command` text is submitted (not typed).
/// Returns `None` if the command should be treated as a regular prompt.
pub(crate) fn execute_command(action: CommandAction, args: &[String]) -> Vec<Action> {
    match action {
        CommandAction::Connect => {
            vec![Action::Overlay(OverlayAction::Open(
                OverlayKind::ConnectDialog,
            ))]
        }
        CommandAction::Model => {
            vec![Action::Overlay(OverlayAction::Open(
                OverlayKind::ModelPanel,
            ))]
        }
        CommandAction::Search => {
            vec![Action::Overlay(OverlayAction::Open(
                OverlayKind::SearchPanel,
            ))]
        }
        CommandAction::Session => {
            vec![Action::Overlay(OverlayAction::Open(
                OverlayKind::SessionPanel,
            ))]
        }
        CommandAction::Message => {
            vec![Action::Overlay(OverlayAction::Open(
                OverlayKind::MessagePanel,
            ))]
        }
        CommandAction::Rename => {
            vec![Action::Overlay(OverlayAction::Open(
                OverlayKind::RenameDialog,
            ))]
        }
        CommandAction::Theme => {
            if args.is_empty() {
                vec![Action::Overlay(OverlayAction::Open(
                    OverlayKind::ThemePanel,
                ))]
            } else if let Some(theme) = ThemeName::parse(&args.join(" ")) {
                vec![Action::Theme(ThemeAction::Set(theme))]
            } else {
                vec![]
            }
        }
        CommandAction::Undo => {
            vec![Action::Session(SessionAction::Undo)]
        }
        CommandAction::Redo => {
            vec![Action::Session(SessionAction::Redo)]
        }
        CommandAction::Settings => {
            vec![Action::Overlay(OverlayAction::Open(
                OverlayKind::SettingsPanel,
            ))]
        }
        CommandAction::Agents => {
            vec![Action::Overlay(OverlayAction::Open(
                OverlayKind::AgentsPanel,
            ))]
        }
        CommandAction::Skills => {
            vec![Action::Overlay(OverlayAction::Open(
                OverlayKind::SkillsPanel,
            ))]
        }
        CommandAction::Clear => {
            vec![Action::Session(SessionAction::Create)]
        }
        CommandAction::Quit => {
            vec![Action::Quit]
        }
        // Commands that need runtime access are handled by App.process_action.
        CommandAction::Compact => {
            vec![Action::Session(SessionAction::Compact)]
        }
        CommandAction::Init => {
            let prompt = tidev_core::prompts::init_command_with_args(&args.join(" "));
            vec![Action::Chat(ChatAction::SetInput(prompt))]
        }
        CommandAction::ExpandThinking => {
            vec![Action::Chat(ChatAction::ExpandAllThinking)]
        }
        CommandAction::CollapseThinking => {
            vec![Action::Chat(ChatAction::CollapseAllThinking)]
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_invocation() {
        let reg = CommandRegistry::new();
        assert_eq!(
            reg.parse_invocation("/theme").map(|(n, _)| n),
            Some("theme".to_string())
        );
        assert_eq!(
            reg.parse_invocation("/session query")
                .map(|(n, a)| (n, a.len())),
            Some(("session".to_string(), 1))
        );
        assert!(reg.parse_invocation("not a command").is_none());
        assert!(reg.parse_invocation("/").is_none());
    }

    #[test]
    fn test_suggestions_exact() {
        let reg = CommandRegistry::new();
        let results = reg.suggestions("/theme");
        assert_eq!(results[0].spec.name, "theme");
        assert!(results[0].score >= 10_000);
    }

    #[test]
    fn test_suggestions_prefix() {
        let reg = CommandRegistry::new();
        let results = reg.suggestions("/se");
        assert!(results.iter().any(|s| s.spec.name == "session"
            || s.spec.name == "search"
            || s.spec.name == "settings"));
    }

    #[test]
    fn test_suggestions_alias() {
        let reg = CommandRegistry::new();
        let results = reg.suggestions("/login");
        assert_eq!(results[0].spec.name, "connect");
    }

    #[test]
    fn test_popup_height() {
        let mut state = CommandPaletteState::new();
        assert_eq!(state.popup_height(), 0);

        state.visible = true;
        state.suggestions = vec![CommandSuggestion {
            spec: &COMMANDS[0],
            score: 100,
        }];
        assert_eq!(state.popup_height(), 3); // 2 border + 1 item
    }

    #[test]
    fn test_command_fragment() {
        assert_eq!(command_fragment("/theme"), Some("theme"));
        assert_eq!(command_fragment("/connect"), Some("connect"));
        assert!(command_fragment("/session name").is_none());
        assert!(command_fragment("plain text").is_none());
    }

    #[test]
    fn test_thinking_commands_registered() {
        let reg = CommandRegistry::new();
        assert_eq!(
            reg.parse_invocation("/expand-thinking").map(|(n, _)| n),
            Some("expand-thinking".to_string())
        );
        assert_eq!(
            reg.parse_invocation("/collapse-thinking").map(|(n, _)| n),
            Some("collapse-thinking".to_string())
        );
        assert_eq!(reg.command("expand").map(|s| s.action), Some(CommandAction::ExpandThinking));
        assert_eq!(reg.command("collapse").map(|s| s.action), Some(CommandAction::CollapseThinking));
    }

    #[test]
    fn test_thinking_commands_execute() {
        assert!(matches!(
            execute_command(CommandAction::ExpandThinking, &[]).as_slice(),
            [Action::Chat(ChatAction::ExpandAllThinking)]
        ));
        assert!(matches!(
            execute_command(CommandAction::CollapseThinking, &[]).as_slice(),
            [Action::Chat(ChatAction::CollapseAllThinking)]
        ));
    }

    #[test]
    fn test_default_selection_follows_best_match() {
        let reg = CommandRegistry::new();
        let mut state = CommandPaletteState::new();

        // Typing / -> /e -> /ex without manual navigation: the selection
        // always tracks the top-ranked suggestion, so /ex lands on "exit".
        state.sync("/", &reg);
        state.sync("/e", &reg);
        state.sync("/ex", &reg);
        assert_eq!(state.selected().map(|s| s.spec.name), Some("exit"));
    }

    #[test]
    fn test_manual_selection_is_preserved_by_name() {
        let reg = CommandRegistry::new();
        let mut state = CommandPaletteState::new();

        // At "/" all commands share the same score and sort by name, so the
        // first entries are agents(0), collapse-thinking(1), compact(2).
        state.sync("/", &reg);
        state.move_selection(1);
        state.move_selection(1);
        assert_eq!(state.selected().map(|s| s.spec.name), Some("compact"));

        // Keep typing: "compact" stays selected as long as it still matches.
        state.sync("/co", &reg);
        assert_eq!(state.selected().map(|s| s.spec.name), Some("compact"));
    }

    #[test]
    fn test_shortened_query_resets_manual_selection() {
        let reg = CommandRegistry::new();
        let mut state = CommandPaletteState::new();

        state.sync("/co", &reg);
        state.move_selection(1); // manual navigation
        assert!(state.user_moved);
        assert_eq!(state.selected().map(|s| s.spec.name), Some("connect"));

        // Deleting back to "/c" resets the manual flag; the selection
        // follows the top-ranked suggestion again.
        state.sync("/c", &reg);
        assert!(!state.user_moved);
        assert_eq!(state.selected().map(|s| s.spec.name), Some("compact"));
    }
}
