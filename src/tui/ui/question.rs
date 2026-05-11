use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use textwrap::wrap;
use tokio::runtime::Runtime;

use crate::{
    session::{ToolCall, ToolExecutionResult},
    tooling::{QuestionArgs, QuestionInfo},
};

use super::App;

#[derive(Clone, Debug)]
pub(crate) struct QuestionDialogState {
    pub tool_call: ToolCall,
    pub questions: Vec<QuestionInfo>,
    pub current_index: usize,
    pub answers: Vec<Vec<String>>,
    pub selected_indices: Vec<usize>,
    pub custom_inputs: Vec<String>,
    pub editing_custom: bool,
}

impl QuestionDialogState {
    pub(crate) fn new(tool_call: ToolCall, questions: Vec<QuestionInfo>) -> Self {
        let answer_count = questions.len();
        Self {
            tool_call,
            questions,
            current_index: 0,
            answers: vec![Vec::new(); answer_count],
            selected_indices: vec![0; answer_count],
            custom_inputs: vec![String::new(); answer_count],
            editing_custom: false,
        }
    }

    pub(crate) fn total(&self) -> usize {
        self.questions.len()
    }

    pub(crate) fn is_last(&self) -> bool {
        self.current_index + 1 >= self.total()
    }

    pub(crate) fn current_question(&self) -> Option<&QuestionInfo> {
        self.questions.get(self.current_index)
    }

    pub(crate) fn title(&self) -> String {
        let count = self.total();
        if count == 0 {
            return "Questions".to_string();
        }

        let header = self
            .current_question()
            .map(|question| question.header.trim())
            .filter(|header| !header.is_empty())
            .unwrap_or("Questions");

        format!(
            "Question {} of {} · {}",
            self.current_index + 1,
            count,
            header
        )
    }

    pub(crate) fn body_title(&self) -> String {
        self.current_question()
            .map(|question| question.question.trim())
            .filter(|question| !question.is_empty())
            .unwrap_or("Ask a question")
            .to_string()
    }

    pub(crate) fn current_option_count(&self) -> usize {
        self.current_question()
            .map(|question| question.options.len() + usize::from(question.custom.unwrap_or(true)))
            .unwrap_or(0)
    }

