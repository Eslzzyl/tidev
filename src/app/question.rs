use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
}

impl QuestionDialogState {
    pub(crate) fn new(tool_call: ToolCall, questions: Vec<QuestionInfo>) -> Self {
        let answer_count = questions.len();
        Self {
            tool_call,
            questions,
            current_index: 0,
            answers: vec![Vec::new(); answer_count],
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

        format!("Question {} of {} · {}", self.current_index + 1, count, header)
    }

    pub(crate) fn body_title(&self) -> String {
        self.current_question()
            .map(|question| question.question.trim())
            .filter(|question| !question.is_empty())
            .unwrap_or("Ask a question")
            .to_string()
    }

    pub(crate) fn answer_placeholder(&self) -> String {
        let custom = self
            .current_question()
            .and_then(|question| question.custom)
            .unwrap_or(true);

        if self.current_question().is_some_and(|question| question.multiple.unwrap_or(false)) {
            if custom {
                "Type comma-separated answers and press Enter".to_string()
            } else {
                "Type comma-separated option labels and press Enter".to_string()
            }
        } else if custom {
            "Type your answer and press Enter".to_string()
        } else {
            "Type an option label and press Enter".to_string()
        }
    }

    pub(crate) fn current_answer_text(&self) -> String {
        self.answers
            .get(self.current_index)
            .map(|answer| answer.join(", "))
            .unwrap_or_default()
    }

    pub(crate) fn set_current_answer_from_text(&mut self, text: &str) {
        let Some(question) = self.current_question() else {
            return;
        };

        let answers = parse_answer_text(text, question);
        if let Some(slot) = self.answers.get_mut(self.current_index) {
            *slot = answers;
        }
    }

    pub(crate) fn move_next(&mut self) {
        if !self.is_last() {
            self.current_index += 1;
        }
    }

    pub(crate) fn move_previous(&mut self) {
        if self.current_index > 0 {
            self.current_index -= 1;
        }
    }

    pub(crate) fn formatted_output(&self) -> String {
        let formatted = self
            .questions
            .iter()
            .enumerate()
            .map(|(index, question)| {
                let answer = self.answers.get(index).cloned().unwrap_or_default();
                let value = if answer.is_empty() {
                    "Unanswered".to_string()
                } else {
                    answer.join(", ")
                };
                format!("\"{}\"=\"{}\"", question.question, value)
            })
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            "User has answered your questions: {formatted}. You can now continue with the user's answers in mind."
        )
    }

    pub(crate) fn prompt_height(&self, input_height: u16) -> u16 {
        let option_lines = self
            .current_question()
            .map(|question| {
                let option_count = question.options.len() as u16;
                if option_count == 0 {
                    2
                } else {
                    option_count.saturating_add(1)
                }
            })
            .unwrap_or(2);

        2u16
            .saturating_add(2)
            .saturating_add(option_lines)
            .saturating_add(input_height)
            .saturating_add(2)
    }
}

impl App {
    pub(crate) fn begin_question_dialog(&mut self, tool_call: ToolCall, args: QuestionArgs) -> Result<()> {
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
            .set_placeholder("Type your answer and press Enter");
        self.last_notice = Some("Answer the question to continue".to_string());
        Ok(())
    }

    pub(crate) fn handle_question_dialog_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        if self.question_dialog.is_none() {
            return Ok(());
        }

        if matches!(key.code, KeyCode::Esc) {
            self.resolve_question_dialog(false, runtime)?;
            return Ok(());
        }

        if (matches!(key.code, KeyCode::Char('p')) && key.modifiers.contains(KeyModifiers::CONTROL))
            || (matches!(key.code, KeyCode::Left) && key.modifiers.is_empty())
        {
            let placeholder = {
                let dialog = self.question_dialog.as_mut().expect("question dialog exists");
                dialog.set_current_answer_from_text(self.composer.text());
                dialog.move_previous();
                let answer_text = dialog.current_answer_text();
                let placeholder = dialog.answer_placeholder();
                self.composer.set_text(answer_text);
                placeholder
            };
            self.composer.set_placeholder(placeholder);
            return Ok(());
        }

        if (matches!(key.code, KeyCode::Char('n')) && key.modifiers.contains(KeyModifiers::CONTROL))
            || (matches!(key.code, KeyCode::Right) && key.modifiers.is_empty())
        {
            let placeholder = {
                let dialog = self.question_dialog.as_mut().expect("question dialog exists");
                dialog.set_current_answer_from_text(self.composer.text());
                dialog.move_next();
                let answer_text = dialog.current_answer_text();
                let placeholder = dialog.answer_placeholder();
                self.composer.set_text(answer_text);
                placeholder
            };
            self.composer.set_placeholder(placeholder);
            return Ok(());
        }

        if matches!(key.code, KeyCode::Enter)
            && !key.modifiers.contains(KeyModifiers::SHIFT)
            && !key.modifiers.contains(KeyModifiers::ALT)
        {
            let submission = self.composer.text().to_string();
            let (is_last, next_text, placeholder) = {
                let dialog = self.question_dialog.as_mut().expect("question dialog exists");
                dialog.set_current_answer_from_text(&submission);

                if dialog.is_last() {
                    (true, String::new(), dialog.answer_placeholder())
                } else {
                    dialog.move_next();
                    (
                        false,
                        dialog.current_answer_text(),
                        dialog.answer_placeholder(),
                    )
                }
            };

            if is_last {
                self.resolve_question_dialog(true, runtime)?;
            } else {
                self.composer.set_text(next_text);
                self.composer.set_placeholder(placeholder);
            }

            return Ok(());
        }

        let _ = self.composer.handle_key_with_history(key, false);
        if let Some(dialog) = self.question_dialog.as_ref() {
            self.composer.set_placeholder(dialog.answer_placeholder());
        }
        Ok(())
    }

    fn resolve_question_dialog(&mut self, allow: bool, runtime: &Runtime) -> Result<()> {
        let Some(dialog) = self.question_dialog.take() else {
            return Ok(());
        };

        if allow {
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

fn parse_answer_text(text: &str, question: &QuestionInfo) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }

    let allow_multiple = question.multiple.unwrap_or(false);
    let tokens = if allow_multiple {
        text.split(',').collect::<Vec<_>>()
    } else {
        vec![text]
    };

    let mut answers = Vec::new();
    for token in tokens {
        let value = token.trim().trim_matches('"').trim_matches('\'').trim();
        if value.is_empty() {
            continue;
        }

        let resolved = value
            .parse::<usize>()
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| question.options.get(index))
            .map(|option| option.label.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| value.to_string());

        if !answers.iter().any(|existing| existing == &resolved) {
            answers.push(resolved);
        }
    }

    answers
}
