use super::*;

use crate::App;
use tidev_types::GoalStatus;

impl App {
    pub(crate) fn refresh_at_mention_state(&mut self) {
        if self.command_palette.visible
            || self.connect_dialog.is_some()
            || self.theme_panel.is_some()
            || self.model_panel.is_some()
            || self.message_panel.is_some()
            || self.session_panel.is_some()
            || self.mcp_panel.is_some()
            || self.agents_panel.is_some()
            || self.question_dialog.is_some()
        {
            self.at_mention.clear();
            return;
        }

        let text = self.composer.text();
        let cursor = self.composer.cursor();
        self.at_mention
            .sync(self.workspace_root.as_path(), text, cursor);
    }

    pub(crate) fn accept_at_mention(&mut self) {
        let Some((start, _query)) =
            current_at_fragment(self.composer.text(), self.composer.cursor())
        else {
            self.at_mention.clear();
            return;
        };

        let Some(selection) = self.at_mention.selected().cloned() else {
            self.at_mention.clear();
            return;
        };

        let replacement = match selection.kind {
            AtMentionKind::Directory => format!("@{}/", selection.path.trim_end_matches('/')),
            _ => format!("@{}", selection.path),
        };
        self.composer
            .replace_range(start, self.composer.cursor(), &replacement);
        self.at_mention.clear();
        self.refresh_at_mention_state();
        self.refresh_snippet_state();
        self.command_palette
            .sync(self.composer.text(), &self.commands);
    }

    pub(crate) fn refresh_snippet_state(&mut self) {
        // If snippets haven't been loaded yet, we need to load them first
        // to determine if they are available
        if self.snippet_state.needs_load() {
            self.snippet_state
                .load_snippets(self.workspace_root.as_path(), &self.paths.config_dir);
        }

        // If no snippets available, skip entirely
        if !self.snippet_state.is_enabled() {
            return;
        }

        if self.command_palette.visible
            || self.connect_dialog.is_some()
            || self.theme_panel.is_some()
            || self.model_panel.is_some()
            || self.message_panel.is_some()
            || self.session_panel.is_some()
            || self.mcp_panel.is_some()
            || self.agents_panel.is_some()
            || self.question_dialog.is_some()
            || self.at_mention.visible
        {
            self.snippet_state.clear();
            return;
        }

        let text = self.composer.text();
        let cursor = self.composer.cursor();
        self.snippet_state.sync(
            self.workspace_root.as_path(),
            &self.paths.config_dir,
            text,
            cursor,
        );
    }

    pub(crate) fn accept_snippet(&mut self) {
        let Some(completion) = self.snippet_state.apply_completion() else {
            self.snippet_state.clear();
            return;
        };

        // Get current word range and replace it
        let cursor = self.composer.cursor();
        let query = self.snippet_state.query.clone();

        // Since query is the exact substring extracted going backwards from cursor,
        // its byte length exactly corresponds to the byte offset of the word start.
        let actual_start = cursor.saturating_sub(query.len());

        self.composer
            .replace_range(actual_start, cursor, &completion);
        self.snippet_state.clear();
        self.refresh_snippet_state();
        self.command_palette
            .sync(self.composer.text(), &self.commands);
    }

    pub(crate) fn accept_shell_completion(&mut self) {
        let Some(completion) = self.shell_completion.accept() else {
            return;
        };

        // Replace the entire input text with the completed command
        let current = self.composer.text();
        // Extract the prefix (everything before the cursor, or the entire text)
        let cursor = self.composer.cursor();
        let word_start = current[..cursor]
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);

