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
        assert_eq!(loaded.model_display_name, "DeepSeek-V4-Flash");
        assert_eq!(loaded.workspace_root, "/tmp/workspace");
        assert_eq!(loaded.context_summary, None);
        assert_eq!(loaded.context_retained_from, 0);

        let conversation = store
            .load_conversation(session_id)
            .expect("conversation should load")
            .expect("conversation should exist");

        assert_eq!(conversation.provider_display_name, "DeepSeek");
        assert_eq!(conversation.model_display_name, "DeepSeek-V4-Flash");
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

#[test]
fn session_system_prompt_round_trip() {
    let path = std::env::temp_dir().join(format!(
        "tidev-session-store-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));
    {
        let store = SessionStore::open(&path).expect("store should open");
        let session_id = uuid::Uuid::new_v4();

        let _record = store
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

        // New session has empty system prompt
        let loaded = store
            .load_session_system_prompt(session_id)
            .expect("should load system prompt");
        assert_eq!(loaded, "", "new session should have empty system prompt");

        // Update and verify round-trip
        let prompt = "You are a helpful AI.\n\n<env>\n  Working directory: /tmp\n</env>";
        store
            .update_session_system_prompt(session_id, prompt)
            .expect("should update system prompt");

        let loaded = store
            .load_session_system_prompt(session_id)
            .expect("should load system prompt");
        assert_eq!(loaded, prompt, "loaded prompt should match updated prompt");
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn forked_session_copies_parent_system_prompt() {
    let path = std::env::temp_dir().join(format!(
        "tidev-session-store-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));
    {
        let store = SessionStore::open(&path).expect("store should open");
        let parent_id = uuid::Uuid::new_v4();

        store
            .create_session(
                parent_id,
                Path::new("/tmp/workspace"),
                "deepseek",
                "DeepSeek",
                "deepseek-v4-flash",
                "DeepSeek-V4-Flash",
                "Parent session",
            )
            .expect("parent session should be created");

        let prompt = "Static system prompt for parent";
        store
            .update_session_system_prompt(parent_id, prompt)
            .expect("should set parent system prompt");

        // Create child session (simulating fork)
        let child_id = uuid::Uuid::new_v4();
        store
            .create_session_with_parent(
                child_id,
                parent_id,
                Path::new("/tmp/workspace"),
                "deepseek",
                "DeepSeek",
                "deepseek-v4-flash",
                "DeepSeek-V4-Flash",
                "Child session",
            )
            .expect("child session should be created");

        // Child does NOT inherit automatically
        let child_prompt = store
            .load_session_system_prompt(child_id)
            .expect("should load child system prompt");
        assert_eq!(
            child_prompt, "",
            "child should not inherit system prompt automatically"
        );

        // Simulate fork copy: parent's prompt → child
        store
            .update_session_system_prompt(child_id, prompt)
            .expect("should copy system prompt to child");

        let child_prompt = store
            .load_session_system_prompt(child_id)
            .expect("should load child system prompt");
        assert_eq!(
            child_prompt, prompt,
            "child should have same prompt after copy"
        );
    }
    let _ = std::fs::remove_file(path);
}

// ── Goal tests ─────────────────────────────────────────────────────────────

#[test]
fn goal_set_and_get_round_trip() {
    let path = std::env::temp_dir().join(format!(
        "tidev-goal-set-get-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let session_id = uuid::Uuid::new_v4();

        // Create a session first (foreign key constraint)
        store
            .create_session(
                session_id,
                Path::new("/tmp/workspace"),
                "test",
                "Test",
                "test-model",
                "Test Model",
                "Goal test",
            )
            .expect("create_session should succeed");

        // No goal yet
        assert!(store.get_goal(session_id).unwrap().is_none());

        // Set a goal
        let goal = store
            .set_goal(session_id, "Implement the /goal command")
            .expect("set_goal should succeed");
        assert_eq!(goal.objective, "Implement the /goal command");
        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.tokens_used, 0);
        assert_eq!(goal.time_used_seconds, 0);

        // Retrieve
        let loaded = store
            .get_goal(session_id)
            .expect("get_goal should succeed")
            .expect("goal should exist");
        assert_eq!(loaded.objective, "Implement the /goal command");
        assert_eq!(loaded.status, GoalStatus::Active);
        assert_eq!(loaded.session_id, session_id);
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn goal_overwrite_resets_counters() {
    let path = std::env::temp_dir().join(format!(
        "tidev-goal-overwrite-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let session_id = uuid::Uuid::new_v4();

        store
            .create_session(
                session_id,
                Path::new("/tmp/workspace"),
                "test",
                "Test",
                "test-model",
                "Test Model",
                "Goal overwrite test",
            )
            .expect("create_session should succeed");

        store
            .set_goal(session_id, "First goal")
            .expect("set_goal should succeed");

        // Accumulate some usage
        store
            .account_goal_usage(session_id, 100, 30)
            .expect("account_goal_usage should succeed");

        let g = store.get_goal(session_id).unwrap().unwrap();
        assert_eq!(g.tokens_used, 100);
        assert_eq!(g.time_used_seconds, 30);

        // Overwrite with new goal — counters reset to 0
        store
            .set_goal(session_id, "Second goal")
            .expect("set_goal should succeed");

        let g = store.get_goal(session_id).unwrap().unwrap();
        assert_eq!(g.objective, "Second goal");
        assert_eq!(g.status, GoalStatus::Active);
        assert_eq!(g.tokens_used, 0);
        assert_eq!(g.time_used_seconds, 0);
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn goal_status_transitions() {
    let path = std::env::temp_dir().join(format!(
        "tidev-goal-status-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let session_id = uuid::Uuid::new_v4();

        store
            .create_session(
                session_id,
                Path::new("/tmp/workspace"),
                "test",
                "Test",
                "test-model",
                "Test Model",
                "Goal status test",
            )
            .expect("create_session should succeed");

        store
            .set_goal(session_id, "Test goal")
            .expect("set_goal should succeed");

        // Active → Paused
        store
            .update_goal_status(session_id, GoalStatus::Paused)
            .expect("update to paused should succeed");
        assert_eq!(
            store.get_goal(session_id).unwrap().unwrap().status,
            GoalStatus::Paused
        );

        // Paused → Active
        store
            .update_goal_status(session_id, GoalStatus::Active)
            .expect("update to active should succeed");
        assert_eq!(
            store.get_goal(session_id).unwrap().unwrap().status,
            GoalStatus::Active
        );

        // Active → Complete
        store
            .update_goal_status(session_id, GoalStatus::Complete)
            .expect("update to complete should succeed");
        assert_eq!(
            store.get_goal(session_id).unwrap().unwrap().status,
            GoalStatus::Complete
        );

        // Updating non-existent goal should error
        let missing = uuid::Uuid::new_v4();
        assert!(
            store
                .update_goal_status(missing, GoalStatus::Active)
                .is_err()
        );
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn goal_clear_removes_goal() {
    let path =
        std::env::temp_dir().join(format!("tidev-goal-clear-{}.sqlite3", uuid::Uuid::new_v4()));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let session_id = uuid::Uuid::new_v4();

        store
            .create_session(
                session_id,
                Path::new("/tmp/workspace"),
                "test",
                "Test",
                "test-model",
                "Test Model",
                "Goal clear test",
            )
            .expect("create_session should succeed");

        store
            .set_goal(session_id, "Goal to clear")
            .expect("set_goal should succeed");
        assert!(store.get_goal(session_id).unwrap().is_some());

        store
            .clear_goal(session_id)
            .expect("clear_goal should succeed");
        assert!(store.get_goal(session_id).unwrap().is_none());

        // Clearing again is idempotent
        store
            .clear_goal(session_id)
            .expect("clear_goal again should succeed");
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn goal_account_usage_accumulates() {
    let path =
        std::env::temp_dir().join(format!("tidev-goal-usage-{}.sqlite3", uuid::Uuid::new_v4()));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let session_id = uuid::Uuid::new_v4();

        store
            .create_session(
                session_id,
                Path::new("/tmp/workspace"),
                "test",
                "Test",
                "test-model",
                "Test Model",
                "Goal usage test",
            )
            .expect("create_session should succeed");

        store
            .set_goal(session_id, "Usage test")
            .expect("set_goal should succeed");

        store
            .account_goal_usage(session_id, 50, 10)
            .expect("account_goal_usage should succeed");
        store
            .account_goal_usage(session_id, 150, 20)
            .expect("account_goal_usage should succeed");

        let g = store.get_goal(session_id).unwrap().unwrap();
        assert_eq!(g.tokens_used, 200);
        assert_eq!(g.time_used_seconds, 30);
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn goal_account_usage_noops_when_no_goal() {
    let path = std::env::temp_dir().join(format!(
        "tidev-goal-usage-noop-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let session_id = uuid::Uuid::new_v4();

        // Create session but no goal
        store
            .create_session(
                session_id,
                Path::new("/tmp/workspace"),
                "test",
                "Test",
                "test-model",
                "Test Model",
                "Goal noop test",
            )
            .expect("create_session should succeed");

        // No goal set — should silently no-op
        store
            .account_goal_usage(session_id, 100, 30)
            .expect("account_goal_usage should no-op");
        assert!(store.get_goal(session_id).unwrap().is_none());
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn goal_account_usage_noops_when_paused_or_complete() {
    let path = std::env::temp_dir().join(format!(
        "tidev-goal-usage-paused-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let session_id = uuid::Uuid::new_v4();

        store
            .create_session(
                session_id,
                Path::new("/tmp/workspace"),
                "test",
                "Test",
                "test-model",
                "Test Model",
                "Goal paused test",
            )
            .expect("create_session should succeed");

        store
            .set_goal(session_id, "Paused test")
            .expect("set_goal should succeed");

        // Pause the goal — usage should no-op
        store
            .update_goal_status(session_id, GoalStatus::Paused)
            .expect("update_goal_status should succeed");
        store
            .account_goal_usage(session_id, 100, 30)
            .expect("account_goal_usage should no-op");

        let g = store.get_goal(session_id).unwrap().unwrap();
        assert_eq!(g.tokens_used, 0, "should not accumulate when paused");
        assert_eq!(g.time_used_seconds, 0, "should not accumulate when paused");

        // Mark complete — usage should no-op
        store
            .update_goal_status(session_id, GoalStatus::Complete)
            .expect("update_goal_status should succeed");
        store
            .account_goal_usage(session_id, 200, 60)
            .expect("account_goal_usage should no-op");

        let g = store.get_goal(session_id).unwrap().unwrap();
        assert_eq!(g.tokens_used, 0, "should not accumulate when complete");
        assert_eq!(
            g.time_used_seconds, 0,
            "should not accumulate when complete"
        );
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn goal_cascades_on_session_delete() {
    let path = std::env::temp_dir().join(format!(
        "tidev-goal-cascade-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));

    {
        let store = SessionStore::open(&path).expect("store should open");
        let session_id = uuid::Uuid::new_v4();

        store
            .create_session(
                session_id,
                Path::new("/tmp/workspace"),
                "test",
                "Test",
                "test-model",
                "Test Model",
                "Goal cascade test",
            )
            .expect("create_session should succeed");

        store
            .set_goal(session_id, "Will be deleted")
            .expect("set_goal should succeed");
        assert!(store.get_goal(session_id).unwrap().is_some());

        // Delete the session
        store
            .delete_session(session_id)
            .expect("delete_session should succeed");

        // Goal should be gone (ON DELETE CASCADE)
        assert!(store.get_goal(session_id).unwrap().is_none());
    }

    let _ = std::fs::remove_file(path);
}
