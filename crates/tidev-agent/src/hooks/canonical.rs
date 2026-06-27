/// Canonicalize a tool name to its standard form.
/// This is a simple string normalization — no dependencies on the tooling crate.
pub fn canonical_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "read" | "read_file" => Some("read"),
        "write" | "write_file" => Some("write"),
        "edit" => Some("edit"),
        "glob" => Some("glob"),
        "grep" => Some("grep"),
        "bash" | "shell" => Some("bash"),
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
