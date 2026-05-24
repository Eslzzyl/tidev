/// Check whether a tool name matches a hook's matcher pattern.
///
/// Patterns are simple pipe-separated OR lists:
/// - `"write|edit"` → matches `"write"` or `"edit"`
/// - `"*"` → matches everything
/// - `"apply_patch"` → matches only `"apply_patch"`
pub fn matches_tool(matcher: &str, tool_name: &str) -> bool {
    if matcher == "*" {
        return true;
    }
    matcher.split('|').any(|part| part.trim() == tool_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        assert!(matches_tool("write", "write"));
        assert!(!matches_tool("write", "edit"));
    }

    #[test]
    fn pipe_or_match() {
        assert!(matches_tool("write|edit|apply_patch", "write"));
        assert!(matches_tool("write|edit|apply_patch", "edit"));
        assert!(matches_tool("write|edit|apply_patch", "apply_patch"));
        assert!(!matches_tool("write|edit|apply_patch", "read"));
    }

    #[test]
    fn wildcard_match() {
        assert!(matches_tool("*", "anything"));
        assert!(matches_tool("*", "write"));
    }

    #[test]
    fn whitespace_tolerance() {
        assert!(matches_tool("write | edit", "edit"));
    }
}
