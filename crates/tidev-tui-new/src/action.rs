//! Layered Action enum definitions.
//!
//! Domain sub-actions are grouped by module; the top-level `Action` provides
//! unified routing.

use anyhow::Result;
use uuid::Uuid;

use tidev_types::message::MessageAttachment;
use tidev_tui::theme::ThemeName;

/// Panel action identifiers (launcher target).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelAction {
    Agents,
    Message,
    Model,
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
/// Note: `Loaded`/`Deleted` contain `anyhow::Result` (not `Clone`),
/// so this enum only derives `Debug`, not `Clone`.
#[derive(Debug)]
pub(crate) enum SessionAction {
    Create,
    Select(Uuid),
    Delete(Uuid),
    Rename(Uuid, String),
    Fork(Uuid),
    Undo,
    Redo,
    Compact,
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
// OverlayKind
// ---------------------------------------------------------------------------

/// Identifier for every overlay component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OverlayKind {
    ImageViewer,
    CommandPalette,
    PanelLauncher,
    PermissionDialog,
    QuestionDialog,
    WorkspaceBoundaryDialog,
    SensitiveFileDialog,
    ForkConfirmDialog,
    UndoConfirmDialog,
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
    Command(CommandAction),

    // ── Internal ──
    Noop,
    Error(String),
}
