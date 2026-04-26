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

/// Format status information for display.
/// Parameters:
/// - session_id: The current session ID
/// - title: Session title
/// - message_count: Total number of messages
/// - user_message_count: Number of user messages
/// - assistant_message_count: Number of assistant messages
/// - tool_call_count: Number of tool calls made
/// - provider_id: LLM provider ID
/// - model_id: Model ID
/// - context_window: Model's context window size (in tokens)
/// - input_tokens: Total input tokens used
/// - output_tokens: Total output tokens used
/// - start_time: Gateway start time (Instant)
/// - avg_response_time_ms: Average response time in milliseconds
pub fn format_status_summary(
    session_id: &str,
    title: &str,
    message_count: usize,
    user_message_count: usize,
    assistant_message_count: usize,
    tool_call_count: usize,
    provider_id: &str,
    model_id: &str,
    context_window: usize,
    input_tokens: u32,
    output_tokens: u32,
    start_time: std::time::Instant,
    avg_response_time_ms: Option<u64>,
) -> String {
    let total_tokens = input_tokens + output_tokens;
    let context_usage_pct = if context_window > 0 {
        (total_tokens as f64 / context_window as f64 * 100.0).min(100.0)
    } else {
        0.0
    };

    let uptime = start_time.elapsed();
    let uptime_str = format_uptime(uptime);

    let response_time_str = avg_response_time_ms
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
        session_id,
        title,
        message_count,
        user_message_count,
        assistant_message_count,
        tool_call_count,
        provider_id,
        model_id,
        total_tokens,
        context_window,
        context_usage_pct,
        input_tokens,
        output_tokens,
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
