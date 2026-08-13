//! Layered Action enum definitions.
//!
//! Domain sub-actions are grouped by module; the top-level `Action` provides
//! unified routing.

use std::path::PathBuf;

use uuid::Uuid;

use tidev_llm::message::MessageAttachment;

/// Panel action identifiers (launcher target).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelAction {
    Agents,
    McpServers,
    Message,
    Model,
    Search,
    Session,
    Settings,
    Skills,
    Theme,
}

// ---------------------------------------------------------------------------
// SessionSummary
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Domain sub-actions
// ---------------------------------------------------------------------------

/// Session management actions.
///
/// Note: variants containing `Result` only derive `Debug`, not `Clone`.
#[derive(Debug)]
pub(crate) enum SessionAction {
    Create,
    Select(Uuid),
    Rename(Uuid, String),
    Fork(Uuid),
    Undo,
    Redo,
    Compact,
    /// Cycle thinking level (Shift+Tab / Ctrl+T).
    CycleThinkingLevel,
    /// Reload session list from store (after view mode change).
    Reload,
}

/// Chat/conversation actions.
#[derive(Clone, Debug)]
pub(crate) enum ChatAction {
    SendMessage {
        text: String,
        attachments: Vec<MessageAttachment>,
    },
    /// Replace the composer input with the given text (e.g. `/skill name`).
    SetInput(String),
    ScrollTo(Uuid),
    ScrollDelta(isize),
    /// Expand every thinking block in the current session (one-shot).
    ExpandAllThinking,
    /// Collapse every thinking block in the current session (one-shot).
    CollapseAllThinking,
}

/// Overlay (panel/dialog) management.
#[derive(Clone, Debug)]
pub(crate) enum OverlayAction {
    Open(OverlayKind),
    Close(OverlayKind),
}

/// Theme management actions.
#[derive(Clone, Debug)]
pub(crate) enum ThemeAction {
    Set(String),
}

/// Search provider panel actions.
#[derive(Clone, Debug)]
pub(crate) enum SearchAction {
    /// Switch to the given provider and persist to config.
    SwitchProvider(String),
    /// Save an API key (or Google CX) to auth store.
    SaveApiKey {
        provider: String,
        key: String,
        is_cx: bool,
    },
}

/// Settings that can be changed from the settings panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingKey {
    NotificationEnabled,
    LoggingEnabled,
    LogLevel,
    SaveRequestBody,
    SaveResponseBody,
    ScrollSpeed,
    AllowSensitiveFileAccess,
    AllowOutsideWorkspaceAccess,
    SubagentEnabled,
    CollapseThinking,
    CollapseDiffs,
    SendWhileBusy,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SettingValue {
    Bool(bool),
    Number(f32),
    Choice(String),
}

#[derive(Clone, Debug)]
pub(crate) enum SettingsAction {
    Change {
        key: SettingKey,
        value: SettingValue,
    },
}

/// Connect/configure LLM provider actions.
#[derive(Clone, Debug)]
pub(crate) enum ConnectAction {
    /// Save an API key for the given LLM provider.
    SaveApiKey { provider_id: String, key: String },
    /// Remove a provider's API key (disconnect).
    Disconnect {
        provider_id: String,
        display_name: String,
    },
    /// Prune orphan auth entries whose provider is no longer in config.
    PruneOrphans,
}

/// MCP server management actions.
#[derive(Clone, Debug)]
pub(crate) enum McpAction {
    /// Connect or disconnect a server by name.
    Toggle(String),
    /// Refresh / reconnect a server.
    Refresh(String),
    /// Remove a server entirely (memory + config).
    Remove(String),
    /// Add a new server, or update an existing one.
    /// `original_name` is `Some` when editing and the name hasn't changed,
    /// or `None` when adding a brand new server.
    Upsert {
        name: String,
        config: tidev_config::mcp::McpServerConfig,
        original_name: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Tool approval pipeline types
// ---------------------------------------------------------------------------

/// User decision for a workspace boundary violation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BoundaryDecision {
    /// Allow this one-time access.
    AllowOnce,
    /// Allow and remember in memory until exit.
    AllowUntilExit,
    /// Deny this one-time access.
    DenyOnce,
    /// Deny and remember in memory until exit.
    DenyUntilExit,
}

/// User decision for a sensitive file access violation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SensitiveFileDecision {
    AllowOnce,
    AllowUntilExit,
    DenyOnce,
    DenyUntilExit,
}

// ---------------------------------------------------------------------------
// OverlayKind
// ---------------------------------------------------------------------------

/// Identifier for every overlay component.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OverlayKind {
    ImageViewer {
        data: Vec<u8>,
        filename: String,
    },
    PanelLauncher,
    QuestionDialog,
    WorkspaceBoundaryDialog,
    SensitiveFileDialog,
    ForkConfirmDialog {
        message_id: Uuid,
        message_count: usize,
    },
    UndoConfirmDialog {
        message_id: Uuid,
        content: String,
    },
    ConnectDialog,
    RenameDialog,
    SessionPanel,
    SettingsPanel,
    ThemePanel,
    ModelPanel,
    AgentsPanel,
    SkillsPanel,
    SearchPanel,
    MessagePanel,
    McpServerPanel,
}

// ---------------------------------------------------------------------------
// Top-level Action
// ---------------------------------------------------------------------------

/// Top-level Action — the universal message type for all component communication.
#[derive(Debug)]
pub(crate) enum Action {
    // ── Lifecycle ──
    Quit,

    // ── Domain ──
    Session(SessionAction),
    Chat(ChatAction),
    Overlay(OverlayAction),
    Theme(ThemeAction),
    Search(SearchAction),
    Settings(SettingsAction),
    Connect(ConnectAction),
    Mcp(McpAction),

    // ── Tool approval pipeline ──
    /// Result from a WorkspaceBoundaryDialog.
    /// `reason` is an optional user-supplied text attached when denying.
    WorkspaceBoundaryResponse {
        path: PathBuf,
        decision: BoundaryDecision,
        reason: Option<String>,
    },
    /// Result from a SensitiveFileDialog.
    /// `reason` is an optional user-supplied text attached when denying.
    SensitiveFileResponse {
        path: PathBuf,
        decision: SensitiveFileDecision,
        reason: Option<String>,
    },
    /// Result from a QuestionDialog.
    /// `None` means the dialog was dismissed (rejected).
    QuestionResponse {
        output: Option<String>,
    },

    // ── Internal ──
    /// Show a one-line status notice at the bottom of the screen.
    Notice(String),
    Noop,
    /// The event was consumed by an overlay with no side effect (e.g. a
    /// scroll tick inside a panel that is already at its edge). Unlike
    /// `Noop`, this prevents scroll fall-through to the chat area.
    Consumed,
}