    pub(crate) fn selected_index(&self) -> usize {
        self.selected_indices
            .get(self.current_index)
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn set_selected_index(&mut self, index: usize) {
        let count = self.current_option_count();
        if count == 0 {
            return;
        }

        if let Some(slot) = self.selected_indices.get_mut(self.current_index) {
            *slot = index.min(count.saturating_sub(1));
        }
    }

    pub(crate) fn move_selection(&mut self, step: isize) {
        let count = self.current_option_count();
        if count == 0 {
            return;
        }

        let current = self.selected_index() as isize;
        let next = (current + step).rem_euclid(count as isize) as usize;
        self.set_selected_index(next);
    }

    pub(crate) fn custom_option_index(&self) -> Option<usize> {
        self.current_question().and_then(|question| {
            question
                .custom
                .unwrap_or(true)
                .then_some(question.options.len())
        })
    }

    pub(crate) fn current_custom_input(&self) -> &str {
        self.custom_inputs
            .get(self.current_index)
            .map(|text| text.as_str())
            .unwrap_or("")
    }

    pub(crate) fn is_option_selected(&self, option_index: usize) -> bool {
        self.current_question()
            .and_then(|question| question.options.get(option_index))
            .map(|option| {
                let value = option.label.trim();
                self.answers
                    .get(self.current_index)
                    .is_some_and(|answers| answers.iter().any(|answer| answer == value))
            })
            .unwrap_or(false)
    }

    pub(crate) fn is_custom_answer_selected(&self) -> bool {
        let value = self.current_custom_input().trim();
        !value.is_empty()
            && self
                .answers
                .get(self.current_index)
                .is_some_and(|answers| answers.iter().any(|answer| answer == value))
    }

    pub(crate) fn start_custom_editing(&mut self) {
        self.editing_custom = true;
    }

    pub(crate) fn stop_custom_editing(&mut self) {
        self.editing_custom = false;
    }

    pub(crate) fn sync_current_custom_input(&mut self, text: &str) {
        let allow_multiple = self
            .current_question()
            .map(|question| question.multiple.unwrap_or(false))
            .unwrap_or(false);
        let previous = self.current_custom_input().trim().to_string();
        let normalized = text.trim().to_string();

        if let Some(slot) = self.custom_inputs.get_mut(self.current_index) {
            *slot = text.to_string();
        }

        if let Some(slot) = self.answers.get_mut(self.current_index) {
            if allow_multiple {
                if !previous.is_empty() {
                    slot.retain(|answer| answer != &previous);
                }
                if !normalized.is_empty() && !slot.iter().any(|answer| answer == &normalized) {
                    slot.push(normalized);
                }
            } else {
                *slot = if normalized.is_empty() {
                    Vec::new()
                } else {
                    vec![normalized]
                };
            }
        }
    }

    pub(crate) fn toggle_regular_option(&mut self, option_index: usize) {
        let Some(question) = self.current_question() else {
            return;
        };
        let Some(option) = question.options.get(option_index) else {
            return;
        };

        let value = option.label.trim().to_string();
        let allow_multiple = question.multiple.unwrap_or(false);

        if let Some(slot) = self.answers.get_mut(self.current_index) {
            if allow_multiple {
                if let Some(existing_index) = slot.iter().position(|answer| answer == &value) {
                    slot.remove(existing_index);
                } else {
                    slot.push(value);
                }
            } else {
                *slot = vec![value];
                if let Some(custom) = self.custom_inputs.get_mut(self.current_index) {
                    custom.clear();
                }
            }
        }
    }

    pub(crate) fn toggle_custom_option(&mut self) {
        let (allow_custom, allow_multiple) = self
            .current_question()
            .map(|question| {
                (
                    question.custom.unwrap_or(true),
                    question.multiple.unwrap_or(false),
                )
            })
            .unwrap_or((false, false));

        if !allow_custom {
            return;
        }

        if allow_multiple && self.is_custom_answer_selected() {
            self.sync_current_custom_input("");
            self.stop_custom_editing();
        } else {
            self.start_custom_editing();
        }
    }

    pub(crate) fn answer_placeholder(&self) -> String {
        "Type your own answer and press Enter".to_string()
    }

    pub(crate) fn current_answer_text(&self) -> String {
        self.answers
            .get(self.current_index)
            .map(|answer| answer.join(", "))
            .unwrap_or_default()
    }

    pub(crate) fn move_next(&mut self) {
        if !self.is_last() {
            self.current_index += 1;
            self.stop_custom_editing();
        }
    }

    pub(crate) fn move_previous(&mut self) {
        if self.current_index > 0 {
            self.current_index -= 1;
            self.stop_custom_editing();
        }
    }

    pub(crate) fn formatted_output(&self) -> String {
        let mut parts: Vec<String> = Vec::new();

        for (index, question) in self.questions.iter().enumerate() {
            let answer = self.answers.get(index).cloned().unwrap_or_default();
            let value = if answer.is_empty() {
                "Unanswered".to_string()
            } else {
                answer.join(", ")
            };
            // Multi-line format: clearly shows each question followed by its answer
            parts.push(format!("Q{}: {}", index + 1, question.question));
            parts.push(format!("A: {}", value));
        }

        parts.join("\n")
    }

    pub(crate) fn regular_options_lines(&self, width: u16) -> Vec<String> {
        let Some(question) = self.current_question() else {
            return vec!["No questions available.".to_string()];
        };

        let wrap_width = width.max(1) as usize;
        let mut lines = Vec::new();

        if question.options.is_empty() {
            if question.custom.unwrap_or(true) {
                push_wrapped_line(
                    &mut lines,
                    wrap_width,
                    "No predefined options were provided. Type a freeform answer below."
                        .to_string(),
                );
                return lines;
            }

            return vec!["No predefined options were provided.".to_string()];
        }

        push_wrapped_line(
            &mut lines,
            wrap_width,
            format!(
                "{}{}",
                if question.multiple.unwrap_or(false) {
                    "Select one or more options. "
                } else {
                    "Select one option. "
                },
                if question.custom.unwrap_or(true) {
                    "Type your own answer if needed."
                } else {
                    "Type the option number or label."
                }
            ),
        );

        for (index, option) in question.options.iter().enumerate() {
            let selected = self.selected_index() == index;
            let checked = if self.is_option_selected(index) {
                "✓"
            } else {
                " "
            };
            let cursor = if selected { ">" } else { " " };
            push_wrapped_line(
                &mut lines,
                wrap_width,
                format!(
                    "{} {}. [{}] {}",
                    cursor,
                    index + 1,
                    checked,
                    option.label.trim()
                ),
            );

            if let Some(description) = option
                .description
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                for wrapped in wrap(description, wrap_width.saturating_sub(4).max(1)) {
                    lines.push(format!("    {}", wrapped));
                }
            }
        }

        lines
    }

