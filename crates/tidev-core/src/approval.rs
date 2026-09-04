//! Host-owned approval messages and decisions.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{mpsc::UnboundedSender, oneshot};

use tidev_llm::message::{ToolCall, ToolExecutionResult};

/// A tool call with an optional rejection reason.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovedTool {
    pub tool_call: ToolCall,
    /// If `Some`, this result is returned to the model instead of executing
    /// the tool.
    pub rejection: Option<ToolExecutionResult>,
    /// Pre-generated child session ID for a subagent tool.
    pub child_session_id: Option<uuid::Uuid>,
    /// Whether this tool call may access paths outside the workspace.
    pub allow_outside: bool,
    /// Whether this tool call may read sensitive files.
    pub sensitive_file_approved: bool,
    /// Optional user-supplied explanation for the decision.
    pub user_reason: Option<String>,
}

/// A tool call augmented with pre-computed violation information for an
/// approval frontend.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallWithViolations {
    pub tool_call: ToolCall,
    pub workspace_boundary_violation: Option<PathBuf>,
    pub sensitive_file_violation: Option<PathBuf>,
}

/// Request sent by the host to a frontend that can approve tools.
///
/// The request is deliberately transport-neutral.  In particular, it does
/// not carry a Tokio channel: a WebSocket, ACP, or TUI frontend can serialize
/// the request and later answer it through [`ApprovalBroker::respond`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrontendRequest {
    pub request_id: uuid::Uuid,
    pub session_id: uuid::Uuid,
    pub kind: FrontendRequestKind,
}

/// Host approval request variants.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FrontendRequestKind {
    ToolApproval(Vec<ToolCallWithViolations>),
}

/// Response sent by an approval frontend.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FrontendResponse {
    ToolApproval(Vec<ApprovedTool>),
}

/// Errors returned by the frontend approval broker.
#[derive(Debug, thiserror::Error)]
pub enum ApprovalError {
    #[error("approval request channel is closed")]
    ChannelClosed,
    #[error("approval request {0} is no longer pending")]
    NotPending(uuid::Uuid),
    #[error("approval response channel was closed")]
    ResponseClosed,
}

/// Frontend-neutral broker for approval requests.
///
/// Core owns the pending response channels.  Frontends only observe
/// [`FrontendRequest`] values and answer by request ID, which keeps approval
/// semantics identical for TUI, ACP, and future remote clients.
#[derive(Clone)]
pub struct ApprovalBroker {
    request_tx: UnboundedSender<FrontendRequest>,
    pending: Arc<StdMutex<HashMap<uuid::Uuid, PendingRequest>>>,
}

struct PendingRequest {
    request: FrontendRequest,
    response_tx: oneshot::Sender<FrontendResponse>,
}

impl ApprovalBroker {
    pub(crate) fn new(request_tx: UnboundedSender<FrontendRequest>) -> Self {
        Self {
            request_tx,
            pending: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    /// Publish a request and await the first response, or cancellation.
    pub(crate) async fn request(
        &self,
        session_id: uuid::Uuid,
        kind: FrontendRequestKind,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<FrontendResponse, ApprovalError> {
        let request_id = uuid::Uuid::new_v4();
        let (response_tx, response_rx) = oneshot::channel();
        let request = FrontendRequest {
            request_id,
            session_id,
            kind,
        };
        self.pending
            .lock()
            .expect("approval broker mutex poisoned")
            .insert(
                request_id,
                PendingRequest {
                    request: request.clone(),
                    response_tx,
                },
            );
        if self.request_tx.send(request).is_err() {
            self.pending
                .lock()
                .expect("approval broker mutex poisoned")
                .remove(&request_id);
            return Err(ApprovalError::ChannelClosed);
        }

        tokio::select! {
            _ = cancel.cancelled() => {
                self.pending
                    .lock()
                    .expect("approval broker mutex poisoned")
                    .remove(&request_id);
                Err(ApprovalError::ResponseClosed)
            }
            response = response_rx => {
                response.map_err(|_| ApprovalError::ResponseClosed)
            }
        }
    }

    /// Answer an approval request.  Removing the entry before sending makes
    /// the first response win deterministically when multiple frontends are
    /// connected.
    pub fn respond(
        &self,
        request_id: uuid::Uuid,
        response: FrontendResponse,
    ) -> Result<(), ApprovalError> {
        let pending = self
            .pending
            .lock()
            .expect("approval broker mutex poisoned")
            .remove(&request_id)
            .ok_or(ApprovalError::NotPending(request_id))?;
        pending
            .response_tx
            .send(response)
            .map_err(|_| ApprovalError::ResponseClosed)
    }

    /// Return the approvals that still await a frontend response.
    ///
    /// Request subscribers use this snapshot after registering so a frontend
    /// that reconnects can render requests that were already in flight.
    pub fn pending_requests(&self) -> Vec<FrontendRequest> {
        let mut requests: Vec<_> = self
            .pending
            .lock()
            .expect("approval broker mutex poisoned")
            .values()
            .map(|pending| pending.request.clone())
            .collect();
        requests.sort_by_key(|request| request.request_id);
        requests
    }

    /// Cancel a pending request without delivering a response.
    pub fn cancel(&self, request_id: uuid::Uuid) -> bool {
        self.pending
            .lock()
            .expect("approval broker mutex poisoned")
            .remove(&request_id)
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn first_frontend_response_wins() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let broker = ApprovalBroker::new(tx);
        let cancel = tokio_util::sync::CancellationToken::new();
        let waiting_broker = broker.clone();
        let session_id = uuid::Uuid::new_v4();

        let waiting = tokio::spawn(async move {
            waiting_broker
                .request(
                    session_id,
                    FrontendRequestKind::ToolApproval(Vec::new()),
                    &cancel,
                )
                .await
        });
        let request = rx.recv().await.expect("request should be published");
        let pending = broker.pending_requests();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request_id, request.request_id);

        broker
            .respond(
                request.request_id,
                FrontendResponse::ToolApproval(Vec::new()),
            )
            .expect("first response should be accepted");
        assert!(broker.pending_requests().is_empty());
        assert!(matches!(
            broker.respond(
                request.request_id,
                FrontendResponse::ToolApproval(Vec::new())
            ),
            Err(ApprovalError::NotPending(id)) if id == request.request_id
        ));
        assert!(matches!(
            waiting.await.expect("task should join"),
            Ok(FrontendResponse::ToolApproval(tools)) if tools.is_empty()
        ));
    }
}
