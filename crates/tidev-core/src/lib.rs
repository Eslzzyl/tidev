//! tidev-core: core orchestration layer.

pub mod agent_ctx;
pub mod agent_type;
pub mod approval;
pub mod attachment;
pub mod backend_event;
mod event_hub;
pub mod git;
pub mod mcp;
pub mod message_buf;
pub mod mode;
pub mod prompts;
pub mod registry;
pub mod runtime;
pub mod session;
pub mod session_message;
pub mod system_info;
mod tool_adapter;
mod tool_def;
pub mod undo;
pub mod workspace;

pub use agent_ctx::CoreContext;
pub use approval::{
    ApprovalBroker, ApprovalError, ApprovedTool, FrontendRequest, FrontendRequestKind,
    FrontendResponse, ToolCallWithViolations,
};
pub use backend_event::{BackendEvent, agent_event_to_backend_event};
pub use event_hub::{EventCursor, EventEnvelope, EventReplay, EventSubscription};
pub use git::{
    GitChangeKind, GitCommitSummary, GitDiffFile, GitDiffScope, GitDiffSnapshot, GitError,
    GitFileStatus, GitHistoryPage, GitRepoInfo, GitService, GitStatusCounts, GitStatusSnapshot,
};
pub use message_buf::CoreMessageBuffer;
pub use mode::Mode;
pub use registry::ToolRegistry;
pub use runtime::{PromptSubmission, PromptSubmissionReceipt, Runtime};
pub use session::SessionManager;
pub use session_message::SessionMessage;
pub use tidev_agent::StreamEndStatus;
pub use tidev_agent::{CompactionResult, ContextManager};

// Re-export storage/snapshot types so tidev-tui can use them
// without depending on tidev-storage / tidev-snapshot directly.
pub use tidev_snapshot::FileDiff;
pub use tidev_storage::{
    InterruptionData, InterruptionReason, MessageAppData, ProviderErrorData, SessionRecord,
    UsageActiveSessionBucket, UsageActivityDay, UsageModelMixBucket, UsageRecord,
    UsageRequestSizeBucket, UsageRhythmCell,
};