    pub(crate) fn custom_option_lines(&self, width: u16) -> Vec<String> {
        let Some(question) = self.current_question() else {
            return Vec::new();
        };

        if !question.custom.unwrap_or(true) {
            return Vec::new();
        }

        let wrap_width = width.max(1) as usize;
        let custom_index = question.options.len();
        let mut lines = Vec::new();
        let selected = self.selected_index() == custom_index;
        let checked = if self.is_custom_answer_selected() {
            "✓"
        } else {
            " "
        };
        let cursor = if selected { ">" } else { " " };
        push_wrapped_line(
            &mut lines,
            wrap_width,
            format!(
                "{} {}. [{}] Type your own answer",
                cursor,
                custom_index + 1,
                checked
            ),
        );

        let custom_input = self.current_custom_input().trim();
        if !custom_input.is_empty() {
            for wrapped in wrap(custom_input, wrap_width.saturating_sub(4).max(1)) {
                lines.push(format!("    {}", wrapped));
            }
        }

        lines
    }

    pub(crate) fn options_lines(&self, width: u16) -> Vec<String> {
        let mut lines = self.regular_options_lines(width);
        lines.extend(self.custom_option_lines(width));
        lines
    }

    pub(crate) fn prompt_height(&self, width: u16, input_height: u16) -> u16 {
        let regular_option_lines = self.regular_options_lines(width.saturating_sub(2));
        let custom_option_lines = self.custom_option_lines(width.saturating_sub(2));
        let input_height = if self.editing_custom { input_height } else { 0 };

        2u16.saturating_add(2)
            .saturating_add(regular_option_lines.len() as u16)
            .saturating_add(custom_option_lines.len() as u16)
            .saturating_add(input_height)
            .saturating_add(2)
    }
}

impl App {
    pub(crate) fn begin_question_dialog(
        &mut self,
        tool_call: ToolCall,
        args: QuestionArgs,
    ) -> Result<()> {
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.mcp_panel = None;
        self.command_palette.clear();
        self.at_mention.clear();
        self.draft_attachments.clear();

        let dialog = QuestionDialogState::new(tool_call, args.questions);
        self.question_dialog = Some(dialog);
        self.composer.clear();
        self.composer
            .set_placeholder("Type your own answer and press Enter");
        self.last_notice =
            Some("Use arrows to choose, Space to toggle, Enter to select or continue".to_string());
        Ok(())
    }

    pub(crate) fn handle_question_dialog_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        let Some(is_editing_custom) = self
            .question_dialog
            .as_ref()
            .map(|dialog| dialog.editing_custom)
        else {
            return Ok(());
        };

