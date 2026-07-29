//! Bridge tidev permission requests to ACP v2 permission requests.

use agent_client_protocol::schema::v2 as acp;
use tidev_core::{ApprovedTool, TuiRequest, TuiRequestKind, TuiResponse};
use tidev_types::message::ToolExecutionResult;

pub(crate) fn spawn(
    mut request_rx: tokio::sync::mpsc::UnboundedReceiver<TuiRequest>,
    cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(request) = request_rx.recv().await {
            let TuiRequestKind::ToolApproval(tools) = request.kind;
            let session_id = acp::SessionId::new(request.session_id.to_string());
            let mut approved = Vec::new();

            for item in &tools {
                let tool = &item.tool_call;
                let reason = permission_reason(item);
                let options = vec![
                    acp::PermissionOption::new(
                        "approve",
                        "Approve",
                        acp::PermissionOptionKind::AllowOnce,
                    ),
                    acp::PermissionOption::new(
                        "deny",
                        "Deny",
                        acp::PermissionOptionKind::RejectOnce,
                    ),
                ];
                let subject = acp::ToolCallPermissionSubject::new(
                    crate::v2_types::tool_call_update(tool, Some(acp::ToolCallStatus::Pending)),
                );
                let request = acp::RequestPermissionRequest::new(
                    session_id.clone(),
                    crate::types::tool_title(tool),
                    options,
                )
                .description(reason.clone())
                .subject(acp::RequestPermissionSubject::from(subject));

                let result = cx.send_request(request).block_task().await;
                let allowed = result
                    .as_ref()
                    .ok()
                    .map(|response| {
                        matches!(
                            response.outcome,
                            acp::RequestPermissionOutcome::Selected(ref selected)
                                if selected.option_id.to_string() == "approve"
                        )
                    })
                    .unwrap_or(false);

                if allowed {
                    approved.push(ApprovedTool {
                        tool_call: tool.clone(),
                        rejection: None,
                        child_session_id: None,
                        allow_outside: item.workspace_boundary_violation.is_some(),
                        sensitive_file_approved: item.sensitive_file_violation.is_some(),
                        user_reason: Some(reason),
                    });
                } else {
                    let message = if result.is_err() {
                        format!("Permission request failed for: {}", tool.name)
                    } else {
                        format!("Denied by user: {reason}")
                    };
                    approved.push(ApprovedTool {
                        tool_call: tool.clone(),
                        rejection: Some(ToolExecutionResult::new(message)),
                        child_session_id: None,
                        allow_outside: false,
                        sensitive_file_approved: false,
                        user_reason: Some(reason),
                    });
                }
            }
            let _ = request
                .response_tx
                .send(TuiResponse::ToolApproval(approved));
        }
    })
}

fn permission_reason(item: &tidev_core::ToolCallWithViolations) -> String {
    if item.tool_call.name == "question" {
        return "Agent is requesting user input via the question tool".to_string();
    }
    if let Some(path) = &item.workspace_boundary_violation {
        return format!(
            "Tool '{}' accesses path outside workspace: {}",
            item.tool_call.name,
            path.display()
        );
    }
    if let Some(path) = &item.sensitive_file_violation {
        return format!(
            "Tool '{}' accesses sensitive file: {}",
            item.tool_call.name,
            path.display()
        );
    }
    format!("Permission required for tool: {}", item.tool_call.name)
}
