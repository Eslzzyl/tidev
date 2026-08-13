use anyhow::{Context, Result, bail};
use std::io::Read;
use std::path::{Path, PathBuf};

use tidev_core::{Mode, Runtime, TuiRequestKind, TuiResponse};

pub async fn run(
    workspace: Option<PathBuf>,
    instruction: Option<String>,
    instruction_file: Option<PathBuf>,
    read_stdin: bool,
) -> Result<()> {
    let prompt = read_prompt(instruction, instruction_file, read_stdin)?;
    let workspace = workspace.unwrap_or(std::env::current_dir()?);
    let workspace = std::fs::canonicalize(&workspace)
        .with_context(|| format!("failed to resolve workspace {}", workspace.display()))?;

    let runtime = Runtime::builder().workspace_root(workspace).build().await?;
    let mut request_rx = runtime.request_rx().await;
    let session_id = runtime.create_default_session("Headless session")?;

    runtime
        .submit_prompt(session_id, prompt, Mode::Build)
        .await?;

    let approval_task = tokio::spawn(async move {
        while let Some(request) = request_rx.recv().await {
            let TuiRequestKind::ToolApproval(tool_calls) = request.kind;
            let decisions = tool_calls
                .into_iter()
                .map(|pending| tidev_core::ApprovedTool {
                    tool_call: pending.tool_call.clone(),
                    rejection: Some(tidev_llm::message::ToolExecutionResult::new(
                        "Tool request rejected: no interactive approval is available in headless mode.",
                    )),
                    child_session_id: None,
                    allow_outside: false,
                    sensitive_file_approved: false,
                    user_reason: Some("headless mode has no interactive approval prompt".into()),
                })
                .collect();
            if request
                .response_tx
                .send(TuiResponse::ToolApproval(decisions))
                .is_err()
            {
                break;
            }
        }
    });

    let run_result = runtime.wait_for_session(session_id).await;
    approval_task.abort();
    let _ = approval_task.await;
    runtime.shutdown().await;
    run_result?;

    let messages = runtime
        .session_manager()
        .load_session_messages(session_id)?;
    let final_content = messages
        .iter()
        .rev()
        .find(|message| {
            message.role == tidev_llm::message::MessageRole::Assistant
                && message.tool_calls.is_empty()
        })
        .map(|message| message.content.clone());

    if let Some(content) = final_content {
        print!("{content}");
        if !content.ends_with('\n') {
            println!();
        }
    }
    Ok(())
}

fn read_prompt(
    instruction: Option<String>,
    instruction_file: Option<PathBuf>,
    read_stdin: bool,
) -> Result<String> {
    match (instruction, instruction_file, read_stdin) {
        (Some(text), None, false) => Ok(text),
        (None, Some(path), false) => read_file(&path),
        (None, None, true) => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .context("failed to read instruction from stdin")?;
            Ok(text)
        }
        _ => bail!("specify exactly one of --instruction, --instruction-file, or --stdin"),
    }
}

fn read_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("failed to read instruction file {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::read_prompt;
    use anyhow::Result;

    #[test]
    fn prompt_text_is_preserved() -> Result<()> {
        let prompt = read_prompt(Some(" task\n\n".into()), None, false)?;
        assert_eq!(prompt, " task\n\n");
        Ok(())
    }

    #[test]
    fn prompt_sources_are_mutually_exclusive() {
        assert!(read_prompt(Some("text".into()), Some("task.md".into()), false).is_err());
        assert!(read_prompt(None, None, false).is_err());
    }
}
