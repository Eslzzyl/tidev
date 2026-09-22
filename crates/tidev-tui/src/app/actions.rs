use super::*;

use crate::context::UpdateContext;
use crate::theme::resolve_palette;
use tidev_core::ApprovedTool;
use tidev_llm::message::{Message, MessageRole, ToolExecutionResult};

use crate::action::{
    Action, BoundaryDecision, ChatAction, ConnectAction, GitAction, GitQueryKind,
    InstructionsAction, McpAction, OverlayAction, OverlayKind, PanelLauncherAction, SearchAction,
    SensitiveFileDecision, SessionAction, SettingKey, SettingValue, SettingsAction, ThemeAction,
};
use crate::component::Component;

use crate::components::chat::MessageList;
use crate::components::selection::copy_to_clipboard;
use tidev_utils::session::title_from_prompt;

fn last_copyable_assistant_content(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|message| {
            message.role == MessageRole::Assistant
                && !message.streaming
                && !message.content.is_empty()
        })
        .map(|message| message.content.clone())
}

fn apply_setting_change(
    config: &mut tidev_config::AppConfig,
    key: SettingKey,
    value: SettingValue,
) -> bool {
    match (key, value) {
        (SettingKey::NotificationEnabled, SettingValue::Bool(value)) => {
            config.notifications.enabled = value;
            true
        }
        (SettingKey::LoggingEnabled, SettingValue::Bool(value)) => {
            config.logging.enabled = value;
            true
        }
        (SettingKey::LogLevel, SettingValue::Choice(value))
            if matches!(value.as_str(), "DEBUG" | "INFO" | "WARN" | "ERROR") =>
        {
            config.logging.level = value;
            true
        }
        (SettingKey::SaveRequestBody, SettingValue::Bool(value)) => {
            config.logging.save_request_body = value;
            true
        }
        (SettingKey::SaveResponseBody, SettingValue::Bool(value)) => {
            config.logging.save_response_body = value;
            true
        }
        (SettingKey::ScrollSpeed, SettingValue::Number(value)) if (1.0..=10.0).contains(&value) => {
            config.ui.scroll_speed = value;
            true
        }
        (SettingKey::AllowSensitiveFileAccess, SettingValue::Bool(value)) => {
            config.access_control.allow_sensitive_file_access = value;
            true
        }
        (SettingKey::AllowOutsideWorkspaceAccess, SettingValue::Bool(value)) => {
            config.access_control.allow_outside_workspace_access = value;
            true
        }
        (SettingKey::SubagentEnabled, SettingValue::Bool(value)) => {
            config.subagent.enabled = value;
            true
        }
        (SettingKey::CollapseThinking, SettingValue::Bool(value)) => {
            config.ui.collapse_thinking = value;
            true
        }
        (SettingKey::CollapseDiffs, SettingValue::Bool(value)) => {
            config.ui.collapse_diffs = value;
            true
        }
        (SettingKey::SendWhileBusy, SettingValue::Choice(value)) => match value.as_str() {
            "queue" => {
                config.ui.send_while_busy = tidev_config::SendWhileBusy::Queue;
                true
            }
            "steer" => {
                config.ui.send_while_busy = tidev_config::SendWhileBusy::Steer;
                true
            }
            _ => false,
        },
        (SettingKey::FastMode, SettingValue::Bool(value)) => {
            config.ui.fast_mode = value;
            true
        }
        (SettingKey::RightSidebarVisible, SettingValue::Bool(value)) => {
            config.ui.right_sidebar_visible = value;
            true
        }
        (SettingKey::Language, SettingValue::Choice(value))
            if matches!(value.as_str(), "system" | "en-US" | "zh-CN") =>
        {
            config.ui.locale = value;
            true
        }
        _ => false,
    }
}

