use super::*;
use crate::database::Database;
use tempfile::TempDir;

type RawMessageFields = (Vec<u8>, Option<Vec<u8>>, Vec<u8>, Vec<u8>, Vec<u8>);

fn test_store() -> (SessionStore, TempDir) {
    let tmp = TempDir::new().unwrap();
    let db = Database::open(tmp.path().join("test.db")).unwrap();
    let store = db.create_store().unwrap();
    (store, tmp)
}

fn create_test_session(store: &SessionStore, workspace: &str, title: &str) -> Uuid {
    let id = Uuid::new_v4();
    store
        .create_session(
            id,
            workspace,
            "deepseek",
            "DeepSeek",
            "deepseek-v4-flash",
            "DeepSeek-V4-Flash",
            title,
            None,
            None,
        )
        .unwrap();
    id
}

#[test]
fn session_create_and_load_round_trip() {
    let (store, _tmp) = test_store();
    let id = create_test_session(&store, "/workspace", "Test session");

    let loaded = store
        .load_session(id)
        .unwrap()
        .expect("session should exist");
    assert_eq!(loaded.title, "Test session");
    assert_eq!(loaded.provider_id, "deepseek");
    assert_eq!(loaded.model_id, "deepseek-v4-flash");
    assert_eq!(loaded.workspace_root, "/workspace");
    assert_eq!(loaded.status, "active");
    assert!(loaded.parent_session_id.is_none());
}

#[test]
fn context_state_round_trip() {
    let (store, _tmp) = test_store();
    let id = create_test_session(&store, "/workspace", "ctx test");

    store
        .update_session(
            id,
            None,
            None,
            Some("Summary: refactored main.rs"),
            Some(7),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

    let loaded = store.load_session(id).unwrap().unwrap();
    assert_eq!(
        loaded.context_summary.as_deref(),
        Some("Summary: refactored main.rs")
    );
    assert_eq!(loaded.context_retained_from, 7);
}

#[test]
fn parent_child_session_relationship() {
    let (store, _tmp) = test_store();
    let parent = create_test_session(&store, "/workspace", "Parent");
    let child = Uuid::new_v4();
    store
        .create_session(
            child,
            "/workspace",
            "deepseek",
            "DeepSeek",
            "deepseek-v4-flash",
            "DeepSeek-V4-Flash",
            "Child",
            Some(parent),
            None,
        )
        .unwrap();

    let loaded = store.load_session(child).unwrap().unwrap();
    assert_eq!(loaded.parent_session_id, Some(parent));
}

#[test]
fn child_sessions_excluded_from_listing() {
    let (store, _tmp) = test_store();
    let parent = create_test_session(&store, "/workspace", "Parent");
    let child = Uuid::new_v4();
    store
        .create_session(
            child,
            "/workspace",
            "deepseek",
            "DeepSeek",
            "deepseek-v4-flash",
            "DeepSeek-V4-Flash",
            "Child",
            Some(parent),
            None,
        )
        .unwrap();

    let sessions = store.list_sessions(10, 0).unwrap();
    let ids: Vec<Uuid> = sessions.iter().map(|s| s.session_id).collect();
    assert!(ids.contains(&parent));
    assert!(!ids.contains(&child));
}

#[test]
fn workspace_session_scoping() {
    let (store, _tmp) = test_store();
    let _a = create_test_session(&store, "/ws-a", "A");
    let _b = create_test_session(&store, "/ws-b", "B");

    let ws_a_sessions = store.list_sessions_for_workspace("/ws-a", 10, 0).unwrap();
    assert_eq!(ws_a_sessions.len(), 1);
    assert_eq!(ws_a_sessions[0].title, "A");

    let ws_b_sessions = store.list_sessions_for_workspace("/ws-b", 10, 0).unwrap();
    assert_eq!(ws_b_sessions.len(), 1);
    assert_eq!(ws_b_sessions[0].title, "B");
}

#[test]
fn session_id_prefix_lookup_returns_matching_sessions() {
    let (store, _tmp) = test_store();
    let first = Uuid::parse_str("a1b2c3d4-e5f6-4789-abcd-0123456789ab").unwrap();
    let second = Uuid::parse_str("a1b2c3d4-e5f6-4789-abcd-abcdefabcdef").unwrap();
    store
        .create_session(
            first,
            "/workspace",
            "deepseek",
            "DeepSeek",
            "deepseek-v4-flash",
            "DeepSeek-V4-Flash",
            "First",
            None,
            None,
        )
        .unwrap();
    store
        .create_session(
            second,
            "/workspace",
            "deepseek",
            "DeepSeek",
            "deepseek-v4-flash",
            "DeepSeek-V4-Flash",
            "Second",
            None,
            None,
        )
        .unwrap();

    let matches = store.find_session_ids_by_prefix("A1B2C3D4E5F6").unwrap();
    assert_eq!(matches.len(), 2);
    assert!(matches.contains(&first));
    assert!(matches.contains(&second));
}

#[test]
fn activity_listing_filters_and_paginates_top_level_sessions() {
    let (store, _tmp) = test_store();
    let _tidev = create_test_session(&store, "/work/tidev", "Fix sidebar");
    let _vnagent = create_test_session(&store, "/work/vnagent", "Review storyboard");
    let _other = create_test_session(&store, "/work/fundlab", "Run research");

    let matching = store
        .list_sessions_by_activity(10, None, Some("tidev"), None)
        .unwrap();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].workspace_root, "/work/tidev");

    let scoped = store
        .list_sessions_by_activity(10, None, None, Some("/work/vnagent"))
        .unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].title, "Review storyboard");

    let first_page = store
        .list_sessions_by_activity(1, None, None, None)
        .unwrap();
    assert_eq!(first_page.len(), 1);
    let cursor = (first_page[0].updated_at, first_page[0].session_id);
    let second_page = store
        .list_sessions_by_activity(10, Some(cursor), None, None)
        .unwrap();
    assert_eq!(second_page.len(), 2);
    assert!(
        second_page
            .iter()
            .all(|session| session.session_id != first_page[0].session_id)
    );

    let roots = store.list_session_workspace_roots().unwrap();
    assert_eq!(roots, vec!["/work/fundlab", "/work/vnagent", "/work/tidev"]);
}

