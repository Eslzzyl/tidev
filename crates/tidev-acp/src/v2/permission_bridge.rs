//! Bridge tidev permission requests to ACP v2 permission requests.

use std::sync::Arc;

use agent_client_protocol::schema::v2 as acp;
use tidev_core::{ApprovedTool, TuiRequest, TuiRequestKind, TuiResponse};
use tidev_llm::message::ToolExecutionResult;
use tokio::sync::RwLock;

pub(crate) fn spawn(
    mut request_rx: tokio::sync::mpsc::UnboundedReceiver<TuiRequest>,
    cx: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>,
    supports_elicitation: Arc<RwLock<bool>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(request) = request_rx.recv().await {
            let TuiRequestKind::ToolApproval(ref tools) = request.kind;
            let session_id = acp::SessionId::new(request.session_id.to_string());
            let mut approved = Vec::new();

            for item in tools {
                let tool = &item.tool_call;
                let reason = permission_reason(item);
                if tidev_utils::tool_name::canonical_tool_name(&tool.name) == Some("question") {
                    let approved_tool = handle_question(
                        &request,
                        item,
                        &session_id,
                        &cx,
                        *supports_elicitation.read().await,
                    )
                    .await;
                    approved.push(approved_tool);
                    continue;
                }
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
                    crate::v2::types::tool_call_update(tool, Some(acp::ToolCallStatus::Pending)),
                );
                let request = acp::RequestPermissionRequest::new(
                    session_id.clone(),
                    crate::common::tool::tool_title(tool),
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

async fn handle_question(
    _request: &TuiRequest,
    item: &tidev_core::ToolCallWithViolations,
    session_id: &acp::SessionId,
    cx: &agent_client_protocol::ConnectionTo<agent_client_protocol::Client>,
    supports_elicitation: bool,
) -> ApprovedTool {
    let tool = &item.tool_call;
    let rejection = if !supports_elicitation {
        ToolExecutionResult::new(
            "The ACP v2 client does not advertise form elicitation support for the question tool.",
        )
    } else {
        match build_question_request(tool, session_id) {
            Ok(elicitation) => match cx.send_request(elicitation).block_task().await {
                Ok(response) => question_response(response, tool),
                Err(error) => {
                    ToolExecutionResult::new(format!("Question elicitation failed: {error}"))
                }
            },
            Err(error) => ToolExecutionResult::new(error),
        }
    };

    ApprovedTool {
        tool_call: tool.clone(),
        rejection: Some(rejection),
        child_session_id: None,
        allow_outside: false,
        sensitive_file_approved: false,
        user_reason: Some(permission_reason(item)),
    }
}

fn build_question_request(
    tool: &tidev_llm::message::ToolCall,
    session_id: &acp::SessionId,
) -> Result<acp::CreateElicitationRequest, String> {
    let args: serde_json::Value = serde_json::from_str(&tool.arguments)
        .map_err(|error| format!("Invalid question arguments: {error}"))?;
    let questions = args
        .get("questions")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "Question tool arguments must contain a questions array".to_string())?;
    if questions.is_empty() {
        return Err("Question tool received an empty questions array".to_string());
    }

    let mut schema = acp::ElicitationSchema::new();
    for (index, question) in questions.iter().enumerate() {
        let name = format!("q{}", index + 1);
        let prompt = question
            .get("question")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("Question {} is missing its text", index + 1))?;
        let header = question
            .get("header")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(prompt);
        let options = question
            .get("options")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let enum_options = options
            .iter()
            .filter_map(|option| {
                let label = option.get("label")?.as_str()?;
                let description = option
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned);
                Some(acp::EnumOption::new(label, label).description(description))
            })
            .collect::<Vec<_>>();
        let multiple = question
            .get("multiple")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let custom = question
            .get("custom")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if multiple && custom {
            return Err(format!(
                "Question {} cannot combine multiple selection with custom answers",
                index + 1
            ));
        }
        let property = if multiple {
            if enum_options.is_empty() {
                return Err(format!(
                    "Question {} allows multiple answers but has no options",
                    index + 1
                ));
            }
            acp::ElicitationPropertySchema::Array(
                acp::MultiSelectPropertySchema::titled(enum_options)
                    .title(header)
                    .description(prompt),
            )
        } else {
            let description = if custom {
                format!("{prompt} (You may enter a custom answer.)")
            } else {
                prompt.to_string()
            };
            let string_schema = acp::StringPropertySchema::new()
                .title(header)
                .description(description)
                .one_of((!custom && !enum_options.is_empty()).then_some(enum_options));
            acp::ElicitationPropertySchema::String(string_schema)
        };
        schema = schema.property(name, property, true);
    }

    Ok(acp::CreateElicitationRequest::new(
        acp::ElicitationFormMode::new(
            acp::ElicitationSessionScope::new(session_id.clone()),
            schema,
        ),
        "Please answer the questions from tidev",
    ))
}

