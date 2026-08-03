//! Tool name normalization helpers.

/// Map a tool name (possibly an alias) to its canonical form.
pub fn canonical_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "read" | "read_file" => Some("read"),
        "write" | "write_file" => Some("write"),
        "edit" => Some("edit"),
        "glob" => Some("glob"),
        "grep" => Some("grep"),
        "bash" | "shell" => Some("shell"),
        "task" => Some("task"),
        "question" => Some("question"),
        "todowrite" | "todo" => Some("todowrite"),
        "skill" => Some("skill"),
        "websearch" => Some("websearch"),
        "webfetch" => Some("webfetch"),
        "apply_patch" => Some("apply_patch"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_tool_name() {
        assert_eq!(canonical_tool_name("read"), Some("read"));
        assert_eq!(canonical_tool_name("read_file"), Some("read"));
        assert_eq!(canonical_tool_name("write_file"), Some("write"));
        assert_eq!(canonical_tool_name("shell"), Some("shell"));
        assert_eq!(canonical_tool_name("bash"), Some("shell"));
        assert_eq!(canonical_tool_name("todo"), Some("todowrite"));
        assert_eq!(canonical_tool_name("unknown"), None);
    }
}
