use ratatui::{style::Color, text::Line};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use uuid::Uuid;

use crate::tooling::FileReadStamp;

use super::at_mention::AtMentionState;
use super::input::SnippetState;
use super::mcp_panel::McpPanelState;
use super::message_panel::MessagePanelState;
use super::model_panel::ModelPanelState;
use super::mouse_selection::MouseSelectionState;
use super::permission::{
    PendingToolExecution, PermissionDialogState, RunningSubagentExecution, RunningToolExecution,
};
use super::question::QuestionDialogState;
use super::session_panel::SessionPanelState;
use super::theme_panel::ThemePanelState;
use crate::app::ui::agents_panel::AgentsPanelState;
use crate::app::ui::rename::RenameSessionDialogState;
use crate::{
    app::commands::CommandPaletteState,
    app::input::Composer,
    config::ActiveModel,
    context::ContextManager,
    provider_setup::ConnectDialog,
    session::{Conversation, MessageAttachment},
    tooling::TodoItem,
};

pub(crate) const MESSAGE_RENDER_CACHE_MAX_ENTRIES: usize = 1200;

/// State for scrollbar drag interaction.
#[derive(Clone, Debug, Default)]
pub(crate) struct ScrollbarDragState {
    /// The scroll position (in content lines) at the start of the drag.
    pub(crate) start_scroll: usize,
    /// The mouse Y position at the start of the drag.
    pub(crate) start_mouse_y: u16,
    /// The maximum scroll value.
    pub(crate) max_scroll: usize,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ContextUsage {
    pub(crate) input_tokens: u32,
    pub(crate) output_tokens: u32,
    pub(crate) total_tokens: u32,
    pub(crate) cache_read_tokens: u32,
    pub(crate) cache_write_tokens: u32,
    pub(crate) model_id: String,
    pub(crate) tokens_per_second: Option<f32>,
}

/// A block in the message layout index representing a renderable unit.
///
/// Each block contains either:
/// - A single User/System/Error message
/// - An Assistant message with its associated Tool results
///
/// The layout index enables O(log n) lookup of visible messages via binary search,
/// avoiding full traversal of all messages on every frame.
#[derive(Clone, Debug)]
pub(crate) struct MessageBlock {
    /// ID of the primary message in this block
    #[allow(dead_code)]
    pub(crate) message_id: Uuid,
    /// Starting index in the messages array
    pub(crate) message_start_idx: usize,
    /// Number of messages in this block (1 for User/System/Error, 1+ for Assistant+Tool group)
    pub(crate) message_count: usize,
    /// Starting line number in the rendered output
    pub(crate) start_line: usize,
    /// Total lines consumed by this block
    pub(crate) line_count: usize,
}

/// Layout index for efficient viewport virtualization.
///
/// This structure maintains a mapping from messages to their positions in the
/// rendered output, enabling:
/// 1. Binary search to find visible messages without rendering everything
/// 2. Incremental updates when only the last few messages change
/// 3. Accurate scroll position calculations
///
/// The index is invalidated when:
/// - Window width changes (line counts become invalid)
/// - Messages are added/removed
/// - Cache is cleared
#[derive(Clone, Debug, Default)]
pub(crate) struct MessageLayoutIndex {
    /// Sorted list of message blocks
    pub(crate) blocks: Vec<MessageBlock>,
    /// Total lines across all blocks
    pub(crate) total_lines: usize,
    /// Width used for calculating line counts
    pub(crate) width: usize,
    /// Whether the index is valid and up-to-date
    pub(crate) valid: bool,
    /// Whether the index was last built while assistant streaming was active.
    pub(crate) contains_streaming_messages: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MessageRenderCacheKind {
    Cards,
    ToolCall(String),
}

#[derive(Clone, Debug)]
pub(crate) struct SelectableRegionRange {
    pub start_line: usize,
    pub end_line: usize,
    pub min_x: u16,
    pub max_x: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MessageRenderCacheKey {
    pub(crate) session_id: Uuid,
    pub(crate) message_id: Uuid,
    pub(crate) width: usize,
    pub(crate) is_round_end: bool,
    pub(crate) kind: MessageRenderCacheKind,
}

#[derive(Clone, Debug)]
pub(crate) enum MessageRenderCacheValue {
    Cards(Vec<(Color, Vec<Line<'static>>)>),
    ToolResult(Vec<Line<'static>>, Vec<SelectableRegionRange>),
}

#[derive(Clone, Debug)]
pub(crate) struct MessageRenderCacheEntry {
    pub(crate) value: MessageRenderCacheValue,
    pub(crate) last_used_tick: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Screen {
    Welcome,
    Chat,
}

#[derive(Clone, Debug)]
pub(crate) struct QueuedPrompt {
    pub(crate) prompt: String,
    pub(crate) attachments: Vec<MessageAttachment>,
}

impl QueuedPrompt {
    pub(crate) fn new(prompt: impl Into<String>, attachments: Vec<MessageAttachment>) -> Self {
        Self {
            prompt: prompt.into(),
            attachments,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CachedSessionRuntime {
    pub(crate) conversation: Conversation,
    pub(crate) active_model: ActiveModel,
    pub(crate) context_manager: ContextManager,
    pub(crate) pending_tool_execution: Option<PendingToolExecution>,
    pub(crate) permission_dialog: Option<PermissionDialogState>,
    pub(crate) question_dialog: Option<QuestionDialogState>,
    pub(crate) running_tool_executions: Vec<RunningToolExecution>,
    pub(crate) running_subagent_executions: Vec<RunningSubagentExecution>,
    pub(crate) pending_request: bool,
    pub(crate) pending_prompt_queue: std::collections::VecDeque<QueuedPrompt>,
    pub(crate) active_request_id: u64,
    pub(crate) abort_confirmation_deadline: Option<Instant>,
    pub(crate) retrying_hint: Option<(u32, u32, String, Option<u32>)>,
    pub(crate) message_scroll_offset: usize,
    pub(crate) message_follow_tail: bool,
    pub(crate) message_viewport_lines: usize,
    pub(crate) message_total_lines: usize,
    pub(crate) context_usage: Option<ContextUsage>,
    pub(crate) todos: Vec<TodoItem>,
    /// Cached file read records for this session.
    pub(crate) file_reads: Option<HashMap<PathBuf, FileReadStamp>>,
    pub(crate) loaded_instruction_sources: Vec<String>,
    /// Cached instruction file contents to avoid redundant I/O
    #[allow(dead_code)]
    pub(crate) instruction_content_cache: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct UiStateSnapshot {
    pub(crate) screen: Screen,
    pub(crate) connect_dialog: Option<ConnectDialog>,
    pub(crate) theme_panel: Option<ThemePanelState>,
    pub(crate) model_panel: Option<ModelPanelState>,
    pub(crate) message_panel: Option<MessagePanelState>,
    pub(crate) session_panel: Option<SessionPanelState>,
    pub(crate) rename_dialog: Option<RenameSessionDialogState>,
    pub(crate) mcp_panel: Option<McpPanelState>,
    pub(crate) agents_panel: Option<AgentsPanelState>,
    pub(crate) at_mention: AtMentionState,
    pub(crate) snippet_state: SnippetState,
    pub(crate) command_palette: CommandPaletteState,
    pub(crate) leader_key_pending: bool,
    pub(crate) composer: Composer,
    pub(crate) draft_attachments: Vec<MessageAttachment>,
    pub(crate) last_notice: Option<String>,
    pub(crate) toast: Option<(String, Instant)>,
    pub(crate) mouse_selection: MouseSelectionState,
}