#[test]
fn system_prompt_round_trip() {
    let (store, _tmp) = test_store();
    let id = create_test_session(&store, "/workspace", "prompt test");

    let loaded = store.load_session(id).unwrap().unwrap();
    assert_eq!(loaded.system_prompt, "");

    store
        .update_session(
            id,
            None,
            None,
            None,
            None,
            Some("You are a helpful AI."),
            None,
            None,
            None,
            None,
        )
        .unwrap();

    let loaded = store.load_session(id).unwrap().unwrap();
    assert_eq!(loaded.system_prompt, "You are a helpful AI.");
}

#[test]
fn message_append_and_load_round_trip() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "msg test");

    let msg = Message::new(MessageRole::User, "Hello, world!");
    store.append_message(sid, &msg).unwrap();

    let messages = store.load_messages(sid).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content, "Hello, world!");
}

#[test]
fn history_search_matches_projected_message_fields() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "search test");
    let mut message = Message::new(MessageRole::Assistant, "Content needle");
    message.reasoning = "Reasoning needle".into();
    message.tool_calls = vec![tidev_llm::message::ToolCall {
        id: "call-needle".into(),
        name: "shell".into(),
        arguments: r#"{"query":"needle"}"#.into(),
        thought_signature: None,
    }];
    message.metadata.filepath = Some("src/needle.rs".into());
    message.attachments = vec![tidev_llm::message::MessageAttachment::Image {
        filename: "needle.png".into(),
        mime: "image/png".into(),
        data: b"needle in binary data".to_vec(),
        file_size: 21,
    }];
    store.append_message(sid, &message).unwrap();

    let hits = store
        .search_history(&SessionSearchOptions {
            query: "NEEDLE".into(),
            fields: SessionSearchFields::message(),
            session_id: Some(sid),
            workspace_root: None,
            roles: vec!["assistant".into()],
            case_sensitive: false,
            context_chars: 8,
            limit: 50,
            offset: 0,
        })
        .unwrap();
    let fields: Vec<&str> = hits.iter().map(|hit| hit.field.as_str()).collect();
    assert!(fields.contains(&"message.content"));
    assert!(fields.contains(&"message.reasoning"));
    assert!(fields.contains(&"message.tool_calls"));
    assert!(fields.contains(&"message.metadata"));
    assert!(
        !fields
            .iter()
            .any(|field| field.starts_with("message.attachment"))
    );
    assert!(hits.iter().all(|hit| hit.message_id == Some(message.id)));

    let all_hits = store
        .search_history(&SessionSearchOptions {
            query: "needle".into(),
            fields: SessionSearchFields::all(),
            session_id: Some(sid),
            workspace_root: None,
            roles: Vec::new(),
            case_sensitive: false,
            context_chars: 8,
            limit: 50,
            offset: 0,
        })
        .unwrap();
    assert!(
        all_hits
            .iter()
            .any(|hit| hit.field == "message.attachment.filename")
    );
    assert!(
        !all_hits
            .iter()
            .any(|hit| hit.field == "message.attachment.data")
    );
}

#[test]
fn history_search_matches_session_and_retained_tool_output() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "session needle");
    let message = Message::new(MessageRole::Tool, "tool result");
    store.append_message(sid, &message).unwrap();
    store
        .save_tool_output(
            "out-needle",
            sid,
            message.id,
            "call-output",
            "shell",
            "retained needle output",
        )
        .unwrap();

    let hits = store
        .search_history(&SessionSearchOptions {
            query: "needle".into(),
            fields: SessionSearchFields::all(),
            session_id: None,
            workspace_root: Some("/workspace".into()),
            roles: Vec::new(),
            case_sensitive: false,
            context_chars: 20,
            limit: 50,
            offset: 0,
        })
        .unwrap();
    assert!(
        hits.iter()
            .any(|hit| hit.kind == SessionSearchHitKind::Session)
    );
    let tool_output_hit = hits
        .iter()
        .find(|hit| hit.field == "tool_output.content")
        .expect("retained tool output should be searchable");
    assert_eq!(
        tool_output_hit.tool_output_id.as_deref(),
        Some("out-needle")
    );
    assert_eq!(tool_output_hit.message_id, Some(message.id));
}