        if is_editing_custom {
            if matches!(key.code, KeyCode::Esc) {
                if let Some(dialog) = self.question_dialog.as_mut() {
                    dialog.stop_custom_editing();
                }
                self.composer.clear();
                self.composer
                    .set_placeholder("Type your own answer and press Enter");
                return Ok(());
            }

            if matches!(key.code, KeyCode::Enter)
                && !key.modifiers.contains(KeyModifiers::SHIFT)
                && !key.modifiers.contains(KeyModifiers::ALT)
            {
                let should_advance = {
                    let dialog = self
                        .question_dialog
                        .as_mut()
                        .expect("question dialog exists");
                    dialog.sync_current_custom_input(self.composer.text());
                    let allow_multiple = dialog
                        .current_question()
                        .is_some_and(|question| question.multiple.unwrap_or(false));
                    dialog.stop_custom_editing();
                    !allow_multiple
                };

                self.composer.clear();
                self.composer
                    .set_placeholder("Type your own answer and press Enter");

                if should_advance {
                    self.advance_question_dialog(runtime)?;
                }

                return Ok(());
            }

            let _ = self.composer.handle_key_with_history(key, false);
            self.ensure_input_cursor_visible();
            if let Some(dialog) = self.question_dialog.as_mut() {
                dialog.sync_current_custom_input(self.composer.text());
            }
            return Ok(());
        }

        if matches!(key.code, KeyCode::Esc) {
            // Record a tool result for the dismissed question before aborting,
            // so the assistant message's tool_calls remain balanced with a tool response.
            if let Some(dialog) = self.question_dialog.take() {
                self.record_tool_result(
                    dialog.tool_call,
                    ToolExecutionResult::new("Tool 'question' was dismissed by user"),
                )?;
                self.advance_pending_tool_execution();
            }
            self.composer.clear();
            self.composer
                .set_placeholder("Ask TiDev about your code, task, or question...");
            self.last_notice = Some("Question dialog dismissed, request stopped".to_string());
            self.abort_current_request();
            return Ok(());
        }

        if (matches!(key.code, KeyCode::Char('p')) && key.modifiers.contains(KeyModifiers::CONTROL))
            || (matches!(key.code, KeyCode::Left) && key.modifiers.is_empty())
        {
            if let Some(dialog) = self.question_dialog.as_mut() {
                dialog.move_previous();
            }
            self.composer.clear();
            self.composer
                .set_placeholder("Type your own answer and press Enter");
            return Ok(());
        }

        if (matches!(key.code, KeyCode::Char('n')) && key.modifiers.contains(KeyModifiers::CONTROL))
            || (matches!(key.code, KeyCode::Right) && key.modifiers.is_empty())
        {
            self.advance_question_dialog(runtime)?;
            return Ok(());
        }

