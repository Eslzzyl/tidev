#[cfg(test)]
mod tests {
    use crate::config::{AppConfig, AuthStore};
    use crate::prompts::SessionMode;
    use crate::session::{Conversation, Message, MessageRole};
    use crate::theme::ThemePalette;
    use crate::tui::App;
    use crate::tui::chat_render::RenderContext;
    use crate::tui::chat_render::tool::{
        render_tool_call_with_result, render_tool_result_detail_lines,
    };
    use crate::tui::chat_render::utils::render_reasoning_markdown_lines;
    use ratatui::style::Style;
    use ratatui::text::Line;
    use std::collections::{HashMap, HashSet};

    fn line_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn text_lines_to_string(lines: &[Line<'static>]) -> String {
        lines.iter().map(line_text).collect::<Vec<_>>().join("\n")
    }

    fn test_app() -> App {
        let temp_root =
            std::env::temp_dir().join(format!("tidev-render-tests-{}", uuid::Uuid::new_v4()));
        let paths = crate::config::ConfigPaths {
            config_dir: temp_root.join(".config").join("tidev"),
            data_dir: temp_root.join(".local").join("share").join("tidev"),
            config_file: temp_root.join(".config").join("tidev").join("config.toml"),
            auth_file: temp_root
                .join(".local")
                .join("share")
                .join("tidev")
                .join("auth.json"),
            database_file: temp_root
                .join(".local")
                .join("share")
                .join("tidev")
                .join("sessions.sqlite3"),
        };

        App::new_with_paths(paths).unwrap()
    }

    #[test]
    fn reasoning_lines_render_markdown_code_blocks() {
        let lines = render_reasoning_markdown_lines(
            "```rust\nfn main() { println!(\"hi\"); }\n```\n",
            80,
            None,
            ThemePalette::dark(),
        );

        assert_eq!(line_text(&lines[0]), "┃ Thinking:");
        assert_eq!(line_text(&lines[1]), "┃ fn main() { println!(\"hi\"); }");
        assert!(
            lines[1].spans.len() > 2,
            "expected highlighted spans in code line"
        );
        assert!(
            lines[1]
                .spans
                .iter()
                .skip(1)
                .any(|span| span.style != Style::default()),
            "expected syntax highlighting styles on code spans"
        );
    }

    #[test]
    fn reasoning_lines_preserve_empty_state() {
        let lines = render_reasoning_markdown_lines("", 80, None, ThemePalette::dark());

        assert_eq!(lines.len(), 1);
        assert_eq!(line_text(&lines[0]), "┃ Thinking:");
    }

    #[test]
    fn render_tool_result_detail_lines_list_shows_output_preview() {
        use crate::session::{Message, ToolExecutionResult};

        let message = Message::tool_result(
            "tool-call-id",
            "list",
            ToolExecutionResult::new("./\nfile1.txt\nfile2.txt"),
        );

        let ctx = RenderContext {
            palette: ThemePalette::dark(),
            spinner: "|",
            workspace_root: std::path::Path::new("/tmp"),
            expanded_tool_results: &HashSet::new(),
            expanded_tool_outputs: &HashMap::new(),
            config: &AppConfig::default(),
            auth: &AuthStore::default(),
            conversation: &Conversation::new(
                uuid::Uuid::new_v4(),
                "/tmp",
                "test",
                "test",
                "test",
                "test",
                "test",
            ),
            mode: SessionMode::Build,
        };

        let (lines, _, _) = render_tool_result_detail_lines(&message, 80, &ctx);
        let text = text_lines_to_string(&lines);
        assert!(
            text.contains("file1.txt"),
            "should contain file listing: {}",
            text
        );
    }

    #[test]
    fn render_tool_result_detail_lines_todowrite_formats_checkbox_list() {
        use crate::session::{Message, ToolExecutionResult};
        use crate::tooling::TodoItem;

        let todos = vec![
            TodoItem {
                content: "Task 1".to_string(),
                status: "completed".to_string(),
                priority: "high".to_string(),
            },
            TodoItem {
                content: "Task 2".to_string(),
                status: "in_progress".to_string(),
                priority: "medium".to_string(),
            },
            TodoItem {
                content: "Task 3".to_string(),
                status: "pending".to_string(),
                priority: "low".to_string(),
            },
        ];
        let output = serde_json::to_string_pretty(&todos).unwrap();
        let message = Message::tool_result(
            "tool-call-id",
            "todowrite",
            ToolExecutionResult::new(output),
        );

        let ctx = RenderContext {
            palette: ThemePalette::dark(),
            spinner: "|",
            workspace_root: std::path::Path::new("/tmp"),
            expanded_tool_results: &HashSet::new(),
            expanded_tool_outputs: &HashMap::new(),
            config: &AppConfig::default(),
            auth: &AuthStore::default(),
            conversation: &Conversation::new(
                uuid::Uuid::new_v4(),
                "/tmp",
                "test",
                "test",
                "test",
                "test",
                "test",
            ),
            mode: SessionMode::Build,
        };

        let (lines, _, _) = render_tool_result_detail_lines(&message, 80, &ctx);

        let text = text_lines_to_string(&lines);
        assert!(
            text.contains("Updated todo list"),
            "should contain header: {}",
            text
        );
        assert!(text.contains("Task 1"), "should contain Task 1: {}", text);
        assert!(text.contains("Task 2"), "should contain Task 2: {}", text);
        assert!(text.contains("Task 3"), "should contain Task 3: {}", text);
    }

    #[test]
    fn streaming_tool_call_switches_to_summary_after_arguments_parse() {
        use crate::session::ToolCall;

        let tool_call = ToolCall {
            id: "tool-call-id".to_string(),
            name: "read".to_string(),
            arguments: "{\"path\": \"/tmp/example.txt\"}".to_string(),
        };

        let ctx = RenderContext {
            palette: ThemePalette::dark(),
            spinner: "|",
            workspace_root: std::path::Path::new("/tmp"),
            expanded_tool_results: &HashSet::new(),
            expanded_tool_outputs: &HashMap::new(),
            config: &AppConfig::default(),
            auth: &AuthStore::default(),
            conversation: &Conversation::new(
                uuid::Uuid::new_v4(),
                "/tmp",
                "test",
                "test",
                "test",
                "test",
                "test",
            ),
            mode: SessionMode::Build,
        };

        let (lines, _) = render_tool_call_with_result(&tool_call, None, 80, true, &ctx);
        let text = text_lines_to_string(&lines);

        assert!(
            text.contains("Read example.txt"),
            "should show parsed summary: {}",
            text
        );
        assert!(
            !text.contains("Calling..."),
            "pending state should be replaced: {}",
            text
        );
    }

    #[test]
    fn message_render_cache_hits_on_second_render_same_width() {
        let mut app = test_app();
        app.conversation
            .push(Message::new(MessageRole::User, "show file list"));
        app.conversation.push(Message::new(
            MessageRole::Assistant,
            "Summary with **markdown** and `inline code`.",
        ));

        let _ = app.messages_text(Some(80));
        let (_, misses_before, entries_before) = app.message_render_cache_stats();

        // Parallel path pre-populates the cache eagerly, so there are no misses.
        // But entries should be in the cache.
        assert!(entries_before >= 2, "first render should populate cache");

        let _ = app.messages_text(Some(80));
        let (hits_after, misses_after, entries_after) = app.message_render_cache_stats();

        assert!(hits_after > 0, "second render should have cache hits");
        assert_eq!(
            misses_after, misses_before,
            "second render should use cache (no new misses)"
        );
        assert_eq!(entries_after, entries_before, "cache size should be stable");
    }

    #[test]
    fn message_render_cache_width_change_causes_miss() {
        let mut app = test_app();
        app.conversation
            .push(Message::new(MessageRole::User, "open README"));
        app.conversation.push(Message::new(
            MessageRole::Assistant,
            "A longer paragraph that should wrap differently at another width.",
        ));

        let _ = app.messages_text(Some(72));
        let (_, _, entries_before) = app.message_render_cache_stats();

        let _ = app.messages_text(Some(100));
        let (_, _, entries_after) = app.message_render_cache_stats();

        // Width change triggers full rebuild, re-populating cache.
        // The parallel path pre-populates eagerly, so new entries are inserted.
        assert!(
            entries_after > entries_before,
            "cache should have new entries for the new width"
        );
    }

    #[test]
    fn message_render_cache_invalidation_refreshes_updated_content() {
        let mut app = test_app();
        app.conversation
            .push(Message::new(MessageRole::Assistant, "old cached content"));

        let (before, _, _, _, _, _, _) = app.messages_text(Some(80));
        let before_text = text_lines_to_string(&before.lines);
        assert!(before_text.contains("old cached content"));

        let message_id = app.conversation.messages[0].id;
        app.conversation.messages[0].content = "new refreshed content".to_string();
        app.invalidate_active_message_render_cache_for(message_id);

        let (after, _, _, _, _, _, _) = app.messages_text(Some(80));
        let after_text = text_lines_to_string(&after.lines);
        assert!(after_text.contains("new refreshed content"));
    }

    #[test]
    fn virtualized_render_clamps_scroll_and_keeps_content_visible() {
        let mut app = test_app();
        app.message_viewport_lines = 8;
        app.message_follow_tail = false;
        app.message_scroll_offset = usize::MAX;

        for idx in 0..24 {
            app.conversation.push(Message::new(
                MessageRole::Assistant,
                format!(
                    "message {idx}\n\n```rust\nfn item_{idx}() {{\n    println!(\"ok\");\n}}\n```"
                ),
            ));
        }

        let (text, total_lines, _, _, _, _, _) = app.messages_text(Some(80));

        assert!(total_lines > 0);
        assert!(!text.lines.is_empty());
        assert!(text_lines_to_string(&text.lines).contains("message"));

        let max_scroll = total_lines.saturating_sub(app.message_viewport_lines.max(1));
        assert!(app.message_scroll_offset <= max_scroll);
    }
}
