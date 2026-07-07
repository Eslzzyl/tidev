use super::*;

use crate::App;
use tidev_search::current_at_fragment;
use tidev_types::prompts::init_command_with_args;

impl App {
    pub(crate) fn refresh_at_mention_state(&mut self) {
        if self.ui.command_palette.visible
            || self.ui.connect_dialog.is_some()
            || self.ui.theme_panel.is_some()
            || self.ui.model_panel.is_some()
            || self.ui.message_panel.is_some()
            || self.ui.session_panel.is_some()
            || self.ui.agents_panel.is_some()
            || self.ui.question_dialog.is_some()
        {
            self.ui.at_mention.clear();
            return;
        }

        let text = self.ui.composer.text();
        let cursor = self.ui.composer.cursor();
        self.ui
            .at_mention
            .sync(self.runtime.workspace_root().as_path(), text, cursor);
    }

    pub(crate) fn accept_at_mention(&mut self) {
        let text = self.ui.composer.text().to_string();
        let cursor = self.ui.composer.cursor();
        log::info!("accept_at_mention: text={:?}, cursor={}", text, cursor,);
        let Some((start, _query)) =
            current_at_fragment(self.ui.composer.text(), self.ui.composer.cursor())
        else {
            log::info!("accept_at_mention: current_at_fragment returned None, clearing");
            self.ui.at_mention.clear();
            return;
        };
        log::info!(
            "accept_at_mention: start={}, cursor={}",
            start,
            self.ui.composer.cursor(),
        );

        let Some(selection) = self.ui.at_mention.selected().cloned() else {
            log::info!("accept_at_mention: no selection, clearing");
            self.ui.at_mention.clear();
            return;
        };

        let replacement = match selection.kind {
            AtMentionKind::Directory => format!("@{}/", selection.path.trim_end_matches('/')),
            _ => format!("@{}", selection.path),
        };
        log::info!(
            "accept_at_mention: replacing range {}..{} with {:?}",
            start,
            self.ui.composer.cursor(),
            replacement,
        );
        self.ui
            .composer
            .replace_range(start, self.ui.composer.cursor(), &replacement);
        // Register an atomic inline span for the accepted @ reference
        let span_end = self.ui.composer.cursor();
        self.ui.composer.register_span(
            start,
            span_end,
            replacement,
            InlineSpanKind::AtReference,
            None,
        );
        self.ui.at_mention.clear();
        self.refresh_at_mention_state();
        self.refresh_snippet_state();
        self.ui
            .command_palette
            .sync(self.ui.composer.text(), &self.ui.commands);
    }

    pub(crate) fn refresh_snippet_state(&mut self) {
        // If snippets haven't been loaded yet, we need to load them first
        // to determine if they are available
        if self.ui.snippet_state.needs_load() {
            self.ui.snippet_state.load_snippets(
                self.runtime.workspace_root().as_path(),
                &self.runtime.paths().config_dir,
            );
        }

        // If no snippets available, skip entirely
        if !self.ui.snippet_state.is_enabled() {
            return;
        }

        if self.ui.command_palette.visible
            || self.ui.connect_dialog.is_some()
            || self.ui.theme_panel.is_some()
            || self.ui.model_panel.is_some()
            || self.ui.message_panel.is_some()
            || self.ui.session_panel.is_some()
            || self.ui.agents_panel.is_some()
            || self.ui.question_dialog.is_some()
            || self.ui.at_mention.visible
        {
            self.ui.snippet_state.clear();
            return;
        }

        let text = self.ui.composer.text();
        let cursor = self.ui.composer.cursor();
        self.ui.snippet_state.sync(
            self.runtime.workspace_root().as_path(),
            &self.runtime.paths().config_dir,
            text,
            cursor,
        );
    }