impl App {
    pub(crate) fn process_action(&mut self, action: Action) {
        let mut queue = vec![action];
        while let Some(action) = queue.pop() {
            match action {
                Action::Quit => {
                    self.should_quit = true;
                }
                Action::CopyLastAssistant => {
                    self.copy_last_assistant_message();
                }
                Action::ImageAttachmentUnsupported => {
                    self.set_notice(self.ui_text().text(TextKey::ImageAttachmentUnsupported));
                }
                Action::ToggleRightSidebar(visible) => {
                    let target = visible.unwrap_or(!self.runtime.config().ui.right_sidebar_visible);
                    let key = SettingKey::RightSidebarVisible;
                    let value = SettingValue::Bool(target);
                    // Reuse Settings handling for persistence and overlay sync.
                    // Push Notice first so Settings is popped and persisted before the notice is shown.
                    let ui_text = self.ui_text();
                    queue.push(Action::Notice(if target {
                        ui_text.text(TextKey::RightSidebarShown)
                    } else {
                        ui_text.text(TextKey::RightSidebarHidden)
                    }));
                    queue.push(Action::Settings(SettingsAction::Change { key, value }));
                }
                Action::ToggleFastMode => {
                    if !self.runtime.active_model().is_gpt() {
                        continue;
                    }
                    let key = SettingKey::FastMode;
                    let value = SettingValue::Bool(!self.runtime.config().ui.fast_mode);
                    queue.push(Action::Settings(SettingsAction::Change { key, value }));
                }
                Action::Overlay(OverlayAction::Open(kind)) => {
                    self.open_overlay(kind);
                }
                Action::Overlay(OverlayAction::Close(kind)) => {
                    let is_model_panel = kind == OverlayKind::ModelPanel;
                    self.close_overlay(kind, &mut queue);
                    if is_model_panel {
                        self.thinking_level = self.runtime.active_model().thinking_level.clone();
                        if let Some(ref mut composer) = self.composer {
                            let model = self.runtime.active_model();
                            composer.set_model_supports_images(model.supports_images);
                        }
                    }
                }
                Action::PanelLauncher(PanelLauncherAction::Select(panel)) => {
                    let kind = panel.overlay_kind();
                    self.close_overlay(OverlayKind::PanelLauncher, &mut queue);
                    queue.push(Action::Overlay(OverlayAction::Open(kind)));
                }
                Action::Settings(SettingsAction::Change { key, value }) => {
                    let change = SettingsAction::Change {
                        key,
                        value: value.clone(),
                    };
                    let previous = self.runtime.config();
                    let valid = {
                        let mut changed = false;
                        self.runtime.update_config(|config| {
                            changed = apply_setting_change(config, key, value.clone());
                        });
                        changed
                    };

                    if !valid {
                        self.set_notice(self.ui_text().text(TextKey::InvalidSettingsValue));
                        continue;
                    }

                    if let Err(error) = self.runtime.save_config() {
                        self.runtime.update_config(|config| *config = previous);
                        self.set_notice(self.ui_text().text_with_value(
                            TextKey::FailedSaveSettings,
                            "error",
                            &error.to_string(),
                        ));
                        continue;
                    }

                    let config = self.runtime.config();
                    self.desktop_notifications
                        .apply_config(&config.notifications);

                    let ctx = UpdateContext {
                        runtime: &mut self.runtime,
                    };
                    queue.extend(self.overlays.update_all(&Action::Settings(change), &ctx));
                }
                Action::Theme(ThemeAction::Set(name)) => {
                    self.current_palette = resolve_palette(&self.theme_catalog, &name);
                    self.runtime.update_config(|cfg| cfg.set_theme(&name));
                    let _ = self.runtime.save_config();
                    if let Some(ref mut chat) = self.message_list {
                        chat.invalidate_theme();
                    }
                }
                Action::Search(SearchAction::SwitchProvider(provider)) => {
                    self.runtime
                        .update_config(|cfg| cfg.websearch.default_provider = provider);
                    let _ = self.runtime.save_config();
                }
                Action::Search(SearchAction::SaveApiKey {
                    provider,
                    key,
                    is_cx,
                }) => {
                    self.runtime.update_auth(|auth| {
                        if is_cx {
                            auth.web.google_cx = Some(key);
                        } else {
                            auth.web.search_api_keys.insert(provider, key);
                        }
                    });
                    let _ = self.runtime.save_auth();
                }
                Action::Connect(ConnectAction::SaveApiKey { provider_id, key }) => {
                    if key.trim().is_empty() {
                        self.set_notice(self.ui_text().text(TextKey::ApiKeyEmpty));
                        return;
                    }

                    self.runtime
                        .update_auth(|auth| auth.set_api_key(&provider_id, &key));
                    let _ = self.runtime.save_auth();

                    // Resolve the provider's default model and switch to it
                    match self
                        .runtime
                        .config()
                        .resolve_provider_default_model(&self.runtime.auth(), &provider_id)
                    {
                        Ok(model) => {
                            // Keep the persisted default in sync with the runtime model.
                            self.runtime.update_config(|cfg| {
                                cfg.default_provider = model.provider_id.clone();
                                cfg.default_model = model.model_id.clone();
                            });
                            let _ = self.runtime.save_config();
                            self.runtime.set_active_model(model.clone());

                            // Update composer's image support flag.
                            if let Some(ref mut composer) = self.composer {
                                composer.set_model_supports_images(model.supports_images);
                            }

                            // Persist model to current session if one is active
                            if let Some(session_id) = self.current_session_id
                                && self
                                    .runtime
                                    .session_manager()
                                    .store()
                                    .load_session_record(session_id)
                                    .ok()
                                    .flatten()
                                    .is_some()
                            {
                                let _ = self.runtime.session_manager().update_session_model(
                                    session_id,
                                    &model.provider_id,
                                    &model.provider_display_name,
                                    &model.model_id,
                                    &model.display_name,
                                );
                            }

                            let ui_text = self.ui_text();
                            self.set_notice(ui_text.text_with_value(
                                TextKey::ConnectedTo,
                                "name",
                                &model.provider_display_name,
                            ));
                        }
                        Err(e) => {
                            self.set_notice(self.ui_text().text_with_value(
                                TextKey::ConnectedResolveModelFailed,
                                "error",
                                &e.to_string(),
                            ));
                        }
                    }
                }
                Action::Connect(ConnectAction::CopyApiKey { provider_id }) => {
                    let display_name = self
                        .runtime
                        .config()
                        .provider_display_name(&provider_id)
                        .unwrap_or(&provider_id)
                        .to_string();
                    let key = self.runtime.auth().api_key(&provider_id).map(str::to_owned);

                    let Some(key) = key else {
                        let ui_text = self.ui_text();
                        self.set_toast(
                            ui_text.text_with_value(
                                TextKey::NoApiKeyConfigured,
                                "name",
                                &display_name,
                            ),
                            std::time::Duration::from_secs(3),
                        );
                        continue;
                    };

                    match copy_to_clipboard(&key) {
                        Ok(()) => self.set_toast(
                            self.ui_text().text_with_value(
                                TextKey::ApiKeyCopied,
                                "name",
                                &display_name,
                            ),
                            std::time::Duration::from_secs(3),
                        ),
                        Err(error) => self.set_toast(
                            self.ui_text().text_with_value(
                                TextKey::CopyFailed,
                                "error",
                                &error.to_string(),
                            ),
                            std::time::Duration::from_secs(5),
                        ),
                    }
                }
                Action::Connect(ConnectAction::Disconnect {
                    provider_id,
                    display_name,
                }) => {
                    let mut removed = false;
                    self.runtime.update_auth(|auth| {
                        removed = auth.remove_api_key(&provider_id);
                    });
                    if removed {
                        let _ = self.runtime.save_auth();
                        self.set_notice(self.ui_text().text_with_value(
                            TextKey::DisconnectedFrom,
                            "name",
                            &display_name,
                        ));
                    }
                }
                Action::Connect(ConnectAction::PruneOrphans) => {
                    let known_ids = self.runtime.config().provider_ids();
                    let mut pruned = 0usize;
                    self.runtime.update_auth(|auth| {
                        pruned = auth.prune_orphan_providers(&known_ids);
                    });
                    if pruned > 0 {
                        let _ = self.runtime.save_auth();
                        self.set_notice(self.ui_text().text_with_value(
                            TextKey::PrunedAuthEntries,
                            "count",
                            &pruned.to_string(),
                        ));
                    } else {
                        self.set_notice(self.ui_text().text(TextKey::NoOrphanAuthEntries));
                    }
                }
                Action::Mcp(action) => {
                    self.handle_mcp_action(action);
                }
                Action::Instructions(action) => match action {
                    InstructionsAction::Reload => {
                        let ctx = UpdateContext {
                            runtime: &mut self.runtime,
                        };
                        queue.extend(
                            self.overlays.update_all(
                                &Action::Instructions(InstructionsAction::Reload),
                                &ctx,
                            ),
                        );
                    }
                    InstructionsAction::Edit => {
                        let content = match self.runtime.load_global_instructions() {
                            Ok(content) => content.unwrap_or_default(),
                            Err(error) => {
                                self.set_notice(self.ui_text().text_with_value(
                                    TextKey::GlobalInstructionsError,
                                    "error",
                                    &error.to_string(),
                                ));
                                continue;
                            }
                        };
                        let ui_config = self.runtime.config().ui;
                        match crate::editor::open_external_editor(&content, &ui_config) {
                            Ok(edited) => match self.runtime.save_global_instructions(&edited) {
                                Ok(()) => {
                                    self.set_notice(
                                        self.ui_text().text(TextKey::GlobalInstructionsSaved),
                                    );
                                    queue.push(Action::Instructions(InstructionsAction::Reload));
                                }
                                Err(error) => {
                                    self.set_notice(self.ui_text().text_with_value(
                                        TextKey::GlobalInstructionsError,
                                        "error",
                                        &error.to_string(),
                                    ));
                                }
                            },
                            Err(error) => {
                                self.set_notice(self.ui_text().text_with_value(
                                    TextKey::EditorError,
                                    "error",
                                    &error.to_string(),
                                ));
                            }
                        }
                    }
                    InstructionsAction::Delete => match self.runtime.delete_global_instructions() {
                        Ok(()) => {
                            self.set_notice(
                                self.ui_text().text(TextKey::GlobalInstructionsDeleted),
                            );
                            queue.push(Action::Instructions(InstructionsAction::Reload));
                        }
                        Err(error) => {
                            self.set_notice(self.ui_text().text_with_value(
                                TextKey::GlobalInstructionsError,
                                "error",
                                &error.to_string(),
                            ));
                        }
                    },
                },
                Action::Git(action) => match action {
                    GitAction::Refresh => {
                        let request_id = self.spawn_git_status();
                        let loading = Self::git_loading_action(request_id, GitQueryKind::Status);
                        let ctx = UpdateContext {
                            runtime: &mut self.runtime,
                        };
                        queue.extend(self.overlays.update_all(&loading, &ctx));
                    }
                    GitAction::LoadHistory { head, skip } => {
                        let request = Action::Git(GitAction::LoadHistory {
                            head: head.clone(),
                            skip,
                        });
                        let ctx = UpdateContext {
                            runtime: &mut self.runtime,
                        };
                        queue.extend(self.overlays.update_all(&request, &ctx));
                        let request_id = self.spawn_git_history(head, skip);
                        let loading = Self::git_loading_action(request_id, GitQueryKind::History);
                        let ctx = UpdateContext {
                            runtime: &mut self.runtime,
                        };
                        queue.extend(self.overlays.update_all(&loading, &ctx));
                    }
                    GitAction::LoadDiff { scope } => {
                        let request = Action::Git(GitAction::LoadDiff {
                            scope: scope.clone(),
                        });
                        let ctx = UpdateContext {
                            runtime: &mut self.runtime,
                        };
                        queue.extend(self.overlays.update_all(&request, &ctx));
                        let request_id = self.spawn_git_diff(scope);
                        let loading = Self::git_loading_action(request_id, GitQueryKind::Diff);
                        let ctx = UpdateContext {
                            runtime: &mut self.runtime,
                        };
                        queue.extend(self.overlays.update_all(&loading, &ctx));
                    }
                    action => {
                        let ctx = UpdateContext {
                            runtime: &mut self.runtime,
                        };
                        queue.extend(self.overlays.update_all(&Action::Git(action), &ctx));
                    }
                },
                Action::Session(SessionAction::Select(session_id)) => {
                    // Ignore if already on this session
                    if self.current_session_id == Some(session_id) {
                        return;
                    }

                    // Cache current session's context_usage before switching away.
                    if let Some(current_id) = self.current_session_id {
                        if let Some(usage) = &self.context_usage {
                            self.context_usage_cache.insert(current_id, usage.clone());
                        }
                        // Save composer text for the session we're leaving.
                        if let Some(ref composer) = self.composer {
                            let text = composer.text().to_string();
                            self.composer_texts.insert(current_id, text);
                        }
                    }

                    // Restore composer text for the session we're switching to.
                    if let Some(ref mut composer) = self.composer {
                        if let Some(saved) = self.composer_texts.remove(&session_id) {
                            composer.set_text(saved);
                        } else {
                            composer.clear();
                        }
                    }

                    // Fast path: if the MessageList already has a chat_context for
                    // this session, use switch_to_session to preserve in-memory
                    // streaming state (avoiding DB reload that would lose content).
                    if let Some(ref mut chat) = self.message_list
                        && chat.switch_to_session(session_id)
                    {
                        self.current_session_id = Some(session_id);
                        self.scroll_target = None;
                        self.screen = AppScreen::Chat;

                        // Restore cached context_usage for the target session.
                        self.context_usage = self.context_usage_cache.remove(&session_id);

                        // Clear stale interaction state on session switch.
                        self.mouse_selection.clear();
                        self.abort_confirmation_deadline = None;

                        // Reload todos for the target session.
                        if let Ok(todos) = self
                            .runtime
                            .session_manager()
                            .store()
                            .load_todos(session_id)
                        {
                            self.todos = todos;
                        }

                        // Restore instruction sources so that InstructionsLoaded
                        // events emitted on loop restart are de-duplicated.
                        if let Ok(sources) = self
                            .runtime
                            .session_manager()
                            .store()
                            .load_instruction_sources(session_id)
                        {
                            self.shown_instruction_sources = sources;
                        }

                        // Refresh the Runtime's in-memory message buffer.
                        // Use the already-cached messages to avoid a redundant DB read.
                        if let Some(ctx) = chat.active_chat_context() {
                            let buf_messages = ctx.session_messages();
                            let rt = self.runtime.clone();
                            tokio::spawn(async move {
                                rt.set_session_message_buffer(session_id, buf_messages)
                                    .await;
                            });
                        }

                        // Switch the runtime's active model to match this
                        // session's model and restore its latest thinking level.
                        let session_thinking_level = chat.active_chat_context().and_then(|ctx| {
                            ctx.messages
                                .iter()
                                .rev()
                                .find(|m| m.role == MessageRole::User && !m.is_compaction())
                                .and_then(|m| m.thinking_level.clone())
                        });
                        self.sync_active_model_for_session(session_id, session_thinking_level);
                        // Resolve the target session's mode instead of reusing
                        // the previous session's global mode.
                        self.sync_mode_for_session(session_id);

                        log::info!("Switching to session: existing context (fast path)");

                        // Close the session panel overlay.
                        queue.push(Action::Overlay(OverlayAction::Close(
                            OverlayKind::SessionPanel,
                        )));
                        return;
                    }

                    // Slow path: first time entering this session — load from DB.
                    self.current_session_id = Some(session_id);
                    self.scroll_target = None;
                    self.screen = AppScreen::Chat;

                    // Load session record and messages for chat display
                    let session_messages = self
                        .runtime
                        .session_manager()
                        .load_session_messages(session_id)
                        .unwrap_or_default();
                    let messages: Vec<_> = session_messages
                        .iter()
                        .map(|message| message.message.clone())
                        .collect();

                    let session_thinking_level = messages
                        .iter()
                        .rev()
                        .find(|m| m.role == MessageRole::User && !m.is_compaction())
                        .and_then(|m| m.thinking_level.clone());

                    // Compute context_usage from stored messages (last assistant
                    // message holds cumulative token counts).
                    self.context_usage = messages
                        .iter()
                        .rev()
                        .find(|m| m.role == MessageRole::Assistant)
                        .and_then(|m| {
                            m.input_tokens.map(|input| ContextUsage {
                                input_tokens: input,
                                output_tokens: m.output_tokens.unwrap_or(0),
                                tokens_per_second: m.tokens_per_second,
                            })
                        });
                    // Cache it for fast-path restoration on subsequent switches.
                    if let Some(usage) = &self.context_usage {
                        self.context_usage_cache.insert(session_id, usage.clone());
                    }

                    // Refresh the Runtime's in-memory message buffer so the
                    // next submit_prompt picks up the latest data from the store.
                    // Use the already-loaded messages to avoid a redundant DB read.
                    let rt = self.runtime.clone();
                    let sid = session_id;
                    let buf_messages = session_messages.clone();
                    tokio::spawn(async move {
                        rt.set_session_message_buffer(sid, buf_messages).await;
                    });

                    let chat_context = {
                        let mut ctx = crate::chat_context::ChatContext::from_session_messages(
                            session_id,
                            String::new(),
                            session_messages,
                            None,
                            String::new(),
                            String::new(),
                        );
                        if let Ok(Some(record)) =
                            self.runtime.session_manager().load_session(session_id)
                        {
                            ctx.title = record.title;
                            ctx.parent_session_id = record.parent_session_id;
                            ctx.model_display_name = record.model_display_name;
                            ctx.provider_display_name = record.provider_display_name;
                        }

                        ctx
                    };

                    // Switch the runtime's active model to match this session
                    // and restore its latest thinking level.
                    self.sync_active_model_for_session(session_id, session_thinking_level);

                    let session_title = chat_context.title.clone();

                    // Create or update MessageList
                    self.message_list
                        .get_or_insert_with(MessageList::new)
                        .set_chat_context(chat_context);
                    self.sync_mode_for_session(session_id);

                    // Reload todos for the target session.
                    if let Ok(todos) = self
                        .runtime
                        .session_manager()
                        .store()
                        .load_todos(session_id)
                    {
                        self.todos = todos;
                    }

                    // Restore instruction sources for dedup on loop restart.
                    if let Ok(sources) = self
                        .runtime
                        .session_manager()
                        .store()
                        .load_instruction_sources(session_id)
                    {
                        self.shown_instruction_sources = sources;
                    }

                    log::info!("Switching to session: {} ({})", session_title, session_id);

                    // Close the session panel overlay (mirrors old Enter → select + close).
                    queue.push(Action::Overlay(OverlayAction::Close(
                        OverlayKind::SessionPanel,
                    )));
                }
                Action::Session(SessionAction::Reload) => {
                    // Broadcast to overlays so SessionPanel reloads its list.
                    let ctx = UpdateContext {
                        runtime: &mut self.runtime,
                    };
                    queue.extend(self.overlays.update_all(&action, &ctx));
                }
                Action::Session(SessionAction::Fork(message_id)) => {
                    let session_id = match self.current_session_id {
                        Some(id) => id,
                        None => return,
                    };

                    // Count the retained messages for the confirmation notice.
                    let messages = match self.runtime.session_manager().load_messages(session_id) {
                        Ok(msgs) => msgs,
                        Err(e) => {
                            log::error!("Failed to load messages for fork: {e}");
                            return;
                        }
                    };

                    let message_index = match messages.iter().position(|m| m.id == message_id) {
                        Some(idx) => idx,
                        None => {
                            log::warn!("Fork target message not found: {}", message_id);
                            return;
                        }
                    };

                    let new_session_id =
                        match self.runtime.fork_session(session_id, message_id, None) {
                            Ok(id) => id,
                            Err(e) => {
                                log::error!("Failed to fork session: {e}");
                                return;
                            }
                        };

                    // Switch to the new session
                    self.current_session_id = Some(new_session_id);
                    self.shown_instruction_sources.clear();
                    self.scroll_target = None;

                    self.set_notice(self.ui_text().text_with_value(
                        TextKey::ForkedSession,
                        "count",
                        &(message_index + 1).to_string(),
                    ));

                    log::info!(
                        "Forked session {} -> {} with {} messages",
                        session_id,
                        new_session_id,
                        message_index + 1,
                    );
                }
                Action::Session(SessionAction::Undo) => {
                    let session_id = match self.current_session_id {
                        Some(id) => id,
                        None => return,
                    };
                    self.set_notice(self.ui_text().text(TextKey::UndoInProgress));
                    let rt = self.runtime.clone();
                    tokio::spawn(async move {
                        // Cancel this session's running loop first.
                        rt.cancel_session(session_id).await;
                        if let Err(e) = rt.undo(session_id).await {
                            log::error!("Undo failed: {e}");
                        }
                    });
                }
                Action::Session(SessionAction::Redo) => {
                    let session_id = match self.current_session_id {
                        Some(id) => id,
                        None => return,
                    };
                    self.set_notice(self.ui_text().text(TextKey::RedoInProgress));
                    let rt = self.runtime.clone();
                    tokio::spawn(async move {
                        rt.cancel_session(session_id).await;
                        if let Err(e) = rt.redo(session_id).await {
                            log::error!("Redo failed: {e}");
                        }
                    });
                }
                Action::Session(SessionAction::Compact) => {
                    let Some(sid) = self.current_session_id else {
                        return;
                    };
                    // If a request is in progress, queue the compact.
                    if self.has_active_request() {
                        self.pending_compacts.insert(sid);
                        self.set_notice(self.ui_text().text(TextKey::CompactionQueued));
                        return;
                    }
                    self.execute_compact();
                }
                Action::Session(SessionAction::Rename(session_id, title)) => {
                    let final_title = if title.trim().is_empty() {
                        self.ui_text().text(TextKey::UntitledSession)
                    } else {
                        title.trim().to_string()
                    };
                    match self.runtime.update_session_title(session_id, &final_title) {
                        Ok(_) => {
                            self.set_notice(self.ui_text().text(TextKey::SessionTitleUpdated));
                            log::info!("Renamed session {} to {}", session_id, final_title);
                        }
                        Err(e) => log::error!("Failed to rename session: {e}"),
                    }
                }
                Action::Session(SessionAction::CycleThinkingLevel) => {
                    let next = self.thinking_level.next();
                    self.thinking_level = next.clone();
                    let model = self.runtime.active_model();
                    let _ = self.runtime.set_model_thinking_level(
                        &model.provider_id,
                        &model.model_id,
                        &next.to_string(),
                    );
                    if next.is_supported() {
                        self.set_notice(self.ui_text().text_with_value(
                            TextKey::ThinkingLevel,
                            "level",
                            &crate::i18n::thinking_level_name(&self.ui_text(), next.display_name()),
                        ));
                    } else {
                        self.set_notice(self.ui_text().text(TextKey::ThinkingOff));
                    }
                }
                Action::Session(SessionAction::Create) => {
                    self.reset_to_welcome();
                    self.pending_approvals.clear();
                    self.active_approval_session = None;
                    self.pending_inputs.clear();
                    self.pending_compacts.clear();
                    self.compacting_sessions.clear();
                }
                Action::Session(SessionAction::CurrentSessionDeleted) => {
                    let Some(sid) = self.current_session_id else {
                        return;
                    };
                    // Drop only the state the deleted session would consume,
                    // leaving other sessions' pending state intact.
                    self.composer_texts.remove(&sid);
                    self.context_usage_cache.remove(&sid);
                    self.pending_inputs.retain(|input| input.session_id != sid);
                    self.pending_compacts.remove(&sid);
                    self.compacting_sessions.remove(&sid);
                    self.session_modes.remove(&sid);
                    self.pending_modes.remove(&sid);
                    self.pending_approvals.remove(&sid);
                    if self.active_approval_session == Some(sid) {
                        self.active_approval_session = None;
                    }
                    self.reset_to_welcome();
                }
                Action::Chat(action) => {
                    match &action {
                        ChatAction::SendMessage { text, attachments } => {
                            let text = text.clone();
                            let attachments = attachments.clone();

                            // Check if this is a /command.
                            if let Some((name, args)) =
                            crate::components::composer::command_palette::CommandRegistry::new()
                                .parse_invocation(&text)
                            && let Some(spec) =
                                crate::components::composer::command_palette::CommandRegistry::new()
                                    .command(&name)
                            {
                                let actions =
                                    crate::components::composer::command_palette::execute_command(
                                        spec.action,
                                        &args,
                                        &self.theme_catalog,
                                    );
                                for action in actions {
                                    self.process_action(action);
                                }
                                return;
                            }
                            // Unknown command — fall through to submit as prompt.

                            // Extract @-reference paths from the text (matching old
                            // `inline_file_references` behaviour).
                            let ref_paths = extract_inline_refs(&text);

                            // Also collect paths from any inline spans (the composer
                            // puts accepted @mention paths into the attachments field as
                            // a placeholder — handled below).
                            let workspace_root = self.runtime.workspace_root().clone();
                            let mut final_attachments = tidev_core::attachment::build_attachments(
                                &workspace_root,
                                &ref_paths,
                            );

                            // Append any already-built attachments (images, files from
                            // composer spans).
                            final_attachments.extend(attachments);

                            // The runtime decides delivery: when the session's
                            // agent loop is busy, the `send_while_busy` config
                            // determines whether the message is queued (next
                            // turn) or steered (next request boundary). No
                            // TUI-side queue is involved.

                            // If no active session, create one and enter Chat mode.
                            let session_id = self.current_session_id;
                            let sid = match session_id {
                                Some(id) => id,
                                None => {
                                    let untitled_title =
                                        self.ui_text().text(TextKey::UntitledSession);
                                    match self.runtime.create_default_session(&untitled_title) {
                                        Ok(id) => {
                                            self.current_session_id = Some(id);
                                            self.shown_instruction_sources.clear();

                                            // Initialize MessageList for the new session.
                                            let active_model =
                                                self.runtime.resolve_active_model().ok();
                                            let model_display = active_model
                                                .as_ref()
                                                .map(|m| m.display_name.clone())
                                                .unwrap_or_default();
                                            let provider_display = active_model
                                                .as_ref()
                                                .map(|m| m.provider_display_name.clone())
                                                .unwrap_or_default();
                                            let chat_context =
                                                crate::chat_context::ChatContext::new(
                                                    id,
                                                    String::new(),
                                                    Vec::new(),
                                                    None,
                                                    model_display,
                                                    provider_display,
                                                );
                                            self.message_list
                                                .get_or_insert_with(MessageList::new)
                                                .set_chat_context(chat_context);
                                            self.screen = AppScreen::Chat;

                                            id
                                        }
                                        Err(e) => {
                                            log::error!("Failed to create session: {e}");
                                            self.set_notice(
                                                self.ui_text().text(TextKey::FailedCreateSession),
                                            );
                                            return;
                                        }
                                    }
                                }
                            };

                            // Spawn submission to avoid blocking the UI.
                            let mode = self.mode;
                            self.session_modes.insert(sid, mode);
                            let thinking_level = self.runtime.active_model().thinking_level.clone();
                            self.thinking_level = thinking_level.clone();
                            let rt = self.runtime.clone();
                            let text_for_title = text.clone();
                            self.set_notice(self.ui_text().text(TextKey::Sending));
                            if let Some(ref mut chat) = self.message_list {
                                chat.follow_tail = true;
                            }
                            tokio::spawn(async move {
                                if let Err(error) = rt
                                    .submit_prompt_with_attachments(
                                        sid,
                                        mode,
                                        text,
                                        final_attachments,
                                        Some(thinking_level),
                                    )
                                    .await
                                {
                                    log::error!("submit_prompt failed: {error}");
                                }
                            });

                            // Update session title from prompt (matching old behaviour).
                            let untitled_title = self.ui_text().text(TextKey::UntitledSession);
                            if let Some(ref mut chat) = self.message_list
                                && let Some(ref mut ctx) = chat.active_chat_context_mut()
                                && (ctx.title.is_empty() || ctx.title == untitled_title)
                            {
                                let title = title_from_prompt(&text_for_title);
                                ctx.title = title.clone();
                                if let Err(e) = self.runtime.update_session_title(sid, &title) {
                                    log::error!("Failed to update session title: {e}");
                                }
                            }
                        }
                        ChatAction::SetInput(text) => {
                            if let Some(ref mut composer) = self.composer {
                                composer.set_text(text.clone());
                            }
                        }
                        ChatAction::ExpandAllThinking | ChatAction::CollapseAllThinking => {
                            // The welcome page has no session to operate on: no-op,
                            // matching Undo/Redo/Compact behaviour in that state.
                            if self.screen != AppScreen::Welcome
                                && let Some(ref mut chat) = self.message_list
                            {
                                let ctx = UpdateContext {
                                    runtime: &mut self.runtime,
                                };
                                queue.extend(chat.update(&Action::Chat(action), &ctx));
                            }
                        }
                        _ => {
                            // Forward other chat actions (scroll, stream, etc.) to MessageList.
                            if let Some(ref mut chat) = self.message_list {
                                let ctx = UpdateContext {
                                    runtime: &mut self.runtime,
                                };
                                queue.extend(chat.update(&Action::Chat(action), &ctx));
                            }
                        }
                    }
                }
                Action::Notice(msg) => {
                    self.set_notice(msg);
                }
                Action::CommandUsage(command) => {
                    self.set_notice(self.ui_text().text_with_value(
                        TextKey::UsageCommand,
                        "command",
                        command,
                    ));
                }
                Action::ClipboardError(error) => {
                    self.set_notice(self.ui_text().text_with_value(
                        TextKey::ClipboardError,
                        "error",
                        &error,
                    ));
                }
                Action::Noop => {}
                Action::Consumed => {}
                // ── Tool approval pipeline ──
                Action::WorkspaceBoundaryResponse {
                    path,
                    decision,
                    reason,
                } => {
                    self.record_boundary_decision(&path, &decision);

                    let allowed = matches!(
                        decision,
                        BoundaryDecision::AllowOnce | BoundaryDecision::AllowUntilExit
                    );
                    let path_str = path.to_string_lossy().to_string();

                    // Record in cache regardless of which session.
                    self.boundary_permissions.insert(path_str.clone(), allowed);

                    if let Some(r) = reason
                        && !r.is_empty()
                    {
                        self.boundary_reasons.insert(path_str, r);
                    }

                    self.process_next_tool();
                }
                Action::SensitiveFileResponse {
                    path,
                    decision,
                    reason,
                } => {
                    self.record_sensitive_decision(&path, &decision);

                    let allowed = matches!(
                        decision,
                        SensitiveFileDecision::AllowOnce | SensitiveFileDecision::AllowUntilExit
                    );
                    let path_str = path.to_string_lossy().to_string();

                    self.sensitive_permissions.insert(path_str.clone(), allowed);

                    if let Some(r) = reason
                        && !r.is_empty()
                    {
                        self.sensitive_reasons.insert(path_str, r);
                    }

                    self.process_next_tool();
                }
                Action::QuestionResponse { output } => {
                    let Some(session_id) = self.active_approval_session else {
                        break;
                    };
                    let Some(approval) = self.pending_approvals.get_mut(&session_id) else {
                        break;
                    };
                    if approval.tool_index >= approval.tools.len() {
                        break;
                    }
                    let twv = &approval.tools[approval.tool_index];

                    let result = match output {
                        Some(answers) => ToolExecutionResult::new(answers),
                        None => ToolExecutionResult::new("Tool 'question' was dismissed by user"),
                    };

                    approval.approved_tools.push(ApprovedTool {
                        tool_call: twv.tool_call.clone(),
                        rejection: Some(result),
                        child_session_id: None,
                        allow_outside: false,
                        sensitive_file_approved: false,
                        user_reason: None,
                    });

                    approval.tool_index += 1;
                    self.process_next_tool();
                }
            }
        }
    }