#[test]
fn history_search_applies_offset_limit_and_case_sensitivity() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "case test");
    for content in ["Needle one", "needle two", "NEEDLE three"] {
        store
            .append_message(sid, &Message::new(MessageRole::User, content))
            .unwrap();
    }

    let hits = store
        .search_history(&SessionSearchOptions {
            query: "needle".into(),
            fields: SessionSearchFields {
                content: true,
                ..SessionSearchFields::default()
            },
            session_id: Some(sid),
            workspace_root: None,
            roles: Vec::new(),
            case_sensitive: false,
            context_chars: 20,
            limit: 1,
            offset: 1,
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].snippet, "needle two");
}

#[test]
fn message_reload_preserves_insert_order_for_equal_timestamps() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "equal timestamp test");
    let timestamp = Utc::now();
    let mut messages = Vec::new();
    for content in ["first", "second", "third"] {
        let mut message = Message::new(MessageRole::User, content);
        message.created_at = timestamp;
        messages.push(message);
    }

    store.append_messages(sid, &messages).unwrap();
    let loaded = store.load_messages(sid).unwrap();

    let contents: Vec<&str> = loaded
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(contents, ["first", "second", "third"]);
}

#[test]
fn message_app_data_round_trip() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "app data test");
    let msg = Message::new(MessageRole::User, "hello");
    let mut app_data = HashMap::new();
    app_data.insert(
        msg.id,
        MessageAppData {
            snapshot_hash: Some("snap-1".into()),
            patch_files: Some(r#"[{"files":["src/main.rs"]}]"#.into()),
            file_diffs: Some("[]".into()),
            mode: Some("plan".into()),
            child_session_id: None,
            provider_error: Some(ProviderErrorData {
                message: "HTTP 503 overloaded".into(),
                retryable: true,
                request_id: 7,
                user_message_id: Some(msg.id),
            }),
            ..Default::default()
        },
    );
    store
        .append_messages_with_app_data(sid, std::slice::from_ref(&msg), &app_data)
        .unwrap();

    let loaded = store.load_message_app_data(sid).unwrap();
    assert_eq!(loaded.get(&msg.id), app_data.get(&msg.id));
    let child_id = Uuid::new_v4();
    store
        .update_message_child_session_id(sid, msg.id, child_id)
        .unwrap();
    assert_eq!(
        store.load_message_app_data(sid).unwrap()[&msg.id].child_session_id,
        Some(child_id)
    );
    let protocol = store.load_messages(sid).unwrap();
    assert_eq!(protocol[0].content, "hello");
    let serialized = serde_json::to_value(&protocol[0]).unwrap();
    assert!(serialized.get("snapshot_hash").is_none());
    assert!(serialized.get("patch_files").is_none());
    assert!(serialized.get("file_diffs").is_none());
    assert!(serialized.get("mode").is_none());
}

#[test]
fn interrupted_stream_recovery_is_durable_and_idempotent() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "interrupted stream");
    let mut draft = Message::streaming(MessageRole::Assistant, "partial response");
    draft.reasoning = "partial reasoning".into();
    store.append_message(sid, &draft).unwrap();

    assert_eq!(store.recover_interrupted_streams().unwrap(), 1);
    assert_eq!(store.recover_interrupted_streams().unwrap(), 0);

    let messages = store.load_messages(sid).unwrap();
    let recovered = messages
        .iter()
        .find(|message| message.id == draft.id)
        .unwrap();
    assert!(recovered.streaming);
    assert_eq!(recovered.content, "partial response");
    assert_eq!(recovered.reasoning, "partial reasoning");
    assert!(recovered.completed_at.is_some());

    let app_data = store.load_message_app_data(sid).unwrap();
    assert_eq!(
        app_data[&draft.id]
            .interruption
            .as_ref()
            .map(|data| data.reason),
        Some(InterruptionReason::RuntimeRestarted)
    );
    let notice = messages
        .iter()
        .find(|message| message.role == MessageRole::Error)
        .unwrap();
    assert_eq!(
        app_data[&notice.id]
            .interruption
            .as_ref()
            .map(|data| data.reason),
        Some(InterruptionReason::RuntimeRestarted)
    );
}

