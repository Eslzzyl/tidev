use std::sync::Arc;
use tokio::sync::Mutex;

use uuid::Uuid;

use tidev_types::prompts::SessionMode;

use tidev_llm::LlmClient;
use tidev_storage::SessionStore;

use crate::{config::ConfigPaths, context::ContextManager};
use tidev_session::session::{
    BackendEvent, Conversation, Message, MessageRole, ToolCall, ToolExecutionResult,
};

use super::AgentRuntime;

/// Create a minimal ActiveModel for passing to execute_tool_calls.
#[allow(dead_code)]
fn test_active_model() -> crate::config::ActiveModel {
    crate::config::ActiveModel {
        provider_id: "test".into(),
        provider_display_name: "Test".into(),
        base_url: "http://localhost".into(),
        api_type: crate::config::ApiType::OpenAiChatCompletions,
        model_id: "test-model".into(),
        request_model_id: "test-model".into(),
        display_name: "Test Model".into(),
        context_window: 4096,
        max_output_tokens: 1024,
        temperature: Some(0.0),
        supports_images: false,
        system_prompt: String::new(),
        api_key: None,
        extra_body: None,
        thinking_level: crate::config::reasoning::ThinkingLevelType::default(),
    }
}

/// Create a minimal AgentRuntime backed by a tempfile database.
fn agent_runtime() -> (AgentRuntime, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let store = SessionStore::open(&db_path).unwrap();
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(&ws).unwrap();
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();

    let agent = AgentRuntime {
        workspace_root: ws,
        config_dir,
        config_paths: ConfigPaths {
            config_dir: tmp.path().join("config"),
            data_dir: tmp.path().join("data"),
            config_file: tmp.path().join("config").join("config.toml"),
            database_file: db_path.clone(),
            auth_file: tmp.path().join("auth.json"),
        },
        config: crate::config::AppConfig::default(),
        auth: crate::config::AuthStore::default(),
        store: Arc::new(Mutex::new(store)),
        llm_client: LlmClient::new(false, 100, false, 100).unwrap(),
        tools: crate::tooling::ToolRegistry::new(
            tmp.path().join("workspace"),
            tmp.path().join("config"),
            vec![],
            crate::mcp::McpManager::new(tmp.path().join("workspace"), Default::default()),
            tidev_types::types::PermissionConfig::default(),
            std::sync::Arc::new(crate::tooling::FileReadTracker::new()),
            std::sync::Arc::new(crate::memory::MemoryStore::open(&db_path).unwrap()),
            false,
            None,
            crate::config::WebSearchConfig::default(),
            std::sync::Arc::new(crate::config::AuthStore::default()),
        ),
        instructions: vec![],
        instruction_content_cache: Default::default(),
        queued_messages: std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::new(),
        )),
        auto_approve_permissions: false,
        hooks: crate::hooks::HookEngine::new(Default::default(), tmp.path().join("workspace")),
    };
    (agent, tmp)
}

#[test]
fn build_request_messages_basic_filtering() {
    let msgs = vec![
        Message::new(MessageRole::User, "Hello"),
        Message::new(MessageRole::Assistant, "Hi there!"),
        Message::new(MessageRole::User, "What is the weather?"),
        Message::new(MessageRole::Assistant, "Let me check."),
    ];
    let cm = ContextManager {
        retained_from: 2,
        ..ContextManager::new()
    };
    let mut conv = Conversation::new(Uuid::nil(), "", "", "", "", "", "");
    conv.messages = msgs;
    let result = cm.build_request_messages(&conv, SessionMode::Build);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].content, "What is the weather?");
    assert_eq!(result[1].content, "Let me check.");
}

#[test]
fn build_request_messages_empty_after_retained() {
    let msgs = vec![
        Message::new(MessageRole::User, "Hello"),
        Message::new(MessageRole::Assistant, "Hi"),
    ];
    let cm = ContextManager {
        retained_from: 2,
        ..ContextManager::new()
    };
    let mut conv = Conversation::new(Uuid::nil(), "", "", "", "", "", "");
    conv.messages = msgs;
    let result = cm.build_request_messages(&conv, SessionMode::Build);
    assert_eq!(result.len(), 0);
}

