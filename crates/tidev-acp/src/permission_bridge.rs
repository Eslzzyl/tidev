//! Permission bridge: translates tidev [`TuiRequest`]s into ACP
//! [`session/request_permission`](acp::RequestPermissionRequest) requests.
//!
//! Runs as a background task that consumes `TuiRequest`s from the channel,
//! sends permission requests to the ACP client, waits for the user's
//! response, and sends back [`TuiResponse`]s.

use agent_client_protocol::schema::v1 as acp;
use tidev_core::{ApprovedTool, TuiRequest, TuiRequestKind, TuiResponse};
use tidev_types::message::ToolExecutionResult;

use crate::types::tool_kind;

/// Spawn the permission bridge task.
///
/// This task consumes [`TuiRequest`]s from the given channel, converts each
/// to an ACP [`session/request_permission`] request, sends it to the client
/// via the given connection, waits for the response, and sends back a
/// [`TuiResponse`] through the original oneshot channel.
pub fn spawn(
    mut request_rx: tokio::sync::mpsc::UnboundedReceiver<TuiRequest>,
    cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(request) = request_rx.recv().await {
            match request.kind {
                TuiRequestKind::ToolApproval(tools_with_violations) => {
                    let acp_session_id =
                        acp::SessionId::new(request.session_id.to_string());
                    let mut approved_tools = Vec::new();

                    for twv in &tools_with_violations {
                        let tc = &twv.tool_call;

                        // Determine the permission reason and options.
                        let reason = build_permission_reason(twv);
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

                        let raw_input: Option<serde_json::Value> =
                            serde_json::from_str(&tc.arguments).ok();
                        let tool_call_update = acp::ToolCallUpdate::new(
                            tc.id.clone(),
                            acp::ToolCallUpdateFields::new()
                                .title(Some(crate::types::tool_title(tc)))
                                .kind(Some(tool_kind(tc)))
                                .locations(crate::types::tool_locations(tc))
                                .raw_input(raw_input),
                        );

                        let permission_request = acp::RequestPermissionRequest::new(
                            acp_session_id.clone(),
                            tool_call_update,
                            options,
                        );

                        // Send the request and wait for the client's response.
                        let response = cx
                            .send_request(permission_request)
                            .block_task()
                            .await;

                        match response {
                            Ok(resp) => {
                                let approved = matches!(
                                    resp.outcome,
                                    acp::RequestPermissionOutcome::Selected(ref selected)
                                    if selected.option_id.to_string() == "approve"
                                );

                                if approved {
                                    approved_tools.push(ApprovedTool {
                                        tool_call: tc.clone(),
                                        rejection: None,
                                        child_session_id: None,
                                        allow_outside: twv
                                            .workspace_boundary_violation
                                            .is_some(),
                                        sensitive_file_approved: twv
                                            .sensitive_file_violation
                                            .is_some(),
                                        user_reason: Some(reason),
                                    });
                                } else {
                                    approved_tools.push(ApprovedTool {
                                        tool_call: tc.clone(),
                                        rejection: Some(
                                            ToolExecutionResult::new(format!(
                                                "Denied by user: {reason}"
                                            )),
                                        ),
                                        child_session_id: None,
                                        allow_outside: false,
                                        sensitive_file_approved: false,
                                        user_reason: Some(reason),
                                    });
                                }
                            }
                            Err(_) => {
                                // Request failed — treat as rejection.
                                approved_tools.push(ApprovedTool {
                                    tool_call: tc.clone(),
                                    rejection: Some(ToolExecutionResult::new(
                                        format!("Permission request failed for: {}", tc.name),
                                    )),
                                    child_session_id: None,
                                    allow_outside: false,
                                    sensitive_file_approved: false,
                                    user_reason: None,
                                });
                            }
                        }
                    }

                    // Send the combined response back.
                    let _ = request
                        .response_tx
                        .send(TuiResponse::ToolApproval(approved_tools));
                }
            }
        }
        log::info!("ACP permission bridge: channel closed, shutting down");
    })
}

/// Build a human-readable reason for why permission is being requested.
fn build_permission_reason(twv: &tidev_core::ToolCallWithViolations) -> String {
    let tc = &twv.tool_call;

    if tc.name == "question" {
        return "Agent is requesting user input via the question tool".to_string();
    }

    if let Some(ref path) = twv.workspace_boundary_violation {
        return format!(
            "Tool '{}' accesses path outside workspace: {}",
            tc.name,
            path.display()
        );
    }

    if let Some(ref path) = twv.sensitive_file_violation {
        return format!(
            "Tool '{}' accesses sensitive file: {}",
            tc.name,
            path.display()
        );
    }

    format!("Permission required for tool: {}", tc.name)
}