#[test]
fn session_inspection_includes_application_data_and_tool_output() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "inspection test");
    let message = Message::tool_result(
        "call-1",
        "shell",
        tidev_llm::message::ToolExecutionResult::new("stored preview"),
    );
    let child_id = Uuid::new_v4();
    let mut app_data = HashMap::new();
    app_data.insert(
        message.id,
        MessageAppData {
            mode: Some("plan".into()),
            child_session_id: Some(child_id),
            ..Default::default()
        },
    );
    store
        .append_messages_with_app_data(sid, std::slice::from_ref(&message), &app_data)
        .unwrap();
    store
        .save_tool_output(
            "out-test-1",
            sid,
            message.id,
            "call-1",
            "shell",
            "complete tool output",
        )
        .unwrap();

    let inspection = store
        .load_session_inspection(sid)
        .unwrap()
        .expect("session should exist");
    assert_eq!(inspection.session.session_id, sid);
    assert_eq!(inspection.messages.len(), 1);
    assert_eq!(inspection.messages[0].sequence, 0);
    assert_eq!(inspection.messages[0].app_data, app_data[&message.id]);
    let tool_out = inspection.messages[0].tool_output.as_ref().unwrap();
    assert_eq!(tool_out.id, "out-test-1");
    assert_eq!(tool_out.tool_name, "shell");
    assert_eq!(tool_out.byte_size, "complete tool output".len());
}

#[test]
fn jsonl_export_contains_session_id_and_message_sequence() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "jsonl test");
    let messages = vec![
        Message::new(MessageRole::User, "first"),
        Message::new(MessageRole::Assistant, "second"),
    ];
    store.append_messages(sid, &messages).unwrap();

    let export_tmp = TempDir::new().unwrap();
    let export_path = export_tmp.path().join("messages.jsonl");
    let count = store.export_to_jsonl(&[sid], &export_path).unwrap();
    assert_eq!(count, 2);

    let lines = std::fs::read_to_string(export_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["session_id"], sid.to_string());
    assert_eq!(lines[0]["sequence"], 0);
    assert_eq!(lines[0]["message"]["content"], "first");
    assert_eq!(lines[1]["sequence"], 1);
    assert_eq!(lines[1]["message"]["content"], "second");
}

#[test]
fn sqlite_export_import_preserves_child_session_id() {
    let (source, _source_tmp) = test_store();
    let sid = create_test_session(&source, "/workspace", "export app data test");
    let msg = Message::new(MessageRole::Tool, "subagent finished");
    let child_id = Uuid::new_v4();
    let mut app_data = HashMap::new();
    app_data.insert(
        msg.id,
        MessageAppData {
            child_session_id: Some(child_id),
            ..Default::default()
        },
    );
    source
        .append_messages_with_app_data(sid, std::slice::from_ref(&msg), &app_data)
        .unwrap();

    let export_tmp = TempDir::new().unwrap();
    let export_path = export_tmp.path().join("session-export.db");
    source.export_to_sqlite(&[sid], &export_path).unwrap();

    let (target, _target_tmp) = test_store();
    assert_eq!(
        target
            .import_from_sqlite(&export_path, None, false)
            .unwrap(),
        vec![sid]
    );
    let loaded = target.load_message_app_data(sid).unwrap();
    assert_eq!(loaded[&msg.id].child_session_id, Some(child_id));
}

#[test]
fn message_streaming_update_content() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "stream test");
    let msg = Message::new(MessageRole::Assistant, "");
    store.append_message(sid, &msg).unwrap();

    store
        .update_message_content(sid, msg.id, "streamed content")
        .unwrap();

    let messages = store.load_messages(sid).unwrap();
    assert_eq!(messages[0].content, "streamed content");
}

#[test]
fn message_streaming_update_tool_calls() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "toolcall test");
    let msg = Message::new(MessageRole::Assistant, "");
    store.append_message(sid, &msg).unwrap();

    let calls = vec![tidev_llm::message::ToolCall {
        id: "tc-1".into(),
        name: "shell".into(),
        arguments: r#"{"command":"ls"}"#.into(),
        thought_signature: None,
    }];
    store
        .update_message_tool_calls(sid, msg.id, &calls)
        .unwrap();

    let messages = store.load_messages(sid).unwrap();
    assert_eq!(messages[0].tool_calls.len(), 1);
    assert_eq!(messages[0].tool_calls[0].name, "shell");
}

#[test]
fn message_update_metadata() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "meta test");
    let msg = Message::new(MessageRole::User, "hello");
    store.append_message(sid, &msg).unwrap();

    let meta = tidev_llm::message::ToolMetadata {
        prior_summary: Some("old summary".into()),
        prior_retained_from: Some(42),
        ..Default::default()
    };
    store.update_message_metadata(sid, msg.id, &meta).unwrap();

    let messages = store.load_messages(sid).unwrap();
    assert_eq!(
        messages[0].metadata.prior_summary.as_deref(),
        Some("old summary")
    );
    assert_eq!(messages[0].metadata.prior_retained_from, Some(42));
}

