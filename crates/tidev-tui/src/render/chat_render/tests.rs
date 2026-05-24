use tidev_engine::config::{AppConfig, AuthStore};
use tidev_types::prompts::SessionMode;
use tidev_session::session::{Conversation, Message, MessageRole};
use tidev_engine::theme::ThemePalette;
use crate::App;
use crate::chat_render::RenderContext;
use crate::chat_render::tool::{
    render_tool_call_with_result, render_tool_result_detail_lines,
};
use crate::chat_render::utils::render_reasoning_markdown_lines;
use ratatui::style::Style;
use ratatui::text::Line;
use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};
use tempfile::TempDir;

fn line_text(line: &Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

fn text_lines_to_string(lines: &[Line<'static>]) -> String {
    lines.iter().map(line_text).collect::<Vec<_>>().join("\n")
}

/// Wraps App + TempDir so the temp directory is auto-cleaned on drop.
struct TestApp {
    app: App,
    _temp_root: TempDir,
}

impl Deref for TestApp {
    type Target = App;
    fn deref(&self) -> &App {
        &self.app
    }
}

impl DerefMut for TestApp {
    fn deref_mut(&mut self) -> &mut App {
        &mut self.app
    }
}

fn test_app() -> TestApp {
    let temp_root = TempDir::new().expect("temp dir should be created");
    let paths = tidev_engine::config::ConfigPaths {
        config_dir: temp_root.path().join(".config").join("tidev"),
        data_dir: temp_root.path().join(".local").join("share").join("tidev"),
        config_file: temp_root
            .path()
            .join(".config")
            .join("tidev")
            .join("config.toml"),
        auth_file: temp_root
            .path()
            .join(".local")
            .join("share")
            .join("tidev")
            .join("auth.json"),
        database_file: temp_root
            .path()
            .join(".local")
            .join("share")
            .join("tidev")
            .join("sessions.sqlite3"),
    };

    TestApp {
        app: App::new_with_paths(paths).unwrap(),
        _temp_root: temp_root,
    }
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
fn render_tool_result_detail_lines_todowrite_formats_checkbox_list() {
    use tidev_session::session::{Message, ToolExecutionResult};
    use tidev_engine::tooling::TodoItem;

    let todos = vec![
        TodoItem {
            content: "Task 1".to_string(),
            status: "completed".to_string(),
        },
        TodoItem {
            content: "Task 2".to_string(),
            status: "in_progress".to_string(),
        },
        TodoItem {
            content: "Task 3".to_string(),
            status: "pending".to_string(),
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
    assert!(text.contains("Task 1"), "should contain Task 1: {}", text);
    assert!(text.contains("Task 2"), "should contain Task 2: {}", text);
    assert!(text.contains("Task 3"), "should contain Task 3: {}", text);
}

#[test]
fn streaming_tool_call_switches_to_summary_after_arguments_parse() {
    use tidev_session::session::ToolCall;

    let tool_call = ToolCall {
        id: "tool-call-id".to_string(),
        name: "read".to_string(),
        arguments: "{\"file_path\": \"/tmp/example.txt\"}".to_string(),
        thought_signature: None,
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
fn render_question_result_pairs_shows_qa_formatted() {
    use crate::chat_render::tool::render_question_result_pairs;

    let output = "\
Q1: What scope do you prefer?
A: Global

Q2: What languages do you use?
A: Rust, Python";

    let lines = render_question_result_pairs(output, 80, ThemePalette::dark());
    let text = text_lines_to_string(&lines);

    assert!(
        text.contains("Questions & Answers"),
        "should have title: {}",
        text
    );
    assert!(
        text.contains("What scope do you prefer?"),
        "should show first question: {}",
        text
    );
    assert!(
        text.contains("Global"),
        "should show first answer: {}",
        text
    );
    assert!(
        text.contains("What languages do you use?"),
        "should show second question: {}",
        text
    );
    assert!(
        text.contains("Rust, Python"),
        "should show second answer: {}",
        text
    );
}

#[test]
fn render_question_result_pairs_fallback_on_empty_input() {
    use crate::chat_render::tool::render_question_result_pairs;

    // Completely empty output should show fallback
    let lines = render_question_result_pairs("", 80, ThemePalette::dark());
    let text = text_lines_to_string(&lines);
    assert!(
        text.contains("(no output)") || lines.len() <= 2,
        "empty should produce minimal output: {} lines",
        lines.len()
    );

    // Output that doesn't match Q: / A: format still renders gracefully
    let lines = render_question_result_pairs("something else", 80, ThemePalette::dark());
    assert!(
        lines.len() >= 2,
        "non-QA output should render: {}",
        lines.len()
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

    let (before, _, _, _, _, _, _, _) = app.messages_text(Some(80));
    let before_text = text_lines_to_string(&before.lines);
    assert!(before_text.contains("old cached content"));

    let message_id = app.conversation.messages[0].id;
    app.conversation.messages[0].content = "new refreshed content".to_string();
    app.invalidate_active_message_render_cache_for(message_id);

    let (after, _, _, _, _, _, _, _) = app.messages_text(Some(80));
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
            format!("message {idx}\n\n```rust\nfn item_{idx}() {{\n    println!(\"ok\");\n}}\n```"),
        ));
    }

    let (text, total_lines, _, _, _, _, _, _) = app.messages_text(Some(80));

    assert!(total_lines > 0);
    assert!(!text.lines.is_empty());
    assert!(text_lines_to_string(&text.lines).contains("message"));

    let max_scroll = total_lines.saturating_sub(app.message_viewport_lines.max(1));
    assert!(app.message_scroll_offset <= max_scroll);
}