fn question_response(
    response: acp::CreateElicitationResponse,
    tool: &tidev_llm::message::ToolCall,
) -> ToolExecutionResult {
    match response.action {
        acp::ElicitationAction::Accept(action) => {
            let content = action.content.unwrap_or_default();
            let questions = serde_json::from_str::<serde_json::Value>(&tool.arguments)
                .ok()
                .and_then(|args| args.get("questions").cloned())
                .and_then(|questions| questions.as_array().cloned())
                .unwrap_or_default();
            let output = content
                .into_iter()
                .map(|(name, value)| {
                    let index = name
                        .strip_prefix('q')
                        .and_then(|value| value.parse::<usize>().ok())
                        .and_then(|value| value.checked_sub(1));
                    let prompt = index
                        .and_then(|index| questions.get(index))
                        .and_then(|question| question.get("question"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(name.as_str());
                    let question_number = index.map_or_else(
                        || name.trim_start_matches('q').to_string(),
                        |index| (index + 1).to_string(),
                    );
                    format!(
                        "Q{}: {}\nA: {}",
                        question_number,
                        prompt,
                        elicitation_value(value)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            ToolExecutionResult::new(output)
        }
        acp::ElicitationAction::Decline => ToolExecutionResult::new("User declined the questions."),
        acp::ElicitationAction::Cancel => ToolExecutionResult::new("User cancelled the questions."),
        _ => ToolExecutionResult::new("The client returned an unsupported elicitation action."),
    }
}

fn elicitation_value(value: acp::ElicitationContentValue) -> String {
    match value {
        acp::ElicitationContentValue::String(value) => value,
        acp::ElicitationContentValue::Integer(value) => value.to_string(),
        acp::ElicitationContentValue::Number(value) => value.to_string(),
        acp::ElicitationContentValue::Boolean(value) => value.to_string(),
        acp::ElicitationContentValue::StringArray(values) => values.join(", "),
        _ => "(unsupported value)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn tool(arguments: &str) -> tidev_llm::message::ToolCall {
        tidev_llm::message::ToolCall {
            id: "question-1".to_string(),
            name: "question".to_string(),
            arguments: arguments.to_string(),
            thought_signature: None,
        }
    }

    #[test]
    fn question_schema_preserves_single_and_multiple_choices() {
        let request = build_question_request(
            &tool(r#"{
                "questions": [
                    {"question":"Pick one","header":"One","options":[{"label":"A"},{"label":"B"}]},
                    {"question":"Pick many","header":"Many","multiple":true,"options":[{"label":"X"},{"label":"Y"}]}
                ]
            }"#),
            &acp::SessionId::new("session-1"),
        )
        .unwrap();
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["mode"], "form");
        assert_eq!(
            value["requestedSchema"]["properties"]["q1"]["oneOf"][0]["const"],
            "A"
        );
        assert_eq!(
            value["requestedSchema"]["properties"]["q2"]["type"],
            "array"
        );
    }

    #[test]
    fn accepted_question_response_contains_question_and_answer() {
        let response = acp::CreateElicitationResponse::new(acp::ElicitationAction::Accept(
            acp::ElicitationAcceptAction::new().content(BTreeMap::from([(
                "q1".to_string(),
                acp::ElicitationContentValue::from("A"),
            )])),
        ));
        let result = question_response(
            response,
            &tool(r#"{"questions":[{"question":"Pick one"}]}"#),
        );
        assert_eq!(result.output, "Q1: Pick one\nA: A");
    }
}