#[test]
fn tool_output_save_and_load() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "toolout test");
    let msg_id = Uuid::new_v4();
    store
        .save_tool_output(
            "out-abc12345",
            sid,
            msg_id,
            "call-xyz",
            "read",
            "file content here",
        )
        .unwrap();

    // Lookup by ID
    let loaded = store.load_tool_output("out-abc12345").unwrap().unwrap();
    match loaded {
        ToolOutputContent::Available { record, output } => {
            assert_eq!(record.id, "out-abc12345");
            assert_eq!(record.session_id, sid);
            assert_eq!(record.message_id, msg_id);
            assert_eq!(record.tool_name, "read");
            assert_eq!(record.byte_size, 17);
            assert_eq!(record.line_count, 1);
            assert_eq!(output, "file content here");
        }
        ToolOutputContent::Expired { .. } => panic!("should be available"),
    }

    // Lookup by message_id
    assert!(
        store
            .load_tool_output(&msg_id.to_string())
            .unwrap()
            .is_some()
    );
    // Lookup by tool_call_id
    assert!(store.load_tool_output("call-xyz").unwrap().is_some());
}

#[test]
fn tool_output_tombstone_expiration() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "toolout expire test");
    let msg_id = Uuid::new_v4();
    store
        .save_tool_output(
            "out-expire-1",
            sid,
            msg_id,
            "call-exp",
            "bash",
            "big output",
        )
        .unwrap();

    // Clear output older than -1 days (clears everything up to tomorrow)
    let cleared = store.clear_expired_tool_outputs(-1).unwrap();
    assert_eq!(cleared, 1);

    // Record still exists in Expired state
    let loaded = store.load_tool_output("out-expire-1").unwrap().unwrap();
    match loaded {
        ToolOutputContent::Available { .. } => panic!("should be expired"),
        ToolOutputContent::Expired { record } => {
            assert_eq!(record.id, "out-expire-1");
            assert_eq!(record.tool_name, "bash");
            assert_eq!(record.byte_size, 10);
        }
    }

    // Delete tombstones
    let deleted = store.delete_tombstones_older_than(-1).unwrap();
    assert_eq!(deleted, 1);
    assert!(store.load_tool_output("out-expire-1").unwrap().is_none());
}

#[test]
fn todo_save_and_load() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "todo test");
    let todos = vec![
        tidev_tools::types::TodoItem {
            content: "Fix bug".into(),
            status: "pending".into(),
        },
        tidev_tools::types::TodoItem {
            content: "Write tests".into(),
            status: "completed".into(),
        },
    ];
    store.save_todos(sid, &todos).unwrap();

    let loaded = store.load_todos(sid).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].content, "Fix bug");
    assert_eq!(loaded[1].status, "completed");
}

#[test]
fn session_delete_removes_session() {
    let (store, _tmp) = test_store();
    let id = create_test_session(&store, "/workspace", "delete test");
    assert!(store.load_session(id).unwrap().is_some());

    store.delete_session(id).unwrap();
    assert!(store.load_session(id).unwrap().is_none());
}

#[test]
fn session_update_title_and_status() {
    let (store, _tmp) = test_store();
    let id = create_test_session(&store, "/workspace", "original");
    store
        .update_session(
            id,
            Some("updated"),
            Some("ended"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

    let loaded = store.load_session(id).unwrap().unwrap();
    assert_eq!(loaded.title, "updated");
    assert_eq!(loaded.status, "ended");
}

#[test]
fn session_token_stats() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "token test");

    let mut msg = Message::new(MessageRole::User, "hello");
    msg.input_tokens = Some(10);
    msg.output_tokens = Some(20);
    store.append_message(sid, &msg).unwrap();

    let stats = store.get_session_token_stats(sid).unwrap();
    assert_eq!(stats.input_tokens, 10);
    assert_eq!(stats.output_tokens, 20);
}

#[test]
fn usage_activity_days_are_grouped_in_sql_and_scoped_to_the_range() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "activity test");
    let day_one = DateTime::parse_from_rfc3339("2026-01-02T10:00:00+00:00")
        .unwrap()
        .with_timezone(&Utc);
    let day_two = DateTime::parse_from_rfc3339("2026-01-03T12:00:00+00:00")
        .unwrap()
        .with_timezone(&Utc);
    let day_three = DateTime::parse_from_rfc3339("2026-01-04T08:00:00+00:00")
        .unwrap()
        .with_timezone(&Utc);

    for (created_at, total_tokens) in [(day_one, 10), (day_two, 20), (day_two, 30)] {
        let mut message = Message::new(MessageRole::Assistant, "response");
        message.created_at = created_at;
        message.total_tokens = Some(total_tokens);
        store.append_message(sid, &message).unwrap();
    }

    let mut out_of_range = Message::new(MessageRole::Assistant, "later response");
    out_of_range.created_at = day_three;
    out_of_range.total_tokens = Some(99);
    store.append_message(sid, &out_of_range).unwrap();

    let days = store.load_usage_activity_days(day_one, day_three).unwrap();

    assert_eq!(days.len(), 2);
    assert_eq!(days[0].date, "2026-01-02");
    assert_eq!(days[0].request_count, 1);
    assert_eq!(days[0].total_tokens, 10);
    assert_eq!(days[1].date, "2026-01-03");
    assert_eq!(days[1].request_count, 2);
    assert_eq!(days[1].total_tokens, 50);
}