        self.composer.replace_range(word_start, cursor, &completion);
        self.command_palette
            .sync(self.composer.text(), &self.commands);
    }

    pub(crate) fn execute_command_line(&mut self, line: &str, runtime: &Runtime) -> Result<()> {
        let Some((name, args)) = self.commands.parse_invocation(line) else {
            // Not a valid command format, treat as regular message
            return self.submit_prompt(line.to_string(), runtime);
        };

        let Some(spec) = self.commands.command(&name).cloned() else {
            // Unknown command, treat as regular message
            return self.submit_prompt(line.to_string(), runtime);
        };

        self.run_command(spec.name, spec.action, &args, runtime)?;
        self.commands.mark_used(spec.name);
        Ok(())
    }

    pub(crate) fn run_command(
        &mut self,
        command_name: &str,
        action: CommandAction,
        args: &[String],
        runtime: &Runtime,
    ) -> Result<()> {
        if self.pending_request {
            match action {
                CommandAction::Theme
                | CommandAction::Quit
                | CommandAction::Undo
                | CommandAction::Redo
                | CommandAction::Session
                | CommandAction::Clear
                | CommandAction::Balance
                | CommandAction::Connect
                | CommandAction::Mcp
                | CommandAction::Model
                | CommandAction::Message
                | CommandAction::Rename
                | CommandAction::Settings
                | CommandAction::Stats
                | CommandAction::Init
                | CommandAction::Memory
                | CommandAction::Agents
                | CommandAction::Search
                | CommandAction::Skills
                | CommandAction::Sync
                | CommandAction::Goal => {}
                _ => {
                    self.last_notice = Some(
                        "A response is still streaming. Wait for it to finish before changing sessions.".to_string(),
                    );
                    return Ok(());
                }
            }
        }

        match action {
            CommandAction::Balance => {
                if !args.is_empty() {
                    self.last_notice = Some("Ignoring arguments to /balance".to_string());
                }
                self.open_balance_panel(runtime)?;
            }
            CommandAction::Connect => {
                if !args.is_empty() {
                    self.last_notice = Some("Ignoring arguments to /connect".to_string());
                }
                self.open_connect_dialog()?;
            }
            CommandAction::Mcp => match args.first().map(|value| value.as_str()) {
                Some("add") | Some("new") | Some("create") => {
                    self.open_mcp_panel(String::new());
                    self.open_new_mcp_server_editor(String::new());
                }
                Some("edit") => {
                    if let Some(server_name) = args.get(1) {
                        self.open_mcp_panel(server_name.clone());
                        self.open_existing_mcp_server_editor(String::new(), server_name.clone())?;
                    } else {
                        self.last_notice = Some("Usage: /mcp edit <server-name>".to_string());
                    }
                }
                Some("remove") | Some("delete") | Some("rm") => {
                    if let Some(server_name) = args.get(1) {
                        if let Err(error) = self.remove_mcp_server_from_editor(runtime, server_name)
                        {
                            self.last_notice = Some(error.to_string());
                        }
                    } else {
                        self.last_notice = Some("Usage: /mcp remove <server-name>".to_string());
                    }
                }
                _ => {
                    self.open_mcp_panel(args.join(" "));
                }
            },
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
                self.active_request_id = self.active_request_id.wrapping_add(1);
                let request_id = self.active_request_id;
                let msg = tidev_session::session::Message::streaming(
                    tidev_session::session::MessageRole::System,
                    format!("{}\n\n", tidev_session::session::COMPACTION_MESSAGE_LABEL),
                );
                self.conversation.push(msg);

                self.schedule_context_compaction_for_session(
                    self.conversation.session_id,
                    runtime,
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
                self.undo_last_user_message(runtime)?;
            }
            CommandAction::Redo => {
                self.redo_last_user_message(runtime)?;
            }
            CommandAction::Theme => {
                self.apply_theme_command(args)?;
            }
            CommandAction::Settings => {
                self.open_settings_panel();
            }
            CommandAction::Stats => {
                self.toggle_stats_panel();
            }
            CommandAction::Quit => {
                self.should_quit = true;
            }
            CommandAction::Init => {
                self.composer.set_text(init_command().to_string());
                self.last_notice = Some("Init prompt loaded".to_string());
            }
            CommandAction::Memory => {
                self.open_memory_panel()?;
            }
            CommandAction::Agents => {
                self.agents_panel = Some(ui::agents_panel::AgentsPanelState::new());
            }
            CommandAction::Skills => {
                self.open_skills_panel();
            }
            CommandAction::Sync => {
                self.open_sync_panel();
            }
            CommandAction::Goal => {
                let store = &self.store;
                let session_id = self.conversation.session_id;
                match command_name {
                    "goal-clear" => {
                        if self.screen == Screen::Welcome {
                            self.last_notice = Some("No goal to clear.".to_string());
                        } else {
                            store.clear_goal(session_id)?;
                            self.last_notice = Some("Goal cleared.".to_string());
                        }
                    }
                    "goal-pause" => {
                        if self.screen == Screen::Welcome {
                            self.last_notice = Some("No active goal.".to_string());
                        } else {
                            store.update_goal_status(session_id, GoalStatus::Paused)?;
                            self.last_notice = Some("Goal paused.".to_string());
                        }
                    }
                    "goal-resume" => {
                        if self.screen == Screen::Welcome {
                            self.last_notice = Some("No paused goal.".to_string());
                        } else {
                            store.update_goal_status(session_id, GoalStatus::Active)?;
                            self.last_notice = Some("Goal resumed.".to_string());
                        }
                    }
                    _ => {
                        // "goal" — show status or set
                        if args.is_empty() {
                            match store.get_goal(session_id)? {
                                Some(g) => {
                                    self.last_notice = Some(format!(
                                        "Goal: {}  [{}]  (tokens: {}, time: {}s)",
                                        g.objective,
                                        g.status.as_str(),
                                        g.tokens_used,
                                        g.time_used_seconds
                                    ));
                                }
                                None => {
                                    self.last_notice = Some(
                                        "No goal set. Use /goal <objective> to set one."
                                            .to_string(),
                                    );
                                }
                            }
                        } else {
                            // If on welcome screen, create the session first
                            self.ensure_goal_session()?;
                            let objective = args.join(" ");
                            self.store.set_goal(self.conversation.session_id, &objective)?;
                            self.last_notice = Some("Goal set.".to_string());
                        }
                    }
                }
            }
            CommandAction::Sandbox => {
                self.open_sandbox_panel();
            }
        }

        Ok(())
    }
}
