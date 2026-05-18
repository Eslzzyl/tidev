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
    CommandSpec {
        name: "status",
        description: "Show session statistics",
    },
    CommandSpec {
        name: "stop",
        description: "Stop current task",
    },
    CommandSpec {
        name: "balance",
        description: "Query provider balance",
    },
    CommandSpec {
        name: "compact",
        description: "Compact session context to save tokens",
    },
    CommandSpec {
        name: "init",
        description: "Analyze project and create AGENTS.md",
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
        "/balance - query provider balance",
        "/status - show session statistics",
        "/stop - stop current task",
        "/compact - compact session context to save tokens",
        "/init - analyze project and create AGENTS.md",
    ]
    .join("\n")
}

/// Statistics for a session, used when formatting status summaries.
#[derive(Debug, Clone)]
pub struct SessionStats<'a> {
    pub session_id: &'a str,
    pub title: &'a str,
    pub message_count: usize,
    pub user_message_count: usize,
    pub assistant_message_count: usize,
    pub tool_call_count: usize,
    pub provider_id: &'a str,
    pub model_id: &'a str,
    pub context_window: usize,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub start_time: std::time::Instant,
    pub avg_response_time_ms: Option<u64>,
}

/// Format status information for display.
pub fn format_status_summary(stats: &SessionStats<'_>) -> String {
    let token_usage = crate::utils::TokenUsage::new(stats.input_tokens, stats.output_tokens, 0, 0);
    let total_tokens = token_usage.total();
    let context_usage_pct = token_usage.context_usage_pct(stats.context_window);

    let uptime = stats.start_time.elapsed();
    let uptime_str = format_uptime(uptime);

    let response_time_str = stats
        .avg_response_time_ms
        .map(|ms| format!("{} ms", ms))
        .unwrap_or_else(|| "N/A".to_string());

    format!(
        "📊 Session Status\n\
        ────────────────\n\
        Session: {}\n\
        Title: {}\n\
        ────────────────\n\
        Messages: {} (user: {}, assistant: {})\n\
        Tool calls: {}\n\
        ────────────────\n\
        Model: {}/{}\n\
        Context: {}/{} ({:.1}%)\n\
        Tokens: in={}, out={}, total={}\n\
        ────────────────\n\
        Avg response: {}\n\
        Gateway uptime: {}",
        stats.session_id,
        stats.title,
        stats.message_count,
        stats.user_message_count,
        stats.assistant_message_count,
        stats.tool_call_count,
        stats.provider_id,
        stats.model_id,
        total_tokens,
        stats.context_window,
        context_usage_pct,
        stats.input_tokens,
        stats.output_tokens,
        total_tokens,
        response_time_str,
        uptime_str
    )
}

/// Format duration into human-readable uptime string.
fn format_uptime(duration: std::time::Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
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
        let cmd = parse_command("/model deepseek:deepseek-v4-flash").expect("command");
        assert_eq!(cmd.name, "model");
        assert_eq!(cmd.args, vec!["deepseek:deepseek-v4-flash"]);
    }

    #[test]
    fn parses_command_with_bot_mention() {
        let cmd = parse_command("/model@my_bot deepseek:deepseek-v4-flash").expect("command");
        assert_eq!(cmd.name, "model");
        assert_eq!(cmd.args, vec!["deepseek:deepseek-v4-flash"]);
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
