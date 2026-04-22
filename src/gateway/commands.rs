//! Shared slash command definitions for all gateway channels.
//!
//! This module provides:
//! - `CommandInvocation`: parsed command structure
//! - `parse_command`: parse slash command from message content
//! - `GATEWAY_COMMANDS`: list of available commands for registration
//! - Help text generators

/// Parsed slash command invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInvocation {
    pub name: String,
    pub args: Vec<String>,
}

/// Command specification for platform registration.
pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
}

/// All gateway shared slash commands.
pub const GATEWAY_COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        description: "Show help",
    },
    CommandSpec {
        name: "new",
        description: "Start a fresh session",
    },
    CommandSpec {
        name: "session",
        description: "Manage current session",
    },
    CommandSpec {
        name: "model",
        description: "Switch provider or model",
    },
];

/// Parse a slash command from message content.
///
/// Returns `None` if the content doesn't start with `/`.
/// Handles bot mentions like `/model@my_bot args`.
pub fn parse_command(content: &str) -> Option<CommandInvocation> {
    let mut parts = content.split_whitespace();
    let first = parts.next()?;
    if !first.starts_with('/') {
        return None;
    }

    let raw_name = first.trim_start_matches('/');
    if raw_name.is_empty() {
        return None;
    }

    // Strip bot mention (e.g., "model@my_bot" -> "model")
    let name = raw_name
        .split('@')
        .next()
        .unwrap_or(raw_name)
        .trim()
        .to_ascii_lowercase();

    if name.is_empty() {
        return None;
    }

    Some(CommandInvocation {
        name,
        args: parts.map(str::to_string).collect(),
    })
}

/// Gateway command help text.
pub fn gateway_help_text() -> String {
    [
        "Gateway command help",
        "/new - start a fresh session",
        "/session - show current session status",
        "/model - switch provider or model",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_command() {
        let cmd = parse_command("/new").expect("command");
        assert_eq!(cmd.name, "new");
        assert!(cmd.args.is_empty());
    }

    #[test]
    fn parses_command_with_args() {
        let cmd = parse_command("/model deepseek:deepseek-chat").expect("command");
        assert_eq!(cmd.name, "model");
        assert_eq!(cmd.args, vec!["deepseek:deepseek-chat"]);
    }

    #[test]
    fn parses_command_with_bot_mention() {
        let cmd = parse_command("/model@my_bot deepseek:deepseek-chat").expect("command");
        assert_eq!(cmd.name, "model");
        assert_eq!(cmd.args, vec!["deepseek:deepseek-chat"]);
    }

    #[test]
    fn parses_session_command_without_args() {
        let cmd = parse_command("/session").expect("command");
        assert_eq!(cmd.name, "session");
        assert!(cmd.args.is_empty());
    }

    #[test]
    fn returns_none_for_non_command() {
        assert!(parse_command("hello world").is_none());
    }

    #[test]
    fn returns_none_for_empty_command() {
        assert!(parse_command("/").is_none());
    }

    #[test]
    fn normalizes_command_to_lowercase() {
        let cmd = parse_command("/NEW").expect("command");
        assert_eq!(cmd.name, "new");
    }
}