#[test]
fn usage_insight_aggregates_are_scoped_and_bounded() {
    let (store, _tmp) = test_store();
    let primary = create_test_session(&store, "/workspace", "primary model");
    let secondary = Uuid::new_v4();
    store
        .create_session(
            secondary,
            "/workspace",
            "openai",
            "OpenAI",
            "gpt-5",
            "GPT-5",
            "secondary model",
            None,
            None,
        )
        .unwrap();
    let first = DateTime::parse_from_rfc3339("2026-01-05T10:00:00+00:00")
        .unwrap()
        .with_timezone(&Utc);
    let second = DateTime::parse_from_rfc3339("2026-01-06T12:00:00+00:00")
        .unwrap()
        .with_timezone(&Utc);
    let end = DateTime::parse_from_rfc3339("2026-01-07T00:00:00+00:00")
        .unwrap()
        .with_timezone(&Utc);

    for total_tokens in [500, 3_000] {
        let mut message = Message::new(MessageRole::Assistant, "response");
        message.created_at = first;
        message.total_tokens = Some(total_tokens);
        store.append_message(primary, &message).unwrap();
    }
    let mut secondary_message = Message::new(MessageRole::Assistant, "response");
    secondary_message.created_at = second;
    secondary_message.total_tokens = Some(20_000);
    store.append_message(secondary, &secondary_message).unwrap();

    let mut out_of_range = Message::new(MessageRole::Assistant, "later response");
    out_of_range.created_at = end;
    out_of_range.total_tokens = Some(200_000);
    store.append_message(secondary, &out_of_range).unwrap();

    let active_sessions = store
        .load_usage_active_sessions(Some(first), Some(end), "day")
        .unwrap();
    assert_eq!(active_sessions.len(), 2);
    assert_eq!(active_sessions[0].time_bucket, "2026-01-05T00:00:00Z");
    assert_eq!(active_sessions[0].active_sessions, 1);
    assert_eq!(active_sessions[1].active_sessions, 1);

    let rhythm = store.load_usage_rhythm(Some(first), Some(end)).unwrap();
    assert_eq!(rhythm.len(), 2);
    assert_eq!(rhythm[0].weekday, 0);
    assert_eq!(rhythm[0].hour, 10);
    assert_eq!(rhythm[0].request_count, 2);
    assert_eq!(rhythm[1].weekday, 1);
    assert_eq!(rhythm[1].hour, 12);
    assert_eq!(rhythm[1].total_tokens, 20_000);

    let model_mix = store
        .load_usage_model_mix(Some(first), Some(end), "day", 1)
        .unwrap();
    assert_eq!(model_mix.len(), 2);
    assert!(model_mix.iter().any(|bucket| {
        bucket.model_id == "gpt-5" && !bucket.is_other && bucket.total_tokens == 20_000
    }));
    assert!(model_mix.iter().any(|bucket| {
        bucket.model_id == "other" && bucket.is_other && bucket.total_tokens == 3_500
    }));

    let distribution = store
        .load_usage_request_size_distribution(Some(first), Some(end))
        .unwrap();
    assert_eq!(distribution.len(), 5);
    assert_eq!(distribution[0].request_count, 1);
    assert_eq!(distribution[1].request_count, 1);
    assert_eq!(distribution[2].request_count, 1);
    assert_eq!(distribution[4].request_count, 0);
}

#[test]
fn session_search_by_title() {
    let (store, _tmp) = test_store();
    create_test_session(&store, "/ws", "Refactor database layer");
    create_test_session(&store, "/ws", "Add unit tests");

    let results = store.search_sessions("Refactor", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Refactor database layer");
}

#[test]
fn instruction_sources_save_and_load() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "instr test");
    store
        .save_instruction_sources(
            sid,
            &["/path/to/AGENTS.md".into(), "/path/to/PROJECT.md".into()],
        )
        .unwrap();

    let loaded = store.load_instruction_sources(sid).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0], "/path/to/AGENTS.md");
    assert_eq!(loaded[1], "/path/to/PROJECT.md");

    store
        .append_instruction_sources(
            sid,
            &["/path/to/AGENTS.md".into(), "/path/to/EXTRA.md".into()],
        )
        .unwrap();
    let loaded2 = store.load_instruction_sources(sid).unwrap();
    assert_eq!(loaded2.len(), 3);
    assert_eq!(loaded2[0], "/path/to/AGENTS.md");
    assert_eq!(loaded2[1], "/path/to/EXTRA.md");
    assert_eq!(loaded2[2], "/path/to/PROJECT.md");
}

