// ---------------------------------------------------------------------------
// Shell exit code

/// Parse the exit code from the tool output.
///
/// Supports two formats:
/// - `[exit N]` (current)
/// - `Exit code: N` (legacy)
pub(super) fn parse_shell_exit_code(output: &str) -> (Option<i32>, &str) {
    // Try [exit N] format (new tool output)
    if let Some(stripped) = output.strip_prefix("[exit ")
        && let Some(end_idx) = stripped.find(']')
    {
        let code_str = &stripped[..end_idx];
        if let Ok(code) = code_str.parse::<i32>() {
            let remaining = &stripped[end_idx + 1..];
            let remaining = remaining.strip_prefix('\n').unwrap_or(remaining);
            return (Some(code), remaining);
        }
    }
    // Fallback: "Exit code: N" format (legacy)
    if let Some(pos) = output.rfind("Exit code: ") {
        let rest = &output[pos + "Exit code: ".len()..];
        let code_str = rest.split_whitespace().next().unwrap_or("");
        if let Ok(code) = code_str.parse::<i32>() {
            let body = &output[..pos].trim_end();
            return (Some(code), body);
        }
    }
    (None, output)
}
