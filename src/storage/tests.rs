use super::*;
use std::path::Path;

#[test]
fn session_display_names_round_trip() {
    let path = std::env::temp_dir().join(format!(
        "tidev-session-store-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let session_id = uuid::Uuid::new_v4();

        let record = store
            .create_session(
                session_id,
                Path::new("/tmp/workspace"),
                "deepseek",
                "DeepSeek",
                "deepseek-v4-flash",
                "DeepSeek-V4-Flash",
                "Untitled session",
            )
            .expect("session should be created");

        assert_eq!(record.provider_id, "deepseek");
        assert_eq!(record.provider_display_name, "DeepSeek");
        assert_eq!(record.model_id, "deepseek-v4-flash");
        assert_eq!(record.model_display_name, "DeepSeek-V4-Flash");
        assert_eq!(record.workspace_root, "/tmp/workspace");
        assert_eq!(record.context_summary, None);
        assert_eq!(record.context_retained_from, 0);

        let loaded = store
            .load_session_record(session_id)
            .expect("session should load")
            .expect("session should exist");

        assert_eq!(loaded.provider_display_name, "DeepSeek");
        assert_eq!(loaded.model_display_name, "DeepSeek Chat");
        assert_eq!(loaded.workspace_root, "/tmp/workspace");
        assert_eq!(loaded.context_summary, None);
        assert_eq!(loaded.context_retained_from, 0);

        let conversation = store
            .load_conversation(session_id)
            .expect("conversation should load")
            .expect("conversation should exist");

        assert_eq!(conversation.provider_display_name, "DeepSeek");
        assert_eq!(conversation.model_display_name, "DeepSeek Chat");
        assert_eq!(conversation.workspace_root, "/tmp/workspace");
        assert_eq!(conversation.context_summary, None);
        assert_eq!(conversation.context_retained_from, 0);
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn context_state_round_trip() {
    let path = std::env::temp_dir().join(format!(
        "tidev-session-store-context-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let session_id = uuid::Uuid::new_v4();

        store
            .create_session(
                session_id,
                Path::new("/tmp/workspace"),
                "openai",
                "OpenAI",
                "gpt-4o",
                "GPT-4o",
                "Untitled session",
            )
            .expect("session should be created");

        store
            .update_session_context_state(
                session_id,
                Some("Context summary for continuation:\n- files: src/main.rs"),
                7,
            )
            .expect("context state should update");

        let record = store
            .load_session_record(session_id)
            .expect("session should load")
            .expect("session should exist");

        assert_eq!(
            record.context_summary.as_deref(),
            Some("Context summary for continuation:\n- files: src/main.rs")
        );
        assert_eq!(record.context_retained_from, 7);

        let conversation = store
            .load_conversation(session_id)
            .expect("conversation should load")
            .expect("conversation should exist");

        assert_eq!(
            conversation.context_summary.as_deref(),
            Some("Context summary for continuation:\n- files: src/main.rs")
        );
        assert_eq!(conversation.context_retained_from, 7);
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn child_session_round_trip_records_parent() {
    let path = std::env::temp_dir().join(format!(
        "tidev-session-store-child-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let parent_session_id = uuid::Uuid::new_v4();
        let child_session_id = uuid::Uuid::new_v4();

        store
            .create_session(
                parent_session_id,
                Path::new("/tmp/workspace"),
                "openai",
                "OpenAI",
                "gpt-4o",
                "GPT-4o",
                "Parent",
            )
            .expect("parent session should be created");

        let child_record = store
            .create_session_with_parent(
                child_session_id,
                parent_session_id,
                Path::new("/tmp/workspace"),
                "openai",
                "OpenAI",
                "gpt-4o",
                "GPT-4o",
                "Task: Child",
            )
            .expect("child session should be created");

        assert_eq!(child_record.parent_session_id, Some(parent_session_id));

        let loaded = store
            .load_session_record(child_session_id)
            .expect("child session should load")
            .expect("child session should exist");
        assert_eq!(loaded.parent_session_id, Some(parent_session_id));

        let children = store
            .load_child_sessions(parent_session_id)
            .expect("child sessions should load");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].session_id, child_session_id);
        assert_eq!(children[0].parent_session_id, Some(parent_session_id));
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn tool_event_output_round_trip() {
    let path = std::env::temp_dir().join(format!(
        "tidev-session-store-tool-output-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let session_id = uuid::Uuid::new_v4();
        let message_id = uuid::Uuid::new_v4();

        store
            .create_session(
                session_id,
                Path::new("/tmp/workspace"),
                "openai",
                "OpenAI",
                "gpt-4o",
                "GPT-4o",
                "Untitled session",
            )
            .expect("session should be created");

        store
            .append_tool_event(
                session_id,
                message_id,
                "grep",
                r#"{"pattern":"foo"}"#,
                "match-one\nmatch-two",
            )
            .expect("tool event should append");

        let output = store
            .load_tool_event_output(session_id, message_id)
            .expect("tool output should load")
            .expect("tool output should exist");

        assert_eq!(output, "match-one\nmatch-two");
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn copy_tool_permissions_inherits_parent_permissions() {
    let path = std::env::temp_dir().join(format!(
        "tidev-session-store-permissions-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let parent_session_id = uuid::Uuid::new_v4();
        let child_session_id = uuid::Uuid::new_v4();

        store
            .create_session(
                parent_session_id,
                Path::new("/tmp/workspace"),
                "openai",
                "OpenAI",
                "gpt-4o",
                "GPT-4o",
                "Parent",
            )
            .expect("parent session should be created");

        store
            .create_session_with_parent(
                child_session_id,
                parent_session_id,
                Path::new("/tmp/workspace"),
                "openai",
                "OpenAI",
                "gpt-4o",
                "GPT-4o",
                "Task: Child",
            )
            .expect("child session should be created");

        store
            .remember_tool_permission(parent_session_id, "bash", true)
            .expect("permission should be recorded");
        store
            .remember_tool_permission(parent_session_id, "read", false)
            .expect("permission should be recorded");

        store
            .copy_tool_permissions(parent_session_id, child_session_id)
            .expect("permissions should be copied");

        assert_eq!(
            store
                .load_tool_permission(child_session_id, "bash")
                .expect("child permission should load"),
            Some(true)
        );
        assert_eq!(
            store
                .load_tool_permission(child_session_id, "read")
                .expect("child permission should load"),
            Some(false)
        );
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn child_sessions_are_hidden_from_session_lists() {
    let path = std::env::temp_dir().join(format!(
        "tidev-session-store-visibility-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let workspace_root = Path::new("/tmp/workspace");
        let parent_session_id = uuid::Uuid::new_v4();
        let child_session_id = uuid::Uuid::new_v4();

        store
            .create_session(
                parent_session_id,
                workspace_root,
                "openai",
                "OpenAI",
                "gpt-4o",
                "GPT-4o",
                "Parent",
            )
            .expect("parent session should be created");

        std::thread::sleep(std::time::Duration::from_millis(2));

        store
            .create_session_with_parent(
                child_session_id,
                parent_session_id,
                workspace_root,
                "openai",
                "OpenAI",
                "gpt-4o",
                "GPT-4o",
                "Task: Child",
            )
            .expect("child session should be created");

        let workspace_sessions = store
            .load_sessions_for_workspace(workspace_root)
            .expect("workspace sessions should load");
        assert_eq!(workspace_sessions.len(), 1);
        assert_eq!(workspace_sessions[0].session_id, parent_session_id);

        let all_sessions = store.load_all_sessions().expect("all sessions should load");
        assert_eq!(all_sessions.len(), 1);
        assert_eq!(all_sessions[0].session_id, parent_session_id);

        let latest_session = store
            .load_latest_session()
            .expect("latest session should load")
            .expect("latest session should exist");
        assert_eq!(latest_session.session_id, parent_session_id);
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn workspace_session_listing_is_scoped_and_sorted() {
    let path = std::env::temp_dir().join(format!(
        "tidev-session-store-list-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let shared_root = Path::new("/tmp/tidev-workspace-a");
        let other_root = Path::new("/tmp/tidev-workspace-b");

        let first = store
            .create_session(
                uuid::Uuid::new_v4(),
                shared_root,
                "openai",
                "OpenAI",
                "gpt-4o",
                "GPT-4o",
                "First",
            )
            .expect("first session should be created");

        std::thread::sleep(std::time::Duration::from_millis(2));

        let second = store
            .create_session(
                uuid::Uuid::new_v4(),
                shared_root,
                "openai",
                "OpenAI",
                "gpt-4o-mini",
                "GPT-4o mini",
                "Second",
            )
            .expect("second session should be created");

        store
            .create_session(
                uuid::Uuid::new_v4(),
                other_root,
                "anthropic",
                "Anthropic",
                "claude",
                "Claude",
                "Other",
            )
            .expect("other session should be created");

        let sessions = store
            .load_sessions_for_workspace(shared_root)
            .expect("sessions should load");

        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, second.session_id);
        assert_eq!(sessions[1].session_id, first.session_id);
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn revert_marker_round_trips() {
    let path = std::env::temp_dir().join(format!(
        "tidev-session-store-revert-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let session_id = uuid::Uuid::new_v4();
        let message_id = uuid::Uuid::new_v4();

        store
            .create_session(
                session_id,
                Path::new("/workspace"),
                "deepseek",
                "DeepSeek",
                "deepseek-v4-flash",
                "DeepSeek-V4-Flash",
                "Untitled session",
            )
            .expect("session should be created");

        assert_eq!(
            store
                .load_revert_message_id(session_id)
                .expect("revert should load"),
            None
        );

        store
            .set_revert_message_id(session_id, Some(message_id), None)
            .expect("revert should save");

        assert_eq!(
            store
                .load_revert_message_id(session_id)
                .expect("revert should load"),
            Some(message_id)
        );

        let conversation = store
            .load_conversation(session_id)
            .expect("conversation should load")
            .expect("conversation should exist");

        assert_eq!(conversation.revert_message_id, Some(message_id));

        store
            .clear_revert_message_id(session_id)
            .expect("revert should clear");

        assert_eq!(
            store
                .load_revert_message_id(session_id)
                .expect("revert should load"),
            None
        );
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn gateway_chat_session_mapping_round_trip() {
    let path = std::env::temp_dir().join(format!(
        "tidev-session-store-gateway-map-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let first_session = uuid::Uuid::new_v4();
        let second_session = uuid::Uuid::new_v4();
        let chat_key = "-100123456:42";

        store
            .create_session(
                first_session,
                Path::new("/workspace"),
                "openai",
                "OpenAI",
                "gpt-4o-mini",
                "GPT-4o mini",
                "First",
            )
            .expect("first session should be created");

        store
            .set_gateway_chat_session("telegram", chat_key, first_session)
            .expect("mapping should save");

        assert_eq!(
            store
                .load_gateway_chat_session("telegram", chat_key)
                .expect("mapping should load"),
            Some(first_session)
        );

        store
            .create_session(
                second_session,
                Path::new("/workspace"),
                "openai",
                "OpenAI",
                "gpt-4o-mini",
                "GPT-4o mini",
                "Second",
            )
            .expect("second session should be created");

        store
            .set_gateway_chat_session("telegram", chat_key, second_session)
            .expect("mapping should update");

        assert_eq!(
            store
                .load_gateway_chat_session("telegram", chat_key)
                .expect("updated mapping should load"),
            Some(second_session)
        );

        store
            .clear_gateway_chat_session("telegram", chat_key)
            .expect("mapping should clear");

        assert_eq!(
            store
                .load_gateway_chat_session("telegram", chat_key)
                .expect("cleared mapping should load"),
            None
        );
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn gateway_chat_model_mapping_round_trip() {
    let path = std::env::temp_dir().join(format!(
        "tidev-session-store-gateway-model-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let chat_key = "12345";

        store
            .set_gateway_chat_model("telegram", chat_key, "openai", "gpt-4o-mini")
            .expect("model mapping should save");

        assert_eq!(
            store
                .load_gateway_chat_model("telegram", chat_key)
                .expect("model mapping should load"),
            Some(("openai".to_string(), "gpt-4o-mini".to_string()))
        );

        store
            .set_gateway_chat_model("telegram", chat_key, "deepseek", "deepseek-v4-flash")
            .expect("model mapping should update");

        assert_eq!(
            store
                .load_gateway_chat_model("telegram", chat_key)
                .expect("updated model mapping should load"),
            Some(("deepseek".to_string(), "deepseek-v4-flash".to_string()))
        );

        store
            .clear_gateway_chat_model("telegram", chat_key)
            .expect("model mapping should clear");

        assert_eq!(
            store
                .load_gateway_chat_model("telegram", chat_key)
                .expect("cleared model mapping should load"),
            None
        );
    }

    let _ = std::fs::remove_file(path);
}