#[test]
fn sqlite_export_import_roundtrip() {
    let (store, tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "export import test");
    let todos = vec![tidev_tools::types::TodoItem {
        content: "Task 1".into(),
        status: "pending".into(),
    }];
    store.save_todos(sid, &todos).unwrap();
    store
        .save_instruction_sources(sid, &["/workspace/AGENTS.md".into()])
        .unwrap();

    let revert_target_msg = Uuid::new_v4();
    store
        .save_revert_state(sid, revert_target_msg, Some(b"snap-hash-123"))
        .unwrap();

    let msg = Message::new(MessageRole::Tool, "truncated snippet");
    store.append_message(sid, &msg).unwrap();
    store
        .save_tool_output(
            "out-export-1",
            sid,
            msg.id,
            "call-exp-1",
            "shell",
            "full shell output text",
        )
        .unwrap();

    let export_path = tmp.path().join("export.db");
    store.export_to_sqlite(&[sid], &export_path).unwrap();

    let (store2, _tmp2) = test_store();
    let imported = store2.import_from_sqlite(&export_path, None, true).unwrap();
    assert_eq!(imported, vec![sid]);

    let loaded_session = store2.load_session(sid).unwrap().unwrap();
    assert_eq!(loaded_session.workspace_root, "/workspace");
    assert_eq!(loaded_session.title, "export import test");

    let loaded_todos = store2.load_todos(sid).unwrap();
    assert_eq!(loaded_todos.len(), 1);
    assert_eq!(loaded_todos[0].content, "Task 1");

    let loaded_instr = store2.load_instruction_sources(sid).unwrap();
    assert_eq!(loaded_instr, vec!["/workspace/AGENTS.md"]);

    let loaded_revert = store2.load_revert_state(sid).unwrap().unwrap();
    assert_eq!(loaded_revert.0, revert_target_msg);
    assert_eq!(
        loaded_revert.1.as_deref(),
        Some(b"snap-hash-123".as_slice())
    );

    let loaded_tool_out = store2.load_tool_output("out-export-1").unwrap().unwrap();
    match loaded_tool_out {
        ToolOutputContent::Available { record, output } => {
            assert_eq!(record.id, "out-export-1");
            assert_eq!(record.tool_name, "shell");
            assert_eq!(output, "full shell output text");
        }
        ToolOutputContent::Expired { .. } => panic!("should be available"),
    }
}

#[test]
fn revert_state_crud() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "revert test");

    // Initially None
    assert!(store.load_revert_state(sid).unwrap().is_none());

    // Set revert state
    let target_msg = Uuid::new_v4();
    store
        .save_revert_state(sid, target_msg, Some(b"redo-hash-456"))
        .unwrap();
    let state = store.load_revert_state(sid).unwrap().unwrap();
    assert_eq!(state.0, target_msg);
    assert_eq!(state.1.as_deref(), Some(b"redo-hash-456".as_slice()));

    // Clear revert state
    store.save_revert_state(sid, Uuid::nil(), None).unwrap();
    assert!(store.load_revert_state(sid).unwrap().is_none());
}

#[test]
fn empty_fields_stored_as_zero_length_blob_and_null() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "empty fields test");

    let msg = Message::new(MessageRole::User, "");
    store.append_message(sid, &msg).unwrap();

    // Check SQLite raw columns directly
    let (content_blob, reasoning_blob, tool_calls_blob, metadata_blob, app_data_blob):
            RawMessageFields = store
            .read(|conn| {
                Ok(conn.query_row(
                    "SELECT content, reasoning, tool_calls, metadata, app_data FROM messages WHERE id = ?1",
                    params![msg.id.to_string()],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )?)
            })
            .unwrap();

    assert!(
        content_blob.is_empty(),
        "empty content should be stored as 0-length blob"
    );
    assert!(
        reasoning_blob.is_none(),
        "empty reasoning should be stored as NULL"
    );
    assert!(
        tool_calls_blob.is_empty(),
        "empty tool_calls should be stored as 0-length blob"
    );
    assert!(
        metadata_blob.is_empty(),
        "default metadata should be stored as 0-length blob"
    );
    assert!(
        app_data_blob.is_empty(),
        "default app_data should be stored as 0-length blob"
    );

    // Verify roundtrip loading
    let loaded = store.load_messages(sid).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].content, "");
    assert_eq!(loaded[0].reasoning, "");
    assert_eq!(loaded[0].tool_calls, Vec::new());
    assert_eq!(
        loaded[0].metadata,
        tidev_llm::message::ToolMetadata::default()
    );
}

