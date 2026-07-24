//! Layered Action enum definitions.
//!
//! Domain sub-actions are grouped by module; the top-level `Action` provides
//! unified routing.

use std::path::PathBuf;

use uuid::Uuid;

use crate::theme::ThemeName;
use tidev_types::message::MessageAttachment;

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
    Set(ThemeName),
    Preview(ThemeName),
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
    SaveApiKey { provider_id: String, key: String },
    /// Prune orphan auth entries whose provider is no longer in config.
    PruneOrphans,
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
    ImageViewer {
        data: Vec<u8>,
        filename: String,
    },
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
    Connect(ConnectAction),

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
    /// Result from a PermissionDialog (final approve / reject).
    /// `reason` is an optional user-supplied text attached when denying.
    PermissionResponse {
        decision: PermissionDecision,
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
}