    /// Reset the app to the welcome screen with a fresh empty chat context.
    ///
    /// Shared by `SessionAction::Create` (which additionally clears every
    /// session's pending state) and `SessionAction::CurrentSessionDeleted`
    /// (which drops only the deleted session's state before calling this).
    fn reset_to_welcome(&mut self) {
        self.current_session_id = None;

        let active_model = self.runtime.resolve_active_model().ok();

        // Restore the runtime's active model to the default so
        // the composer/header no longer shows the previous
        // session's model.
        if let Some(ref model) = active_model {
            self.runtime.set_active_model(model.clone());
            if let Some(ref mut composer) = self.composer {
                composer.set_model_supports_images(model.supports_images);
            }
        }

        let chat_context = crate::chat_context::ChatContext::new(
            uuid::Uuid::nil(),
            String::new(),
            Vec::new(),
            None,
            active_model
                .as_ref()
                .map(|m| m.display_name.clone())
                .unwrap_or_default(),
            active_model
                .as_ref()
                .map(|m| m.provider_display_name.clone())
                .unwrap_or_default(),
        );
        self.message_list
            .get_or_insert_with(MessageList::new)
            .set_chat_context(chat_context);

        self.screen = AppScreen::Welcome;

        if let Some(ref mut composer) = self.composer {
            composer.clear();
        }

        self.abort_confirmation_deadline = None;
        // Drop stale notices (e.g. "Sending...") that belong to the session
        // being left, otherwise the footer would surface them on the
        // welcome screen.
        self.last_notice = None;
        self.context_usage = None;
        self.shown_instruction_sources.clear();
    }
}

