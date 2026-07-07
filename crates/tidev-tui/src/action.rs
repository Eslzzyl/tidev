//! Layered Action enum definitions.
//!
//! Domain sub-actions are grouped by module; the top-level `Action` provides
//! unified routing.

use std::path::PathBuf;

use anyhow::Result;
use uuid::Uuid;

use tidev_types::message::MessageAttachment;
use tidev_tui_old::theme::ThemeName;

/// Panel action identifiers (launcher target).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelAction {
    Agents,
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

/// Summary of a session, used for display in the SessionPanel.
#[derive(Clone, Debug)]
pub(crate) struct SessionSummary {
    pub id: Uuid,
    pub title: String,
    pub message_count: usize,
}

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
    Delete(Uuid),
    DeleteBatch(Vec<Uuid>),
    Rename(Uuid, String),
    Fork(Uuid),
    Undo,
    Redo,
    Compact,
    ExportBatch(Vec<Uuid>),
    /// Reload session list from store (after view mode change).
    Reload,
    /// Async result from a session load operation.
    Loaded(Result<Vec<SessionSummary>>),
    Deleted(Result<()>),
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
    CancelGeneration,
    ScrollTo(Uuid),
    ScrollDelta(isize),
    ToggleToolResult(Uuid),
    ToggleImage(Uuid),
    /// Streaming delta for incremental message rendering.
    StreamDelta {
        message_id: Uuid,
        delta: String,
    },
    StreamEnd(Uuid),
}

/// Overlay (panel/dialog) management.
#[derive(Clone, Debug)]
pub(crate) enum OverlayAction {
    Open(OverlayKind),
    Close(OverlayKind),
    CloseTop,
    CloseAll,
}

/// Theme management actions.
#[derive(Clone, Debug)]
pub(crate) enum ThemeAction {
    Set(ThemeName),
    Toggle,
    Preview(ThemeName),
}

/// Panel launcher (quick-open panel) actions.
#[derive(Clone, Debug)]
pub(crate) enum LauncherAction {
    Open,
    Close,
    Select(usize),
    Execute(PanelAction),
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

/// Connect/configure LLM provider actions.
#[derive(Clone, Debug)]
pub(crate) enum ConnectAction {
    /// Save an API key for the given LLM provider.
    SaveApiKey {
        provider_id: String,
        key: String,
    },
    /// Prune orphan auth entries whose provider is no longer in config.
    PruneOrphans,
}

/// Async command execution result.
///
/// Note: `result` contains `anyhow::Result` (not `Clone`),
/// so this enum only derives `Debug`, not `Clone`.
#[derive(Debug)]
pub(crate) enum CommandAction {
    Response {
        id: Uuid,
        result: Result<Box<[u8]>>,
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

/// User decision for the main tool permission dialog.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PermissionDecision {
    /// Y — allow this one time.
    Allow,
    /// R — allow and persist to DB.
    AllowAndRemember,
    /// N / Esc — deny this one time.
    Deny,
    /// X — deny and persist to DB.
    DenyAndRemember,
}

// ---------------------------------------------------------------------------
// OverlayKind
// ---------------------------------------------------------------------------

/// Identifier for every overlay component.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OverlayKind {
    ImageViewer,
    CommandPalette,
    PanelLauncher,
    PermissionDialog,
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
    Notifications,
}

// ---------------------------------------------------------------------------
// Top-level Action
// ---------------------------------------------------------------------------

/// Top-level Action — the universal message type for all component communication.
#[derive(Debug)]
pub(crate) enum Action {
    // ── Lifecycle ──
    Tick,
    Render,
    Resize(u16, u16),
    Quit,

    // ── Domain ──
    Session(SessionAction),
    Chat(ChatAction),
    Overlay(OverlayAction),
    Theme(ThemeAction),
    Launcher(LauncherAction),
    Search(SearchAction),
    Connect(ConnectAction),
    Command(CommandAction),

    // ── Tool approval pipeline ──
    /// Result from a WorkspaceBoundaryDialog.
    WorkspaceBoundaryResponse {
        path: PathBuf,
        decision: BoundaryDecision,
    },
    /// Result from a SensitiveFileDialog.
    SensitiveFileResponse {
        path: PathBuf,
        decision: SensitiveFileDecision,
    },
    /// Result from a PermissionDialog (final approve / reject).
    PermissionResponse {
        decision: PermissionDecision,
    },
    /// Result from a QuestionDialog.
    /// `None` means the dialog was dismissed (rejected).
    QuestionResponse {
        output: Option<String>,
    },

    // ── Internal ──
    Noop,
    Error(String),
}