#[test]
fn build_request_messages_skips_streaming_messages() {
    let msgs = vec![
        Message::new(MessageRole::User, "hello"),
        Message::streaming(MessageRole::Assistant, "still typing..."),
    ];
    let mut conv = Conversation::new(Uuid::nil(), "", "", "", "", "", "");
    conv.messages = msgs;
    let result = ContextManager::new().build_request_messages(&conv, SessionMode::Build);
    assert!(!result.iter().any(|m| m.content == "still typing..."));
}

#[test]
fn build_request_messages_keeps_valid_tool_results() {
    let mut assistant = Message::new(MessageRole::Assistant, "searching");
    assistant.tool_calls = vec![ToolCall {
        id: "tc-1".to_string(),
        name: "grep".to_string(),
        arguments: "{}".to_string(),
        thought_signature: None,
    }];
    let msgs = vec![
        Message::new(MessageRole::User, "find it"),
        assistant.clone(),
        Message::tool_result("tc-1", "grep", ToolExecutionResult::new("found!")),
        Message::new(MessageRole::Assistant, "result"),
    ];
    let mut conv = Conversation::new(Uuid::nil(), "", "", "", "", "", "");
    conv.messages = msgs;
    let result = ContextManager::new().build_request_messages(&conv, SessionMode::Build);
    let roles: Vec<_> = result.iter().map(|m| m.role.clone()).collect();
    assert_eq!(
        roles,
        vec![
            MessageRole::User,
            MessageRole::Assistant,
            MessageRole::Tool,
            MessageRole::Assistant,
        ]
    );
}

#[test]
fn build_request_messages_injects_orphan_tool_failures() {
    let mut assistant = Message::new(MessageRole::Assistant, "");
    assistant.tool_calls = vec![ToolCall {
        id: "orphan".to_string(),
        name: "edit".to_string(),
        arguments: "{}".to_string(),
        thought_signature: None,
    }];
    let msgs = vec![assistant, Message::new(MessageRole::User, "what happened?")];
    let mut conv = Conversation::new(Uuid::nil(), "", "", "", "", "", "");
    conv.messages = msgs;
    let result = ContextManager::new().build_request_messages(&conv, SessionMode::Build);
    // The orphan tool call should be turned into a synthetic error result
    let orphan_tool = result.iter().find(|m| m.role == MessageRole::Tool);
    assert!(
        orphan_tool.is_some(),
        "orphan tool should become a Tool result"
    );
    if let Some(orphan) = orphan_tool {
        assert!(
            orphan.content.contains("orphan"),
            "tool result should mention orphan: got {}",
            orphan.content
        );
    }
}

#[test]
fn build_request_messages_removes_orphan_tool_calls_without_results() {
    let mut assistant = Message::new(MessageRole::Assistant, "");
    assistant.tool_calls = vec![ToolCall {
        id: "tc-missing".to_string(),
        name: "read".to_string(),
        arguments: "{}".to_string(),
        thought_signature: None,
    }];
    let msgs = vec![
        assistant,
        Message::new(MessageRole::User, "next message"),
        Message::new(MessageRole::Assistant, "final"),
    ];
    let mut conv = Conversation::new(Uuid::nil(), "", "", "", "", "", "");
    conv.messages = msgs;
    let result = ContextManager::new().build_request_messages(&conv, SessionMode::Build);
    let tool_results: Vec<_> = result
        .iter()
        .filter(|m| m.role == MessageRole::Tool)
        .collect();
    assert_eq!(tool_results.len(), 1);
    assert!(tool_results[0].content.contains("tc-missing"));
    assert!(tool_results[0].content.contains("orphaned"));
}

#[test]
fn build_request_messages_removes_orphan_tool_calls_across_messages() {
    let mut assistant = Message::new(MessageRole::Assistant, "");
    assistant.tool_calls = vec![ToolCall {
        id: "tc-orphan".to_string(),
        name: "bash".to_string(),
        arguments: "echo hi".to_string(),
        thought_signature: None,
    }];
    let msgs = vec![
        Message::new(MessageRole::User, "do it"),
        assistant,
        // No tool result follows — next message is a user message
        Message::new(MessageRole::User, "never mind"),
        Message::new(MessageRole::Assistant, "ok"),
    ];
    let mut conv = Conversation::new(Uuid::nil(), "", "", "", "", "", "");
    conv.messages = msgs;
    let result = ContextManager::new().build_request_messages(&conv, SessionMode::Build);
    // The orphan tool call should have a synthetic tool result inserted
    let tools: Vec<_> = result
        .iter()
        .filter(|m| m.role == MessageRole::Tool)
        .collect();
    assert_eq!(tools.len(), 1, "should have one synthetic tool result");
    assert!(tools[0].content.contains("orphaned"));
}