impl App {
    /// Process an [`McpAction`] — update both the in-memory McpManager and
    /// the persisted AppConfig, then save to disk.
    fn handle_mcp_action(&mut self, action: McpAction) {
        match action {
            McpAction::Toggle(name) => {
                let mcp = self.runtime.mcp_manager().clone();
                let notice_tx = self.mcp_notice_tx.clone();
                let name_for_config = name.clone();
                let mut target_config = None;
                self.runtime.update_config(|cfg| {
                    if let Some(srv) = cfg.mcp.servers.get_mut(&name_for_config) {
                        let new_disabled = !srv.is_disabled();
                        srv.set_disabled(new_disabled);
                        target_config = Some((new_disabled, srv.clone()));
                    }
                });
                let _ = self.runtime.save_config();

                tokio::spawn(async move {
                    if let Some((disabled, cfg)) = target_config {
                        if let Err(error) = mcp.upsert_server(name_for_config.clone(), cfg).await {
                            let _ = notice_tx.send(format!(
                                "MCP server '{name_for_config}' update failed: {error:#}"
                            ));
                        } else if disabled {
                            if let Err(error) = mcp.disconnect_server(&name_for_config).await {
                                let _ = notice_tx.send(format!(
                                    "MCP server '{name_for_config}' disconnect failed: {error:#}"
                                ));
                            }
                        } else {
                            if let Err(error) = mcp.refresh_server(&name_for_config).await {
                                let _ = notice_tx.send(format!(
                                    "MCP server '{name_for_config}' refresh failed: {error:#}"
                                ));
                            }
                        }
                    } else {
                        if let Err(error) = mcp.toggle_server(&name).await {
                            let _ = notice_tx
                                .send(format!("MCP server '{name}' toggle failed: {error:#}"));
                        }
                    }
                });
            }
            McpAction::Refresh(name) => {
                let mcp = self.runtime.mcp_manager().clone();
                let notice_tx = self.mcp_notice_tx.clone();
                tokio::spawn(async move {
                    if let Err(error) = mcp.refresh_server(&name).await {
                        let _ = notice_tx
                            .send(format!("MCP server '{name}' refresh failed: {error:#}"));
                    }
                });
            }
            McpAction::Remove(name) => {
                // Remove from McpManager.
                let mcp = self.runtime.mcp_manager().clone();
                let notice_tx = self.mcp_notice_tx.clone();
                let name_for_spawn = name.clone();
                tokio::spawn(async move {
                    if let Err(error) = mcp.remove_server(&name_for_spawn).await {
                        let _ = notice_tx.send(format!(
                            "MCP server '{name_for_spawn}' removal failed: {error:#}"
                        ));
                    }
                });

                // Remove from persisted config.
                self.runtime.update_config(|cfg| {
                    cfg.mcp.servers.remove(&name);
                });
                let _ = self.runtime.save_config();
            }
            McpAction::Upsert {
                name,
                config,
                original_name,
            } => {
                // Clone for the async spawn, keep reference for config update.
                let name_for_spawn = name.clone();
                let cfg_for_spawn = config.clone();
                let orig_for_spawn = original_name.clone();

                // Upsert in McpManager.
                let mcp = self.runtime.mcp_manager().clone();
                let notice_tx = self.mcp_notice_tx.clone();
                tokio::spawn(async move {
                    // If renaming, remove the old entry first.
                    if let Some(ref orig) = orig_for_spawn
                        && orig != &name_for_spawn
                        && let Err(error) = mcp.remove_server(orig).await
                    {
                        let _ = notice_tx.send(format!(
                            "MCP server '{orig}' rename cleanup failed: {error:#}"
                        ));
                        return;
                    }
                    if let Err(error) = mcp
                        .upsert_server(name_for_spawn.clone(), cfg_for_spawn)
                        .await
                    {
                        let _ = notice_tx.send(format!(
                            "MCP server '{name_for_spawn}' update failed: {error:#}"
                        ));
                    }
                });

                // Persist config change.
                self.runtime.update_config(|cfg| {
                    // Remove the old name if it changed.
                    if let Some(ref orig) = original_name {
                        cfg.mcp.servers.remove(orig);
                    }
                    cfg.mcp.servers.insert(name, config);
                });
                let _ = self.runtime.save_config();
            }
        }
    }

