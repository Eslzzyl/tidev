//! Integration tests for tidev-agent types and helper functions.
//!
//! These tests use real SessionStore instances (backed by tempfile databases)
//! to verify message persistence, system prompt composition, and type behavior.

use tempfile::TempDir;

use tidev_storage::SessionStore;
use tidev_session::session::{BackendEvent, Message, MessageRole, ToolCall, ToolExecutionResult};

use crate::persistence::{persist_assistant_message, persist_message, persist_tool_result};
use crate::types::{compose_static_system_prompt, AgentDefinition, SharedAgentState};

// ── Helper ────────────────────────────────────────────────────────────────

fn setup_store() -> (SessionStore, TempDir) {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let store = SessionStore::open(&db_path).unwrap();
    (store, tmp)
}

fn make_store_arc(
    store: SessionStore,
) -> std::sync::Arc<tokio::sync::Mutex<SessionStore>> {
    std::sync::Arc::new(tokio::sync::Mutex::new(store))
}

// ── compose_static_system_prompt ──────────────────────────────────────────

#[test]
fn compose_static_system_prompt_contains_workspace_info() {
    let tmp = TempDir::new().unwrap();
    let prompt = compose_static_system_prompt("You are a test agent.", tmp.path());
    assert!(prompt.contains("You are a test agent."));
    assert!(prompt.contains("Working directory"));
    assert!(prompt.contains("Workspace root folder"));
    assert!(prompt.contains("Is directory a git repo"));
}

#[test]
fn compose_static_system_prompt_with_empty_base() {
    let tmp = TempDir::new().unwrap();
    let prompt = compose_static_system_prompt("", tmp.path());
    // Should still contain env info even with empty base prompt
    assert!(prompt.contains("Working directory"));
    assert!(prompt.contains("Workspace root folder"));
}

#[test]
fn compose_static_system_prompt_trims_base() {
    let tmp = TempDir::new().unwrap();
    let prompt = compose_static_system_prompt("  Hello World  ", tmp.path());
    assert!(prompt.starts_with("Hello World"));
}

// ── AgentDefinition ───────────────────────────────────────────────────────

#[test]
fn agent_definition_explorer_defaults() {
    let def = AgentDefinition::new(tidev_types::agent::AgentType::Explorer);
    assert_eq!(def.display_name, "explorer");
    assert!(def.read_only);
    assert!(def.allowed_tools.is_some());
    let tools = def.allowed_tools.as_ref().unwrap();
    assert!(tools.contains(&"grep".to_string()));
    assert!(!tools.contains(&"write".to_string()));
}

#[test]
fn agent_definition_general_allows_all_tools() {
    let def = AgentDefinition::new(tidev_types::agent::AgentType::General);
    assert!(!def.read_only);
    assert!(def.allowed_tools.is_none());
}

#[test]
fn agent_definition_bootstrap_content() {
    let def = AgentDefinition::new(tidev_types::agent::AgentType::Fixer);
    let content = def.bootstrap_content();
    assert!(content.contains("Fixer"));
    assert!(content.contains("full tool access"));
}

// ── SharedAgentState ──────────────────────────────────────────────────────

#[test]
fn shared_agent_state_queue_and_pop() {
    let state = SharedAgentState::new();
    assert!(state.pop_queued_message().is_none());

    use crate::QueuedUserMessage;
    state.queue_user_message(QueuedUserMessage {
        content: "hello".to_string(),
        attachments: vec![],
        mode: Some(tidev_types::prompts::SessionMode::Build),
        thinking_level: Some(tidev_config::reasoning::ThinkingLevelType::None),
    });

    let msg = state.pop_queued_message();
    assert!(msg.is_some());
    assert_eq!(msg.unwrap().content, "hello");
    assert!(state.pop_queued_message().is_none());
}

// ── Persistence helpers ──────────────────────────────────────────────────