#[test]
fn build_request_messages_remove_stale_tool_results() {
    let msgs = vec![
        Message::tool_result("tc-old", "grep", ToolExecutionResult::new("old")),
        Message::new(MessageRole::Assistant, "some response"),
        Message::new(MessageRole::User, "new query"),
    ];
    let mut conv = Conversation::new(Uuid::nil(), "", "", "", "", "", "");
    conv.messages = msgs;
    let result = ContextManager::new().build_request_messages(&conv, SessionMode::Build);
    assert!(
        !result.iter().any(|m| m.role == MessageRole::Tool),
        "stale tool result should be removed"
    );
}

#[test]
fn build_request_messages_handles_empty_messages() {
    let mut conv = Conversation::new(Uuid::nil(), "", "", "", "", "", "");
    conv.messages = vec![];
    let result = ContextManager::new().build_request_messages(&conv, SessionMode::Build);
    assert!(result.is_empty());
}

#[test]
fn build_request_messages_handle_tool_result_matching() {
    let mut assistant = Message::new(MessageRole::Assistant, "searching");
    assistant.tool_calls = vec![ToolCall {
        id: "tc-valid".to_string(),
        name: "read".to_string(),
        arguments: "{}".to_string(),
        thought_signature: None,
    }];
    let msgs = vec![
        assistant,
        Message::tool_result("tc-valid", "read", ToolExecutionResult::new("found")),
        Message::new(MessageRole::Assistant, "here is the result"),
    ];
    let mut conv = Conversation::new(Uuid::nil(), "", "", "", "", "", "");
    conv.messages = msgs;
    let result = ContextManager::new().build_request_messages(&conv, SessionMode::Build);
    let tool_messages: Vec<_> = result
        .iter()
        .filter(|m| m.role == MessageRole::Tool)
        .collect();
    assert_eq!(tool_messages.len(), 1);
    assert_eq!(tool_messages[0].content, "found");
}

#[test]
fn build_request_messages_no_system_message() {
    let mut conv = Conversation::new(Uuid::nil(), "", "", "", "", "", "");
    conv.messages = vec![Message::new(MessageRole::System, "be helpful")];
    let result = ContextManager::new().build_request_messages(&conv, SessionMode::Build);
    assert_eq!(result.len(), 0);
}

#[tokio::test]
async fn persist_tool_result_appends_message_and_emits_event() {
    let (agent, _tmp) = agent_runtime();
    let session_id = Uuid::new_v4();
    {
        let store = agent.store.lock().await;
        store
            .create_session(
                session_id,
                &agent.workspace_root,
                "test-provider",
                "Test Provider",
                "test-model",
                "Test Model",
                "test-session",
            )
            .unwrap();
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let tool_call = ToolCall {
        id: "tc-1".to_string(),
        name: "read".to_string(),
        arguments: r#"{"path": "test.txt"}"#.to_string(),
        thought_signature: None,
    };
    let result = ToolExecutionResult::new("file content");

    agent
        .persist_tool_result(session_id, 1, &tool_call, &result, &tx)
        .await
        .unwrap();

    // Verify event was emitted
    let event = rx.recv().await;
    assert!(matches!(event, Some(BackendEvent::ToolCompleted { .. })));

    // Verify message was persisted
    let store = agent.store.lock().await;
    let messages = store.load_messages(session_id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, MessageRole::Tool);
}

#[tokio::test]
async fn inject_new_instructions_no_ops_when_no_instructions() {
    let (mut agent, _tmp) = agent_runtime();
    let session_id = Uuid::new_v4();
    {
        let store = agent.store.lock().await;
        store
            .create_session(
                session_id,
                &agent.workspace_root,
                "test-provider",
                "Test Provider",
                "test-model",
                "Test Model",
                "test-session",
            )
            .unwrap();
        // Create a clean user message.
        let mut msg = Message::new(MessageRole::User, "hello");
        store.append_message(session_id, &msg).unwrap();
        msg.id = store
            .load_messages(session_id)
            .unwrap()
            .into_iter()
            .find(|m| m.role == MessageRole::User)
            .unwrap()
            .id;
        drop(store);
        // With no instruction files configured, injection should do nothing.
        let result = agent
            .inject_new_instructions(session_id, &mut msg)
            .await
            .unwrap();
        assert!(!result, "should not inject when no instructions");
    }
}