    pub(crate) fn accept_snippet(&mut self) {
        let Some(completion) = self.ui.snippet_state.apply_completion() else {
            self.ui.snippet_state.clear();
            return;
        };

        // Get current word range and replace it
        let cursor = self.ui.composer.cursor();
        let query = self.ui.snippet_state.query.clone();

        // Since query is the exact substring extracted going backwards from cursor,
        // its byte length exactly corresponds to the byte offset of the word start.
        let actual_start = cursor.saturating_sub(query.len());

        self.ui
            .composer
            .replace_range(actual_start, cursor, &completion);
        self.ui.snippet_state.clear();
        self.refresh_snippet_state();
        self.ui
            .command_palette
            .sync(self.ui.composer.text(), &self.ui.commands);
    }

    /// Execute a command line (starting with /).
    pub(crate) fn execute_command_line(&mut self, line: &str) -> Result<()> {
        let Some((name, args)) = self.ui.commands.parse_invocation(line) else {
            // Not a valid command format, treat as regular message
            return self.submit_prompt(line.to_string());
        };

        let Some(spec) = self.ui.commands.command(&name).cloned() else {
            // Unknown command, treat as regular message
            return self.submit_prompt(line.to_string());
        };

        self.run_command(spec.name, spec.action, &args)?;
        self.ui.commands.mark_used(spec.name);
        Ok(())
    }

    pub(crate) fn run_command(
        &mut self,
        _command_name: &str,
        action: CommandAction,
        args: &[String],
    ) -> Result<()> {
        if self.ui.pending_request {
            match action {
                CommandAction::Theme
                | CommandAction::Quit
                | CommandAction::Undo
                | CommandAction::Redo
                | CommandAction::Session
                | CommandAction::Clear
                | CommandAction::Connect
                | CommandAction::Model
                | CommandAction::Message
                | CommandAction::Rename
                | CommandAction::Settings
                | CommandAction::Init
                | CommandAction::Agents
                | CommandAction::Search
                | CommandAction::Skills => {}
                _ => {
                    self.ui.last_notice = Some(
                        "A response is still streaming. Wait for it to finish before changing sessions.".to_string(),
                    );
                    return Ok(());
                }
            }
        }

        match action {
            CommandAction::Connect => {
                if !args.is_empty() {
                    self.ui.last_notice = Some("Ignoring arguments to /connect".to_string());
                }
                self.open_connect_dialog()?;
            }
            CommandAction::Model => {
                self.open_model_panel(args.join(" "));
            }
            CommandAction::Search => {
                self.open_search_panel();
            }
            CommandAction::Session => {
                self.open_session_panel(args.join(" "))?;
            }
            CommandAction::Compact => {
                self.ui.active_request_id = self.ui.active_request_id.wrapping_add(1);
                let request_id = self.ui.active_request_id;
                let msg = tidev_types::message::Message::streaming(
                    tidev_types::message::MessageRole::System,
                    format!("{}\n\n", tidev_types::message::COMPACTION_MESSAGE_LABEL),
                );
                self.ui.chat_context.push(msg);

                self.schedule_context_compaction_for_session(
                    self.ui.chat_context.session_id,
                    Some(request_id),
                );
            }
            CommandAction::Message => {
                self.open_message_panel(args.join(" "))?;
            }
            CommandAction::Rename => {
                self.open_rename_session_dialog()?;
            }
            CommandAction::Clear => {
                self.start_new_session()?;
            }
            CommandAction::Undo => {
                self.undo_last_user_message()?;
            }
            CommandAction::Redo => {
                self.redo_last_user_message()?;
            }
            CommandAction::Theme => {
                self.apply_theme_command(args)?;
            }
            CommandAction::Settings => {
                self.open_settings_panel();
            }
            CommandAction::Quit => {
                self.ui.should_quit = true;
            }
            CommandAction::Init => {
                self.ui
                    .composer
                    .set_text(init_command_with_args(&args.join(" ")));
                self.ui.last_notice = Some("Init prompt loaded".to_string());
            }
            CommandAction::Agents => {
                self.ui.agents_panel = Some(ui::agents_panel::AgentsPanelState::new());
            }
            CommandAction::Skills => {
                self.open_skills_panel();
            }
            CommandAction::Memory => {
                // TODO: implement memory panel
            }
        }

        Ok(())
    }
}