#[tokio::test]
async fn persist_message_appends_to_store() {
    let (store, _tmp) = setup_store();
    let store_arc = make_store_arc(store);
    let session_id = uuid::Uuid::new_v4();

    // Create session first
    {
        let s = store_arc.lock().await;
        s.create_session(
            session_id,
            std::path::Path::new("/tmp"),
            "test-provider",
            "Test",
            "test-model",
            "Test Model",
            "test",
        )
        .unwrap();
    }

    let msg = Message::new(MessageRole::User, "test message");
    persist_message(&store_arc, session_id, &msg)
        .await
        .unwrap();

    let s = store_arc.lock().await;
    let messages = s.load_messages(session_id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "test message");
    assert_eq!(messages[0].role, MessageRole::User);
}

#[tokio::test]
async fn persist_tool_result_appends_message_and_emits_event() {
    let (store, _tmp) = setup_store();
    let store_arc = make_store_arc(store);
    let session_id = uuid::Uuid::new_v4();

    // Create session
    {
        let s = store_arc.lock().await;
        s.create_session(
            session_id,
            std::path::Path::new("/tmp"),
            "test-provider",
            "Test",
            "test-model",
            "Test Model",
            "test",
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

    persist_tool_result(&store_arc, session_id, 1, &tool_call, &result, &tx)
        .await
        .unwrap();

    // Verify event was emitted
    let event = rx.recv().await;
    assert!(matches!(event, Some(BackendEvent::ToolCompleted { .. })));

    // Verify message was persisted
    let s = store_arc.lock().await;
    let messages = s.load_messages(session_id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, MessageRole::Tool);
    assert_eq!(messages[0].content, "file content");
}

#[tokio::test]
async fn persist_assistant_message_stores_full_turn() {
    let (store, _tmp) = setup_store();
    let store_arc = make_store_arc(store);
    let session_id = uuid::Uuid::new_v4();

    // Create session
    {
        let s = store_arc.lock().await;
        s.create_session(
            session_id,
            std::path::Path::new("/tmp"),
            "test-provider",
            "Test",
            "test-model",
            "Test Model",
            "test",
        )
        .unwrap();
    }

    let turn = tidev_session::session::AssistantTurn {
        content: "Hello, I am an assistant.".to_string(),
        reasoning: "thinking...".to_string(),
        tool_calls: vec![ToolCall {
            id: "tc-1".to_string(),
            name: "read".to_string(),
            arguments: "{}".to_string(),
            thought_signature: None,
        }],
        input_tokens: Some(10),
        output_tokens: Some(20),
        total_tokens: Some(30),
        ..Default::default()
    };

    let msg = persist_assistant_message(&store_arc, session_id, &turn)
        .await
        .unwrap();

    assert_eq!(msg.role, MessageRole::Assistant);
    assert!(!msg.streaming);
    assert_eq!(msg.content, "Hello, I am an assistant.");
    assert_eq!(msg.reasoning, "thinking...");
    assert_eq!(msg.tool_calls.len(), 1);
    assert_eq!(msg.input_tokens, Some(10));
    assert_eq!(msg.output_tokens, Some(20));

    // Verify it was persisted
    let s = store_arc.lock().await;
    let messages = s.load_messages(session_id).unwrap();
    assert_eq!(messages.len(), 1);
}

// ── AgentType ──────────────────────────────────────────────────────────────

#[test]
fn agent_type_parse_all_types() {
    for agent in tidev_types::agent::AgentType::all() {
        let name = agent.display_name();
        let parsed = tidev_types::agent::AgentType::parse(name);
        assert_eq!(parsed, Some(*agent));
    }
}

#[test]
fn agent_type_read_only_check() {
    assert!(tidev_types::agent::AgentType::Explorer.is_read_only());
    assert!(tidev_types::agent::AgentType::Oracle.is_read_only());
    assert!(!tidev_types::agent::AgentType::Fixer.is_read_only());
    assert!(!tidev_types::agent::AgentType::Designer.is_read_only());
}

// ── ToolExecResult construction ──────────────────────────────────────────

#[test]
fn tool_exec_result_roundtrip() {
    let result = crate::ToolExecResult {
        tool_call_id: "tc-1".to_string(),
        tool_name: "read".to_string(),
        result: ToolExecutionResult::new("output"),
    };
    assert_eq!(result.tool_call_id, "tc-1");
    assert_eq!(result.tool_name, "read");
    assert_eq!(result.result.output, "output");
}