        if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
            if let Some(dialog) = self.question_dialog.as_mut() {
                dialog.move_selection(-1);
            }
            return Ok(());
        }

        if matches!(key.code, KeyCode::Down | KeyCode::Char('j')) {
            if let Some(dialog) = self.question_dialog.as_mut() {
                dialog.move_selection(1);
            }
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char(' ')) {
            self.handle_question_dialog_selection(false, runtime)?;
            return Ok(());
        }

        if matches!(key.code, KeyCode::Enter)
            && !key.modifiers.contains(KeyModifiers::SHIFT)
            && !key.modifiers.contains(KeyModifiers::ALT)
        {
            self.handle_question_dialog_selection(true, runtime)?;
            return Ok(());
        }

        if let Some(dialog) = self.question_dialog.as_mut()
            && dialog
                .custom_option_index()
                .is_some_and(|index| index == dialog.selected_index())
            && matches!(
                key.code,
                KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete
            )
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
            && !key.modifiers.contains(KeyModifiers::SUPER)
        {
            let custom_text = dialog.current_custom_input().to_string();
            dialog.toggle_custom_option();
            self.composer.set_text(custom_text);
            self.composer
                .set_placeholder("Type your own answer and press Enter");
            let _ = self.composer.handle_key_with_history(key, false);
            if let Some(dialog) = self.question_dialog.as_mut() {
                dialog.sync_current_custom_input(self.composer.text());
            }
            return Ok(());
        }

        Ok(())
    }

    fn handle_question_dialog_selection(
        &mut self,
        from_enter: bool,
        runtime: &Runtime,
    ) -> Result<()> {
        let Some(dialog) = self.question_dialog.as_mut() else {
            return Ok(());
        };

        let selected_index = dialog.selected_index();
        let custom_index = dialog.custom_option_index();
        let is_custom_selected = custom_index.is_some_and(|index| index == selected_index);

        if is_custom_selected {
            dialog.toggle_custom_option();
            if dialog.editing_custom {
                self.composer
                    .set_text(dialog.current_custom_input().to_string());
                self.composer
                    .set_placeholder("Type your own answer and press Enter");
            } else {
                self.composer.clear();
            }
            return Ok(());
        }

        let allow_multiple = dialog
            .current_question()
            .is_some_and(|question| question.multiple.unwrap_or(false));

        dialog.toggle_regular_option(selected_index);

        if from_enter && !allow_multiple {
            self.advance_question_dialog(runtime)?;
        }

        Ok(())
    }

    fn advance_question_dialog(&mut self, runtime: &Runtime) -> Result<()> {
        if self
            .question_dialog
            .as_ref()
            .is_some_and(QuestionDialogState::is_last)
        {
            self.resolve_question_dialog(true, runtime)?;
            return Ok(());
        }

        if let Some(dialog) = self.question_dialog.as_mut() {
            dialog.move_next();
        }

        self.composer.clear();
        self.composer
            .set_placeholder("Type your own answer and press Enter");
        Ok(())
    }

    fn resolve_question_dialog(&mut self, allow: bool, runtime: &Runtime) -> Result<()> {
        let Some(dialog) = self.question_dialog.take() else {
            return Ok(());
        };

        if self.pending_permission_response.is_some() {
            // Permission channel mode — result is sent back via channel
            let output = if allow {
                dialog.formatted_output()
            } else {
                "Tool 'question' was dismissed by user".to_string()
            };
            let result = ToolExecutionResult::new(output);
            // Also add the tool result to the in-memory conversation so it
            // renders in the TUI tool cards.  record_tool_result with
            // is_runtime_flow = true skips persistence (avoiding duplication
            // with the runtime's persist_tool_result).
            self.record_tool_result(dialog.tool_call.clone(), result.clone())?;
            self.pending_rejected_tools.push((dialog.tool_call, result));
        } else if allow {
            let output = dialog.formatted_output();
            self.record_tool_result(dialog.tool_call, ToolExecutionResult::new(output))?;
        } else {
            self.record_tool_result(
                dialog.tool_call,
                ToolExecutionResult::new("Tool 'question' was dismissed by user"),
            )?;
        }

        self.composer.clear();
        self.composer
            .set_placeholder("Ask TiDev about your code, task, or question...");
        self.advance_pending_tool_execution();
        self.process_pending_tool_execution(runtime)
    }
}

