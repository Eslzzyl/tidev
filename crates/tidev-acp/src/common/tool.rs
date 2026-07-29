//! Shared tool presentation helpers.

use tidev_types::message::ToolCall;
use tidev_types::tools::canonical_tool_name;

/// Build the human-readable title used for a tool call in ACP requests.
pub(crate) fn tool_title(tc: &ToolCall) -> String {
    let args: Option<serde_json::Value> = serde_json::from_str(&tc.arguments).ok();
    let string_arg = |key: &str| {
        args.as_ref()
            .and_then(|value| value.get(key))
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned)
    };

    match canonical_tool_name(&tc.name) {
        Some("read") => match string_arg("file_path").or_else(|| string_arg("path")) {
            Some(path) if !path.is_empty() => format!("Read {path}"),
            _ => "Read file".to_string(),
        },
        Some("write") => match string_arg("file_path") {
            Some(path) if !path.is_empty() => format!("Write {path}"),
            _ => "Write file".to_string(),
        },
        Some("edit") => match string_arg("file_path") {
            Some(path) if !path.is_empty() => format!("Edit {path}"),
            _ => "Edit file".to_string(),
        },
        Some("apply_patch") => "Apply patch".to_string(),
        Some("shell") => {
            let display = string_arg("description").or_else(|| string_arg("command"));
            match display {
                Some(value) if !value.is_empty() => format!("Shell {value}"),
                _ => "Shell".to_string(),
            }
        }
        Some("glob") => match string_arg("pattern") {
            Some(pattern) if !pattern.is_empty() => format!("Glob {pattern}"),
            _ => "Search files".to_string(),
        },
        Some("grep") => match string_arg("pattern") {
            Some(pattern) if !pattern.is_empty() => format!("Grep {pattern}"),
            _ => "Search files".to_string(),
        },
        Some("task") => match string_arg("description") {
            Some(description) if !description.is_empty() => format!("Task: {description}"),
            _ => "Delegate task".to_string(),
        },
        Some("question") => {
            let count = args
                .as_ref()
                .and_then(|value| value.get("questions"))
                .and_then(|value| value.as_array())
                .map_or(0, Vec::len);
            if count <= 1 {
                "Ask 1 question".to_string()
            } else {
                format!("Ask {count} questions")
            }
        }
        Some("websearch") => match string_arg("query") {
            Some(query) if !query.is_empty() => format!("Search web for {query}"),
            _ => "Search web".to_string(),
        },
        Some("webfetch") => match string_arg("url") {
            Some(url) if !url.is_empty() => format!("Fetch {url}"),
            _ => "Fetch web page".to_string(),
        },
        Some("todowrite") => "Update todo list".to_string(),
        Some("skill") => match string_arg("name") {
            Some(name) if !name.is_empty() => format!("Load skill {name}"),
            _ => "Load skill".to_string(),
        },
        _ => tc.name.clone(),
    }
}