    /// Restore the active model and thinking level when entering a session.
    fn sync_active_model_for_session(
        &mut self,
        session_id: uuid::Uuid,
        thinking_level: Option<ThinkingLevelType>,
    ) {
        let Ok(Some(record)) = self.runtime.session_manager().load_session(session_id) else {
            return;
        };
        let config = self.runtime.config();
        let auth = self.runtime.auth();
        let Ok(mut model) =
            config.resolve_model_by_ids(&auth, &record.provider_id, &record.model_id)
        else {
            return;
        };

        if let Some(level) = thinking_level {
            model.thinking_level = level;
        }
        self.thinking_level = model.thinking_level.clone();
        self.runtime.set_active_model(model.clone());
        if let Some(ref mut composer) = self.composer {
            composer.set_model_supports_images(model.supports_images);
        }
    }

    /// Copy the latest completed assistant response without involving the runtime.
    ///
    /// This is intentionally a TUI-only operation: `/copy` must never create a
    /// user message or alter the bytes of a future LLM request.
    fn copy_last_assistant_message(&mut self) {
        let Some(session_id) = self.current_session_id else {
            self.set_toast(
                self.ui_text().text(TextKey::CopyAvailableActive),
                std::time::Duration::from_secs(3),
            );
            return;
        };

        let has_pending_input = self
            .pending_inputs
            .iter()
            .any(|input| input.session_id == session_id);
        let has_pending_compact = self.pending_compacts.contains(&session_id);
        let is_compacting = self.compacting_sessions.contains(&session_id);
        if self.has_active_request() || has_pending_input || has_pending_compact || is_compacting {
            self.set_toast(
                self.ui_text().text(TextKey::CopyAvailableIdle),
                std::time::Duration::from_secs(3),
            );
            return;
        }

        let content = self
            .message_list
            .as_ref()
            .and_then(|chat| chat.active_chat_context())
            .filter(|context| context.session_id == session_id)
            .and_then(|context| last_copyable_assistant_content(context.visible_messages()));

        let Some(content) = content else {
            self.set_toast(
                self.ui_text().text(TextKey::NoAssistantToCopy),
                std::time::Duration::from_secs(3),
            );
            return;
        };

        match copy_to_clipboard(&content) {
            Ok(()) => self.set_toast(
                self.ui_text().text(TextKey::AssistantCopied),
                std::time::Duration::from_secs(3),
            ),
            Err(error) => self.set_toast(
                self.ui_text()
                    .text_with_value(TextKey::CopyFailed, "error", &error.to_string()),
                std::time::Duration::from_secs(5),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::last_copyable_assistant_content;
    use tidev_llm::message::{Message, MessageRole};

    #[test]
    fn last_copyable_assistant_content_uses_latest_completed_text() {
        let mut streaming = Message::streaming(MessageRole::Assistant, "partial");
        streaming.content = "partial".to_string();
        let messages = vec![
            Message::new(MessageRole::Assistant, "first"),
            Message::new(MessageRole::Assistant, ""),
            streaming,
            Message::new(MessageRole::Assistant, "latest"),
        ];

        assert_eq!(
            last_copyable_assistant_content(&messages).as_deref(),
            Some("latest")
        );
    }

    #[test]
    fn last_copyable_assistant_content_skips_non_assistant_messages() {
        let messages = vec![
            Message::new(MessageRole::Assistant, "answer"),
            Message::new(MessageRole::User, "follow-up"),
            Message::new(MessageRole::Tool, "tool output"),
        ];

        assert_eq!(
            last_copyable_assistant_content(&messages).as_deref(),
            Some("answer")
        );
    }
}