#[test]
fn legacy_empty_zstd_frames_backward_compatibility() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "legacy compat test");

    // Manually insert legacy 9/11-byte zstd frames for empty values
    let legacy_content = zstd::stream::encode_all(std::io::Cursor::new(""), 3).unwrap();
    let legacy_reasoning = zstd::stream::encode_all(std::io::Cursor::new(""), 3).unwrap();
    let legacy_tool_calls = zstd::stream::encode_all(std::io::Cursor::new("[]"), 3).unwrap();
    let legacy_metadata = zstd::stream::encode_all(std::io::Cursor::new("{}"), 3).unwrap();
    let legacy_app_data = zstd::stream::encode_all(std::io::Cursor::new("{}"), 3).unwrap();

    let msg_id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();

    store
        .write_execute(
            "INSERT INTO messages (id, session_id, role, content, attachments, reasoning, \
                 tool_calls, metadata, created_at, app_data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                msg_id.to_string(),
                sid.to_string(),
                "assistant",
                legacy_content,
                "[]",
                legacy_reasoning,
                legacy_tool_calls,
                legacy_metadata,
                now,
                legacy_app_data,
            ],
        )
        .unwrap();

    let loaded = store.load_messages(sid).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, msg_id);
    assert_eq!(loaded[0].content, "");
    assert_eq!(loaded[0].reasoning, "");
    assert_eq!(loaded[0].tool_calls, Vec::new());
    assert_eq!(
        loaded[0].metadata,
        tidev_llm::message::ToolMetadata::default()
    );
}

#[test]
fn empty_tool_output_stored_as_empty_blob() {
    let (store, _tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "empty tool output test");
    let msg_id = Uuid::new_v4();

    store
        .save_tool_output("out-empty-1", sid, msg_id, "call-1", "shell", "")
        .unwrap();

    let raw_blob: Vec<u8> = store
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT output FROM tool_outputs WHERE id = 'out-empty-1'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert!(
        raw_blob.is_empty(),
        "empty tool output should be stored as 0-length blob"
    );

    let loaded = store.load_tool_output("out-empty-1").unwrap().unwrap();
    match loaded {
        ToolOutputContent::Available { record, output } => {
            assert_eq!(record.id, "out-empty-1");
            assert_eq!(output, "");
            assert_eq!(record.byte_size, 0);
        }
        ToolOutputContent::Expired { .. } => panic!("should be available"),
    }
}

#[test]
fn session_compressed_fields_storage_test() {
    let (store, tmp) = test_store();
    let sid = create_test_session(&store, "/workspace", "session compress test");

    // 1. Initial state: both context_summary and system_prompt should be empty 0-length blobs in SQLite
    let (raw_summary, raw_prompt): (Vec<u8>, Vec<u8>) = store
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT context_summary, system_prompt FROM sessions WHERE id = ?1",
                params![sid.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .unwrap();
    assert!(
        raw_summary.is_empty(),
        "initial context_summary should be 0-length blob"
    );
    assert!(
        raw_prompt.is_empty(),
        "initial system_prompt should be 0-length blob"
    );

    let loaded = store.load_session(sid).unwrap().unwrap();
    assert_eq!(loaded.context_summary, None);
    assert_eq!(loaded.system_prompt, "");

    // 2. Update with large repetitive content
    let large_prompt = "You are an AI pair programmer following AGENTS.md rules. ".repeat(200);
    let summary_text = "Context summary: modified 10 files and ran cargo test. ".repeat(50);

    store
        .update_session(
            sid,
            None,
            None,
            Some(&summary_text),
            Some(15),
            Some(&large_prompt),
            None,
            None,
            None,
            None,
        )
        .unwrap();

    // 3. Verify SQLite raw BLOBs are zstd-compressed
    const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
    let (comp_summary, comp_prompt): (Vec<u8>, Vec<u8>) = store
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT context_summary, system_prompt FROM sessions WHERE id = ?1",
                params![sid.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .unwrap();

    assert_eq!(&comp_summary[..4], &ZSTD_MAGIC);
    assert!(comp_summary.len() < summary_text.len());

    assert_eq!(&comp_prompt[..4], &ZSTD_MAGIC);
    assert!(comp_prompt.len() < large_prompt.len());

    // 4. Verify load_session and list_sessions decompress correctly
    let loaded = store.load_session(sid).unwrap().unwrap();
    assert_eq!(
        loaded.context_summary.as_deref(),
        Some(summary_text.as_str())
    );
    assert_eq!(loaded.system_prompt, large_prompt);
    assert_eq!(loaded.context_retained_from, 15);

    let listed = store.list_sessions(10, 0).unwrap();
    let session_item = listed.iter().find(|s| s.session_id == sid).unwrap();
    assert_eq!(
        session_item.context_summary.as_deref(),
        Some(summary_text.as_str())
    );
    assert_eq!(session_item.system_prompt, large_prompt);

    // 5. Export and import roundtrip
    let export_path = tmp.path().join("session_export.db");
    store.export_to_sqlite(&[sid], &export_path).unwrap();

    let (store2, _tmp2) = test_store();
    let imported = store2.import_from_sqlite(&export_path, None, true).unwrap();
    assert_eq!(imported, vec![sid]);

    let imported_session = store2.load_session(sid).unwrap().unwrap();
    assert_eq!(
        imported_session.context_summary.as_deref(),
        Some(summary_text.as_str())
    );
    assert_eq!(imported_session.system_prompt, large_prompt);
}
