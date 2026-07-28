//! Type conversions between tidev internal types and ACP v1 schema types.

use agent_client_protocol::schema::v1 as acp;
use tidev_types::message::{MessageAttachment, ToolCall, ToolExecutionResult};
use tidev_types::tools::canonical_tool_name;

// ---------------------------------------------------------------------------
// Title helpers — matches TUI style (bare infinitive, no emoji)
// ---------------------------------------------------------------------------

/// Human-readable title for a tool call, matching TUI conventions.
pub fn tool_title(tc: &ToolCall) -> String {
    let args: Option<serde_json::Value> = serde_json::from_str(&tc.arguments).ok();
    let s = |key: &str| {
        args.as_ref()
            .and_then(|v| v.get(key))
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    };

    match canonical_tool_name(&tc.name) {
        Some("read") => {
            let path = s("file_path").or_else(|| s("path")).unwrap_or_default();
            if path.is_empty() {
                "Read file".into()
            } else {
                format!("Read {}", path)
            }
        }
        Some("write") => {
            let path = s("file_path").unwrap_or_default();
            if path.is_empty() {
                "Write file".into()
            } else {
                format!("Write {}", path)
            }
        }
        Some("edit") => {
            let path = s("file_path").unwrap_or_default();
            if path.is_empty() {
                "Edit file".into()
            } else {
                format!("Edit {}", path)
            }
        }
        Some("apply_patch") => "Apply patch".into(),
        Some("shell") => {
            let desc = s("description");
            let command = s("command");
            let display = desc.or(command).unwrap_or_default();
            if display.is_empty() {
                "Shell".into()
            } else {
                format!("Shell {}", display)
            }
        }
        Some("glob") => {
            let pattern = s("pattern").unwrap_or_default();
            if pattern.is_empty() {
                "Search files".into()
            } else {
                format!("Glob {}", pattern)
            }
        }
        Some("grep") => {
            let pattern = s("pattern").unwrap_or_default();
            if pattern.is_empty() {
                "Search files".into()
            } else {
                format!("Grep {}", pattern)
            }
        }
        Some("task") => {
            let desc = s("description").unwrap_or_default();
            if desc.is_empty() {
                "Delegate task".into()
            } else {
                format!("Task: {}", desc)
            }
        }
        Some("question") => {
            let count = args
                .as_ref()
                .and_then(|v| v.get("questions"))
                .and_then(|a| a.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            if count <= 1 {
                "Ask 1 question".into()
            } else {
                format!("Ask {} questions", count)
            }
        }
        Some("websearch") => {
            let query = s("query").unwrap_or_default();
            if query.is_empty() {
                "Search web".into()
            } else {
                format!("Search web for {}", query)
            }
        }
        Some("webfetch") => {
            let url = s("url").unwrap_or_default();
            if url.is_empty() {
                "Fetch web page".into()
            } else {
                format!("Fetch {}", url)
            }
        }
        Some("todowrite") => "Update todo list".into(),
        Some("skill") => {
            let name = s("name").unwrap_or_default();
            if name.is_empty() {
                "Load skill".into()
            } else {
                format!("Load skill {}", name)
            }
        }
        _ => tc.name.clone(),
    }
}

// ---------------------------------------------------------------------------
// Kind mapper — reused from tidev_tool_call_to_acp
// ---------------------------------------------------------------------------

pub(crate) fn tool_kind(tc: &ToolCall) -> acp::ToolKind {
    match canonical_tool_name(&tc.name) {
        Some("shell") | Some("exec") => acp::ToolKind::Execute,
        Some("read") | Some("glob") | Some("grep") => acp::ToolKind::Read,
        Some("write") | Some("edit") | Some("apply_patch") => acp::ToolKind::Edit,
        Some("websearch") | Some("webfetch") => acp::ToolKind::Fetch,
        Some("task") | Some("question") | Some("todowrite") | Some("skill") => acp::ToolKind::Other,
        _ => acp::ToolKind::Other,
    }
}

// ---------------------------------------------------------------------------
// Locations — extract file_path from tools that operate on files
// ---------------------------------------------------------------------------

/// Build ACP [`ToolCallLocation`] list from a tool call's arguments.
pub fn tool_locations(tc: &ToolCall) -> Vec<acp::ToolCallLocation> {
    let args: serde_json::Value = match serde_json::from_str(&tc.arguments) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let path = match canonical_tool_name(&tc.name) {
        Some("read") | Some("write") | Some("edit") => args
            .get("file_path")
            .or_else(|| args.get("path"))
            .and_then(|v| v.as_str()),
        _ => None,
    };

    match path {
        Some(p) if !p.is_empty() => vec![acp::ToolCallLocation::new(p.to_string())],
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------
// Diff content from write / edit / apply_patch results
// ---------------------------------------------------------------------------

/// Parse a unified diff string into old_text and new_text.
///
/// Handles single-file unified diffs produced by `diffy`. For multi-file
/// patches only the first file's content is returned; callers should iterate
/// `file_changes` for `apply_patch`.
pub fn parse_unified_diff(patch: &str) -> (String, String) {
    let mut old_lines: Vec<&str> = Vec::new();
    let mut new_lines: Vec<&str> = Vec::new();
    let mut in_hunk = false;

    for line in patch.lines() {
        if line.starts_with("@@ ") {
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            continue;
        }
        let b = line.as_bytes();
        if b.first() == Some(&b'-') {
            old_lines.push(&line[1..]);
        } else if b.first() == Some(&b'+') {
            new_lines.push(&line[1..]);
        } else if b.first() == Some(&b' ') {
            old_lines.push(&line[1..]);
            new_lines.push(&line[1..]);
        }
    }

    (old_lines.join("\n"), new_lines.join("\n"))
}

/// Convert a [`ToolExecutionResult`] into `Diff` content items.
///
/// - `write` / `edit`: single `Diff` from `metadata.diff` and `metadata.filepath`.
/// - `apply_patch`: one `Diff` per entry in `metadata.file_changes`.
pub fn tidev_result_to_diff_content(
    tool_call: &ToolCall,
    result: &ToolExecutionResult,
) -> Vec<acp::ToolCallContent> {
    let canonical = match canonical_tool_name(&tool_call.name) {
        Some(n) => n,
        None => return vec![],
    };

    match canonical {
        "write" | "edit" => {
            let diff_str = match &result.metadata.diff {
                Some(d) => d,
                None => return vec![],
            };
            let path = result.metadata.filepath.as_deref().unwrap_or("<unknown>");
            let (old_text, new_text) = parse_unified_diff(diff_str);
            vec![acp::ToolCallContent::Diff(
                acp::Diff::new(path, new_text).old_text(old_text),
            )]
        }
        "apply_patch" => result
            .metadata
            .file_changes
            .iter()
            .filter_map(|fc| {
                let diff_str = fc.diff.as_ref()?;
                let (old_text, new_text) = parse_unified_diff(diff_str);
                Some(acp::ToolCallContent::Diff(
                    acp::Diff::new(fc.path.clone(), new_text).old_text(old_text),
                ))
            })
            .collect(),
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------
// Image attachments → ContentBlock::Image
// ---------------------------------------------------------------------------

/// Convert tidev image attachments into ACP content blocks.
pub fn tidev_attachments_to_content(
    attachments: &[MessageAttachment],
) -> Vec<acp::ToolCallContent> {
    use base64::Engine as _;
    attachments
        .iter()
        .filter_map(|a| match a {
            MessageAttachment::Image { mime, data, .. } => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(data);
                Some(acp::ToolCallContent::Content(acp::Content::new(
                    acp::ContentBlock::Image(acp::ImageContent::new(mime.clone(), b64)),
                )))
            }
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Rich update builders
// ---------------------------------------------------------------------------

/// Build a `tool_call_update` with full metadata for a just-started tool.
pub fn tool_starting_update_rich(tc: &ToolCall) -> acp::ToolCallUpdate {
    let raw_input: Option<serde_json::Value> = serde_json::from_str(&tc.arguments).ok();

    let fields = acp::ToolCallUpdateFields::new()
        .status(acp::ToolCallStatus::InProgress)
        .title(Some(tool_title(tc)))
        .kind(Some(tool_kind(tc)))
        .locations(tool_locations(tc))
        .raw_input(raw_input);

    acp::ToolCallUpdate::new(tc.id.clone(), fields)
}

/// Build a `tool_call_update` marking completion, with raw_output metadata.
pub fn tool_completed_update_rich(
    tc: &ToolCall,
    result: &ToolExecutionResult,
) -> acp::ToolCallUpdate {
    let raw_output = serde_json::json!({
        "truncated": result.output.len() > 100_000,
        "output_length": result.output.len(),
    });

    acp::ToolCallUpdate::new(
        tc.id.clone(),
        acp::ToolCallUpdateFields::new()
            .status(acp::ToolCallStatus::Completed)
            .raw_output(Some(raw_output)),
    )
}

// ---------------------------------------------------------------------------
// Original functions (kept for backward compatibility)
// ---------------------------------------------------------------------------

/// Convert a tidev [`ToolCall`] to an ACP [`ToolCall`].
pub fn tidev_tool_call_to_acp(tc: &ToolCall) -> acp::ToolCall {
    let kind = tool_kind(tc);
    let raw_input: Option<serde_json::Value> = serde_json::from_str(&tc.arguments).ok();

    acp::ToolCall::new(tc.id.clone(), &tc.name)
        .kind(kind)
        .raw_input(raw_input)
}

/// Convert a tidev [`ToolCall`] to an ACP [`ToolCallUpdate`] with optional status.
pub fn tidev_tool_call_to_acp_update(
    tc: &ToolCall,
    status: Option<acp::ToolCallStatus>,
) -> acp::ToolCallUpdate {
    let fields = acp::ToolCallUpdateFields::new().status(status);
    acp::ToolCallUpdate::new(tc.id.clone(), fields)
}

/// Convert a tidev [`ToolExecutionResult`] to ACP [`ToolCallContent`] items.
pub fn tidev_tool_result_to_acp_content(result: &ToolExecutionResult) -> Vec<acp::ToolCallContent> {
    vec![acp::ToolCallContent::Content(acp::Content::new(
        acp::ContentBlock::Text(acp::TextContent::new(&result.output)),
    ))]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tc(name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: "tc-1".into(),
            name: name.into(),
            arguments: args.into(),
            thought_signature: None,
        }
    }

    fn make_result(output: &str) -> ToolExecutionResult {
        ToolExecutionResult::new(output)
    }

    // ── tool_title ───────────────────────────────────────────────────────

    #[test]
    fn title_read_with_path() {
        let tc = make_tc("read", r#"{"file_path":"Cargo.toml"}"#);
        assert_eq!(tool_title(&tc), "Read Cargo.toml");
    }

    #[test]
    fn title_read_with_path_alias() {
        let tc = make_tc("read", r#"{"path":"src/main.rs"}"#);
        assert_eq!(tool_title(&tc), "Read src/main.rs");
    }

    #[test]
    fn title_read_no_path() {
        let tc = make_tc("read", "{}");
        assert_eq!(tool_title(&tc), "Read file");
    }

    #[test]
    fn title_write_with_path() {
        let tc = make_tc("write", r#"{"file_path":"Cargo.toml"}"#);
        assert_eq!(tool_title(&tc), "Write Cargo.toml");
    }

    #[test]
    fn title_edit_with_path() {
        let tc = make_tc("edit", r#"{"file_path":"src/lib.rs"}"#);
        assert_eq!(tool_title(&tc), "Edit src/lib.rs");
    }

    #[test]
    fn title_apply_patch() {
        let tc = make_tc("apply_patch", r#"{"patch_text":"..."}"#);
        assert_eq!(tool_title(&tc), "Apply patch");
    }

    #[test]
    fn title_shell_with_description() {
        let tc = make_tc(
            "shell",
            r#"{"command":"cargo test","description":"Run unit tests"}"#,
        );
        assert_eq!(tool_title(&tc), "Shell Run unit tests");
    }

    #[test]
    fn title_shell_command_only() {
        let tc = make_tc("shell", r#"{"command":"cargo build"}"#);
        assert_eq!(tool_title(&tc), "Shell cargo build");
    }

    #[test]
    fn title_shell_empty() {
        let tc = make_tc("shell", "{}");
        assert_eq!(tool_title(&tc), "Shell");
    }

    #[test]
    fn title_glob_with_pattern() {
        let tc = make_tc("glob", r#"{"pattern":"**/*.rs"}"#);
        assert_eq!(tool_title(&tc), "Glob **/*.rs");
    }

    #[test]
    fn title_grep_with_pattern() {
        let tc = make_tc("grep", r#"{"pattern":"fn main"}"#);
        assert_eq!(tool_title(&tc), "Grep fn main");
    }

    #[test]
    fn title_task_with_description() {
        let tc = make_tc("task", r#"{"description":"explore X","prompt":"..."}"#);
        assert_eq!(tool_title(&tc), "Task: explore X");
    }

    #[test]
    fn title_question_single() {
        let tc = make_tc("question", r#"{"questions":[{"question":"q1"}]}"#);
        assert_eq!(tool_title(&tc), "Ask 1 question");
    }

    #[test]
    fn title_question_multiple() {
        let tc = make_tc(
            "question",
            r#"{"questions":[{"question":"q1"},{"question":"q2"}]}"#,
        );
        assert_eq!(tool_title(&tc), "Ask 2 questions");
    }

    #[test]
    fn title_websearch_with_query() {
        let tc = make_tc("websearch", r#"{"query":"Rust async"}"#);
        assert_eq!(tool_title(&tc), "Search web for Rust async");
    }

    #[test]
    fn title_webfetch_with_url() {
        let tc = make_tc("webfetch", r#"{"url":"https://example.com"}"#);
        assert_eq!(tool_title(&tc), "Fetch https://example.com");
    }

    #[test]
    fn title_todowrite() {
        let tc = make_tc("todowrite", r#"{"todos":[]}"#);
        assert_eq!(tool_title(&tc), "Update todo list");
    }

    #[test]
    fn title_skill_with_name() {
        let tc = make_tc("skill", r#"{"name":"debug"}"#);
        assert_eq!(tool_title(&tc), "Load skill debug");
    }

    #[test]
    fn title_unknown_tool() {
        let tc = make_tc("custom_tool", r#"{"arg":"val"}"#);
        assert_eq!(tool_title(&tc), "custom_tool");
    }

    // ── tool_locations ───────────────────────────────────────────────────

    #[test]
    fn locations_read_with_file_path() {
        let tc = make_tc("read", r#"{"file_path":"src/main.rs"}"#);
        let locs = tool_locations(&tc);
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].path.to_string_lossy(), "src/main.rs");
    }

    #[test]
    fn locations_read_with_path_alias() {
        let tc = make_tc("read", r#"{"path":"Cargo.toml"}"#);
        let locs = tool_locations(&tc);
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].path.to_string_lossy(), "Cargo.toml");
    }

    #[test]
    fn locations_shell_returns_empty() {
        let tc = make_tc("shell", r#"{"command":"ls"}"#);
        let locs = tool_locations(&tc);
        assert!(locs.is_empty());
    }

    #[test]
    fn locations_write_with_file_path() {
        let tc = make_tc("write", r#"{"file_path":"output.txt"}"#);
        let locs = tool_locations(&tc);
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].path.to_string_lossy(), "output.txt");
    }

    #[test]
    fn locations_edit_with_file_path() {
        let tc = make_tc("edit", r#"{"file_path":"src/lib.rs"}"#);
        let locs = tool_locations(&tc);
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].path.to_string_lossy(), "src/lib.rs");
    }

    #[test]
    fn locations_invalid_json_returns_empty() {
        let tc = make_tc("read", "not-json");
        let locs = tool_locations(&tc);
        assert!(locs.is_empty());
    }

    #[test]
    fn locations_empty_path_returns_empty() {
        let tc = make_tc("read", r#"{"file_path":""}"#);
        let locs = tool_locations(&tc);
        assert!(locs.is_empty());
    }

    // ── parse_unified_diff ───────────────────────────────────────────────

    #[test]
    fn parse_unified_diff_simple() {
        let patch = "\
@@ -1,3 +1,4 @@
-foo
+bar
 baz
";
        let (old, new) = parse_unified_diff(patch);
        assert_eq!(old, "foo\nbaz");
        assert_eq!(new, "bar\nbaz");
    }

    #[test]
    fn parse_unified_diff_no_hunks() {
        let patch = "--- a/file\n+++ b/file\n";
        let (old, new) = parse_unified_diff(patch);
        assert!(old.is_empty());
        assert!(new.is_empty());
    }

    // ── tidev_result_to_diff_content ─────────────────────────────────────

    #[test]
    fn diff_content_write() {
        let tc = make_tc("write", r#"{"file_path":"test.txt","content":"hi"}"#);
        let patch = "\
@@ -0,0 +1 @@
+hi
";
        let mut result = make_result("Write test.txt");
        result.metadata.diff = Some(patch.into());
        result.metadata.filepath = Some("test.txt".into());

        let content = tidev_result_to_diff_content(&tc, &result);
        assert_eq!(content.len(), 1);
        match &content[0] {
            acp::ToolCallContent::Diff(d) => {
                assert_eq!(d.path.to_string_lossy(), "test.txt");
                assert_eq!(d.new_text, "hi");
            }
            _ => panic!("expected Diff"),
        }
    }

    #[test]
    fn diff_content_edit() {
        let tc = make_tc(
            "edit",
            r#"{"file_path":"src/lib.rs","old_text":"foo","new_text":"bar"}"#,
        );
        let patch = "\
@@ -1,1 +1,1 @@
-foo
+bar
";
        let mut result = make_result("Edit src/lib.rs");
        result.metadata.diff = Some(patch.into());
        result.metadata.filepath = Some("src/lib.rs".into());

        let content = tidev_result_to_diff_content(&tc, &result);
        assert_eq!(content.len(), 1);
        match &content[0] {
            acp::ToolCallContent::Diff(d) => {
                assert_eq!(d.path.to_string_lossy(), "src/lib.rs");
                assert_eq!(d.old_text.as_deref(), Some("foo"));
                assert_eq!(d.new_text, "bar");
            }
            _ => panic!("expected Diff"),
        }
    }

    #[test]
    fn diff_content_no_diff_falls_back_empty() {
        let tc = make_tc("write", r#"{"file_path":"x"}"#);
        let result = make_result("Write x");
        let content = tidev_result_to_diff_content(&tc, &result);
        assert!(content.is_empty());
    }

    #[test]
    fn diff_content_non_file_tool_returns_empty() {
        let tc = make_tc("shell", r#"{"command":"ls"}"#);
        let result = make_result("ok");
        let content = tidev_result_to_diff_content(&tc, &result);
        assert!(content.is_empty());
    }

    // ── tool_starting_update_rich ────────────────────────────────────────

    #[test]
    fn starting_update_rich_has_title_kind_locations() {
        let tc = make_tc("read", r#"{"file_path":"Cargo.toml"}"#);
        let update = tool_starting_update_rich(&tc);
        assert_eq!(update.fields.status, Some(acp::ToolCallStatus::InProgress));
        assert_eq!(update.fields.title.as_deref(), Some("Read Cargo.toml"));
        assert_eq!(update.fields.kind, Some(acp::ToolKind::Read));
        assert!(update.fields.locations.unwrap_or_default().len() == 1);
    }

    #[test]
    fn starting_update_rich_has_raw_input() {
        let tc = make_tc("read", r#"{"file_path":"Cargo.toml"}"#);
        let update = tool_starting_update_rich(&tc);
        let raw = update.fields.raw_input.expect("raw_input should be Some");
        assert_eq!(
            raw.get("file_path").and_then(|v| v.as_str()),
            Some("Cargo.toml")
        );
    }

    #[test]
    fn starting_update_rich_invalid_args_no_raw_input() {
        let tc = make_tc("read", "not-json");
        let update = tool_starting_update_rich(&tc);
        assert!(update.fields.raw_input.is_none());
    }

    // ── tool_completed_update_rich ───────────────────────────────────────

    #[test]
    fn completed_update_rich_has_status_and_raw_output() {
        let tc = make_tc("read", "{}");
        let result = make_result("hello");
        let update = tool_completed_update_rich(&tc, &result);
        assert_eq!(update.fields.status, Some(acp::ToolCallStatus::Completed));
        let raw = update.fields.raw_output.expect("raw_output should be Some");
        assert_eq!(raw.get("output_length").and_then(|v| v.as_u64()), Some(5));
    }

    // ── existing tests (unchanged) ───────────────────────────────────────

    #[test]
    fn to_acp_kind_execute() {
        let tc = make_tc("shell", r#"{"cmd":"ls"}"#);
        let acp_tc = tidev_tool_call_to_acp(&tc);
        assert_eq!(acp_tc.tool_call_id.to_string(), "tc-1");
        assert_eq!(acp_tc.title, "shell");
        assert_eq!(acp_tc.kind, acp::ToolKind::Execute);
    }

    #[test]
    fn to_acp_kind_read() {
        let tc = make_tc("read", r#"{"path":"x"}"#);
        let acp_tc = tidev_tool_call_to_acp(&tc);
        assert_eq!(acp_tc.kind, acp::ToolKind::Read);
    }

    #[test]
    fn to_acp_kind_edit() {
        let tc = make_tc("write", r#"{"path":"x","content":"hi"}"#);
        let acp_tc = tidev_tool_call_to_acp(&tc);
        assert_eq!(acp_tc.kind, acp::ToolKind::Edit);
    }

    #[test]
    fn to_acp_kind_fetch() {
        let tc = make_tc("websearch", r#"{"query":"rust"}"#);
        let acp_tc = tidev_tool_call_to_acp(&tc);
        assert_eq!(acp_tc.kind, acp::ToolKind::Fetch);
    }

    #[test]
    fn to_acp_kind_other_fallback() {
        let tc = make_tc("custom_tool", r#"{}"#);
        let acp_tc = tidev_tool_call_to_acp(&tc);
        assert_eq!(acp_tc.kind, acp::ToolKind::Other);
    }

    #[test]
    fn to_acp_raw_input_valid_json() {
        let tc = make_tc("read", r#"{"path":"Cargo.toml"}"#);
        let acp_tc = tidev_tool_call_to_acp(&tc);
        let raw = acp_tc.raw_input.expect("raw_input should be Some");
        assert_eq!(raw.get("path").and_then(|v| v.as_str()), Some("Cargo.toml"));
    }

    #[test]
    fn to_acp_raw_input_invalid_json() {
        let tc = make_tc("read", "not-json");
        let acp_tc = tidev_tool_call_to_acp(&tc);
        assert!(
            acp_tc.raw_input.is_none(),
            "invalid JSON should produce None"
        );
    }

    #[test]
    fn to_acp_update_no_status() {
        let tc = make_tc("read", "{}");
        let update = tidev_tool_call_to_acp_update(&tc, None);
        assert_eq!(update.tool_call_id.to_string(), "tc-1");
        assert_eq!(update.fields.status, None);
    }

    #[test]
    fn to_acp_update_in_progress() {
        let tc = make_tc("read", "{}");
        let update = tidev_tool_call_to_acp_update(&tc, Some(acp::ToolCallStatus::InProgress));
        assert_eq!(update.fields.status, Some(acp::ToolCallStatus::InProgress));
    }

    #[test]
    fn to_acp_update_completed() {
        let tc = make_tc("read", "{}");
        let update = tidev_tool_call_to_acp_update(&tc, Some(acp::ToolCallStatus::Completed));
        assert_eq!(update.fields.status, Some(acp::ToolCallStatus::Completed));
    }

    #[test]
    fn result_to_content_contains_output_text() {
        let result = ToolExecutionResult::new("hello world");
        let content = tidev_tool_result_to_acp_content(&result);
        assert_eq!(content.len(), 1);
        match &content[0] {
            acp::ToolCallContent::Content(c) => match &c.content {
                acp::ContentBlock::Text(t) => assert_eq!(t.text, "hello world"),
                _ => panic!("expected Text"),
            },
            _ => panic!("expected ToolCallContent::Content"),
        }
    }

    #[test]
    fn result_to_content_empty_output() {
        let result = ToolExecutionResult::new("");
        let content = tidev_tool_result_to_acp_content(&result);
        assert_eq!(content.len(), 1);
    }
}
