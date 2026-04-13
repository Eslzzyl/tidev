use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use tokio::runtime::Runtime;

use crate::session::ToolCall;

use super::App;

#[derive(Clone, Debug)]
pub(crate) struct PendingToolExecution {
    tool_calls: Vec<ToolCall>,
    next_index: usize,
}

impl PendingToolExecution {
    pub(crate) fn new(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            tool_calls,
            next_index: 0,
        }
    }

    pub(crate) fn current(&self) -> Option<&ToolCall> {
        self.tool_calls.get(self.next_index)
    }

    pub(crate) fn current_index(&self) -> usize {
        self.next_index + 1
    }

    pub(crate) fn total(&self) -> usize {
        self.tool_calls.len()
    }

    pub(crate) fn advance(&mut self) {
        self.next_index = self.next_index.saturating_add(1);
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.next_index >= self.tool_calls.len()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PermissionDialogState {
    pub tool_call: ToolCall,
    pub current_index: usize,
    pub total: usize,
}

impl PermissionDialogState {
    pub(crate) fn title(&self) -> String {
        format!(
            "Approve tool call {} of {} · {}",
            self.current_index, self.total, self.tool_call.name
        )
    }
}

impl App {
    pub(crate) fn begin_tool_execution(
        &mut self,
        tool_calls: Vec<ToolCall>,
        runtime: &Runtime,
    ) -> Result<()> {
        self.pending_tool_execution = Some(PendingToolExecution::new(tool_calls));
        self.process_pending_tool_execution(runtime)
    }

    pub(crate) fn handle_permission_dialog_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.resolve_permission_prompt(true, false, runtime)?;
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.resolve_permission_prompt(true, true, runtime)?;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.resolve_permission_prompt(false, false, runtime)?;
            }
            KeyCode::Char('x') | KeyCode::Char('X') => {
                self.resolve_permission_prompt(false, true, runtime)?;
            }
            _ => {}
        }

        Ok(())
    }

    pub(crate) fn process_pending_tool_execution(&mut self, runtime: &Runtime) -> Result<()> {
        let Some(_) = self.pending_tool_execution.as_ref() else {
            return Ok(());
        };

        loop {
            let Some((tool_call, current_index, total)) = self.pending_tool_snapshot() else {
                break;
            };

            if !self.tools.can_execute(&tool_call.name, self.mode) {
                let output = format!(
                    "Tool '{}' is not available in {} mode",
                    tool_call.name,
                    self.mode.as_str()
                );
                self.record_tool_result(tool_call, output)?;
                self.advance_pending_tool_execution();
                continue;
            }

            if let Some(remembered) = self
                .store
                .load_tool_permission(self.conversation.session_id, &tool_call.name)?
            {
                let output = if remembered {
                    self.tools
                        .execute_call(&self.store, self.conversation.session_id, &tool_call)
                        .unwrap_or_else(|error| format!("Tool failed: {error}"))
                } else {
                    format!(
                        "Tool '{}' was denied by remembered permission",
                        tool_call.name
                    )
                };
                self.record_tool_result(tool_call, output)?;
                self.advance_pending_tool_execution();
                continue;
            }

            let Some(definition) = self.tools.definition_for(&tool_call.name) else {
                let output = format!("Tool '{}' is unknown", tool_call.name);
                self.record_tool_result(tool_call, output)?;
                self.advance_pending_tool_execution();
                continue;
            };

            if definition.needs_confirmation() {
                self.last_notice = Some(format!(
                    "Approve tool call {} of {}: {}",
                    current_index, total, tool_call.name
                ));
                self.permission_dialog = Some(PermissionDialogState {
                    tool_call,
                    current_index,
                    total,
                });
                return Ok(());
            }

            let output = self
                .tools
                .execute_call(&self.store, self.conversation.session_id, &tool_call)
                .unwrap_or_else(|error| format!("Tool failed: {error}"));
            self.record_tool_result(tool_call, output)?;
            self.advance_pending_tool_execution();
        }

        if self
            .pending_tool_execution
            .as_ref()
            .is_some_and(PendingToolExecution::is_finished)
        {
            self.pending_tool_execution = None;
            self.start_assistant_turn(runtime)?;
        }

        Ok(())
    }

    fn resolve_permission_prompt(
        &mut self,
        allow: bool,
        remember: bool,
        runtime: &Runtime,
    ) -> Result<()> {
        let Some(dialog) = self.permission_dialog.take() else {
            return Ok(());
        };

        if remember {
            self.store.remember_tool_permission(
                self.conversation.session_id,
                &dialog.tool_call.name,
                allow,
            )?;
        }

        let output = if allow {
            self.tools
                .execute_call(&self.store, self.conversation.session_id, &dialog.tool_call)
                .unwrap_or_else(|error| format!("Tool failed: {error}"))
        } else if remember {
            format!("Tool '{}' was denied and remembered", dialog.tool_call.name)
        } else {
            format!("Tool '{}' was denied", dialog.tool_call.name)
        };

        self.record_tool_result(dialog.tool_call, output)?;
        self.advance_pending_tool_execution();
        self.process_pending_tool_execution(runtime)
    }

    fn record_tool_result(&mut self, tool_call: ToolCall, output: String) -> Result<()> {
        self.store.append_tool_event(
            self.conversation.session_id,
            &tool_call.name,
            &tool_call.arguments,
            &output,
        )?;

        let message = crate::session::Message::tool_result(tool_call.id, tool_call.name, output);
        self.conversation.push(message.clone());
        self.store
            .append_message(self.conversation.session_id, &message)?;
        Ok(())
    }

    fn advance_pending_tool_execution(&mut self) {
        if let Some(execution) = self.pending_tool_execution.as_mut() {
            execution.advance();
        }
    }

    fn pending_tool_snapshot(&self) -> Option<(ToolCall, usize, usize)> {
        let execution = self.pending_tool_execution.as_ref()?;
        let tool_call = execution.current()?.clone();
        Some((tool_call, execution.current_index(), execution.total()))
    }
}