fn push_wrapped_line(lines: &mut Vec<String>, wrap_width: usize, line: String) {
    let wrapped = wrap(&line, wrap_width);
    if wrapped.is_empty() {
        lines.push(String::new());
    } else {
        lines.extend(wrapped.into_iter().map(|line| line.into_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn question_dialog() -> QuestionDialogState {
        let tool_call = ToolCall {
            id: "call-1".to_string(),
            name: "question".to_string(),
            arguments: "{}".to_string(),
        };
        let questions: Vec<QuestionInfo> = serde_json::from_value(json!([
            {
                "question": "Pick one",
                "header": "Scope",
                "options": [
                    {
                        "label": "Alpha",
                        "description": "This description is intentionally long enough to wrap on a narrow terminal."
                    }
                ],
                "multiple": false,
                "custom": true
            }
        ]))
        .expect("question fixture should deserialize");

        QuestionDialogState::new(tool_call, questions)
    }

    #[test]
    fn prompt_height_accounts_for_wrapped_options() {
        let dialog = question_dialog();

        let wide = dialog.prompt_height(80, 4);
        let narrow = dialog.prompt_height(30, 4);

        assert!(narrow > wide);
    }

    #[test]
    fn prompt_height_adds_custom_input_when_editing() {
        let mut dialog = question_dialog();

        let collapsed = dialog.prompt_height(80, 4);
        dialog.start_custom_editing();
        let expanded = dialog.prompt_height(80, 4);

        assert!(expanded > collapsed);
    }

    #[test]
    fn editing_custom_input_stays_visible_in_options() {
        let mut dialog = question_dialog();
        dialog.start_custom_editing();
        dialog.sync_current_custom_input("Custom answer");

        let lines = dialog.options_lines(80);

        assert!(lines.iter().any(|line| line.contains("Custom answer")));
    }

    #[test]
    fn custom_option_is_last() {
        let dialog = question_dialog();
        let lines = dialog.options_lines(80);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("Type your own answer"))
        );
    }

    #[test]
    fn multi_select_toggles_answers() {
        let tool_call = ToolCall {
            id: "call-2".to_string(),
            name: "question".to_string(),
            arguments: "{}".to_string(),
        };
        let questions: Vec<QuestionInfo> = serde_json::from_value(json!([
            {
                "question": "Pick multiple",
                "header": "Multi",
                "options": [
                    {
                        "label": "One",
                        "description": "First"
                    },
                    {
                        "label": "Two",
                        "description": "Second"
                    }
                ],
                "multiple": true,
                "custom": true
            }
        ]))
        .expect("question fixture should deserialize");

        let mut dialog = QuestionDialogState::new(tool_call, questions);
        dialog.toggle_regular_option(0);
        dialog.toggle_regular_option(1);

        assert_eq!(
            dialog.answers[0],
            vec!["One".to_string(), "Two".to_string()]
        );

        dialog.set_selected_index(2);
        dialog.toggle_custom_option();
        dialog.sync_current_custom_input("Custom answer");

        assert!(dialog.answers[0].contains(&"Custom answer".to_string()));
    }

    #[test]
    fn formatted_output_shows_qa_pairs() {
        let mut dialog = question_dialog();
        dialog.toggle_regular_option(0);

        let output = dialog.formatted_output();
        assert!(
            output.contains("Q1: Pick one"),
            "should contain question: {}",
            output
        );
        assert!(
            output.contains("A: Alpha"),
            "should contain answer: {}",
            output
        );
    }

    #[test]
    fn formatted_output_shows_all_questions() {
        let tool_call = ToolCall {
            id: "call-3".to_string(),
            name: "question".to_string(),
            arguments: "{}".to_string(),
        };
        let questions: Vec<QuestionInfo> = serde_json::from_value(json!([
            {
                "question": "First question",
                "header": "First",
                "options": [{"label": "A", "description": "First opt"}],
                "multiple": false,
                "custom": false
            },
            {
                "question": "Second question",
                "header": "Second",
                "options": [{"label": "B", "description": "Second opt"}],
                "multiple": true,
                "custom": true
            }
        ]))
        .expect("question fixture should deserialize");

        let mut dialog = QuestionDialogState::new(tool_call, questions);
        dialog.toggle_regular_option(0);
        dialog.move_next();
        dialog.toggle_regular_option(0);
        dialog.sync_current_custom_input("Custom answer");

        let output = dialog.formatted_output();
        assert!(output.contains("Q1: First question"), "should contain Q1");
        assert!(output.contains("A: A"), "should contain A1 answer");
        assert!(output.contains("Q2: Second question"), "should contain Q2");
        assert!(
            output.contains("A: B, Custom answer"),
            "should contain A2 multi-answer: {}",
            output
        );
    }
}
