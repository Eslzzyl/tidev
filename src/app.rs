use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use crossterm::{
    cursor::Show,
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
    },
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use image::ImageEncoder;
use ratatui::text::Line;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    env, io,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tokio::{
    runtime::Runtime,
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
};
use uuid::Uuid;

mod at_mention;
mod connect;
mod model_panel;
mod permission;
mod render;
mod render_chat;
mod render_dialog;
mod session_panel;
mod theme_panel;
mod undo;

use crate::{
    app::at_mention::{AtMentionKind, AtMentionState, current_at_fragment},
    app::model_panel::ModelPanelState,
    app::permission::{PendingToolExecution, PermissionDialogState, RunningToolExecution},
    app::session_panel::SessionPanelState,
    app::theme_panel::ThemePanelState,
    commands::{CommandAction, CommandPaletteState, CommandRegistry},
    config::{ActiveModel, AppConfig, AuthStore, ConfigPaths},
    context::ContextManager,
    input::Composer,
    instructions,
    llm::LlmClient,
    markdown_stream::MarkdownStreamCollector,
    prompts::SessionMode,
    provider_setup::ConnectDialog,
    session::{AssistantTurn, BackendEvent, Conversation, Message, MessageAttachment, MessageRole},
    storage::SessionStore,
    theme::{ThemeManager, ThemeName},
    tooling::ToolRegistry,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Welcome,
    Chat,
}

struct App {
    should_quit: bool,
    screen: Screen,
    workspace_root: PathBuf,
    paths: ConfigPaths,
    config: AppConfig,
    auth: AuthStore,
    store: SessionStore,
    llm: LlmClient,
    theme: ThemeManager,
    mode: SessionMode,
    active_model: ActiveModel,
    conversation: Conversation,
    context_manager: ContextManager,
    tools: ToolRegistry,
    commands: CommandRegistry,
    command_palette: CommandPaletteState,
    connect_dialog: Option<ConnectDialog>,
    theme_panel: Option<ThemePanelState>,
    model_panel: Option<ModelPanelState>,
    session_panel: Option<SessionPanelState>,
    at_mention: AtMentionState,
    pending_tool_execution: Option<PendingToolExecution>,
    permission_dialog: Option<PermissionDialogState>,
    running_tool_execution: Option<RunningToolExecution>,
    composer: Composer,
    draft_attachments: Vec<MessageAttachment>,
    pending_request: bool,
    active_request_id: u64,
    abort_confirmation_deadline: Option<Instant>,
    last_notice: Option<String>,
    message_scroll_offset: usize,
    message_follow_tail: bool,
    message_viewport_lines: usize,
    message_total_lines: usize,
    backend_tx: UnboundedSender<BackendEvent>,
    backend_rx: UnboundedReceiver<BackendEvent>,
    streaming_markdown: Option<MarkdownStreamCollector>,
    streaming_preview_lines: Vec<Line<'static>>,
    loading_frame: usize,
}

pub fn run() -> Result<()> {
    let runtime = Runtime::new().context("failed to create runtime")?;
    let mut app = App::new()?;
    app.run(&runtime)
}

impl App {
    fn new() -> Result<Self> {
        let workspace_root = env::current_dir().context("failed to determine workspace root")?;
        let paths = ConfigPaths::discover()?;
        let config = AppConfig::load_or_create(&paths)?;
        let auth = AuthStore::load_or_create(&paths)?;
        let store = SessionStore::open(paths.default_database_path())?;
        let llm = LlmClient::new()?;
        let theme = ThemeManager::new(&config.theme);
        let tools = ToolRegistry::new(workspace_root.clone());
        let commands = CommandRegistry::new();
        let command_palette = CommandPaletteState::default();
        let composer = Composer::new("Ask TiDev about your code, task, or question...");
        let (backend_tx, backend_rx) = unbounded_channel();
        let mode = SessionMode::Build;

        let fallback_model = Self::resolve_fallback_model(&config, &auth)?;
        let session_id = Uuid::new_v4();
        let conversation = Conversation::new(
            session_id,
            workspace_root.display().to_string(),
            fallback_model.provider_id.clone(),
            fallback_model.provider_display_name.clone(),
            fallback_model.model_id.clone(),
            fallback_model.display_name.clone(),
            "Untitled session",
        );
        store.create_session(
            session_id,
            workspace_root.as_path(),
            &fallback_model.provider_id,
            &fallback_model.provider_display_name,
            &fallback_model.model_id,
            &fallback_model.display_name,
            &conversation.title,
        )?;

        let active_model = fallback_model.clone();
        let last_notice = None;

        Ok(Self {
            should_quit: false,
            screen: Screen::Welcome,
            workspace_root,
            paths,
            config,
            auth,
            store,
            llm,
            theme,
            mode,
            active_model,
            conversation,
            context_manager: ContextManager::new(),
            tools,
            commands,
            command_palette,
            connect_dialog: None,
            theme_panel: None,
            model_panel: None,
            session_panel: None,
            at_mention: AtMentionState::default(),
            pending_tool_execution: None,
            permission_dialog: None,
            running_tool_execution: None,
            composer,
            draft_attachments: Vec::new(),
            pending_request: false,
            active_request_id: 0,
            abort_confirmation_deadline: None,
            last_notice,
            message_scroll_offset: 0,
            message_follow_tail: true,
            message_viewport_lines: 0,
            message_total_lines: 0,
            backend_tx,
            backend_rx,
            streaming_markdown: None,
            streaming_preview_lines: Vec::new(),
            loading_frame: 0,
        })
    }

    fn run(&mut self, runtime: &Runtime) -> Result<()> {
        let _session = TerminalSession::enter()?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
        terminal.clear().context("failed to clear terminal")?;

        loop {
            self.process_backend_events(runtime)?;
            terminal
                .draw(|frame| self.render(frame))
                .context("failed to render frame")?;

            if self.should_quit {
                break;
            }

            if event::poll(Duration::from_millis(50)).context("failed to poll terminal events")? {
                let event = event::read().context("failed to read terminal event")?;
                self.handle_event(event, runtime)?;
            }

            if self.should_quit {
                break;
            }
        }

        terminal.show_cursor().ok();
        Ok(())
    }

    fn handle_event(&mut self, event: Event, runtime: &Runtime) -> Result<()> {
        match event {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.handle_key_event(key, runtime)?;
            }
            Event::Paste(text) => {
                self.handle_text_paste(&text)?;
            }
            Event::Mouse(mouse) => {
                self.handle_mouse_event(mouse);
            }
            Event::Resize(_, _) => {}
            _ => {}
        }

        Ok(())
    }

    fn can_scroll_conversation(&self) -> bool {
        self.screen == Screen::Chat
            && self.permission_dialog.is_none()
            && self.connect_dialog.is_none()
            && self.theme_panel.is_none()
            && self.model_panel.is_none()
            && !self.command_palette.visible
    }

    fn scroll_messages_to_bottom(&mut self) {
        self.message_scroll_offset = 0;
        self.message_follow_tail = true;
    }

    fn message_scroll_max(&self) -> usize {
        self.message_total_lines
            .saturating_sub(self.message_viewport_lines)
    }

    fn message_scroll_page(&self) -> usize {
        self.message_viewport_lines.saturating_sub(1).max(1)
    }

    fn scroll_messages_up(&mut self, lines: usize) {
        let max_scroll = self.message_scroll_max();
        let current = if self.message_follow_tail {
            max_scroll
        } else {
            self.message_scroll_offset.min(max_scroll)
        };

        self.message_scroll_offset = current.saturating_sub(lines);
        self.message_follow_tail = self.message_scroll_offset >= max_scroll;
    }

    fn scroll_messages_down(&mut self, lines: usize) {
        let max_scroll = self.message_scroll_max();
        let current = if self.message_follow_tail {
            max_scroll
        } else {
            self.message_scroll_offset.min(max_scroll)
        };

        self.message_scroll_offset = current.saturating_add(lines).min(max_scroll);
        self.message_follow_tail = self.message_scroll_offset >= max_scroll;
    }

    fn handle_message_scroll_key(&mut self, key: KeyEvent) -> bool {
        if !self.can_scroll_conversation() {
            return false;
        }

        match key.code {
            KeyCode::PageUp => {
                self.scroll_messages_up(self.message_scroll_page());
                true
            }
            KeyCode::PageDown => {
                self.scroll_messages_down(self.message_scroll_page());
                true
            }
            _ => false,
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        if !self.can_scroll_conversation() {
            return;
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll_messages_up(3),
            MouseEventKind::ScrollDown => self.scroll_messages_down(3),
            _ => {}
        }
    }

    fn handle_request_abort_key(&mut self, key: KeyEvent) -> Result<bool> {
        if key.code != KeyCode::Esc || !self.pending_request {
            return Ok(false);
        }

        if self
            .abort_confirmation_deadline
            .is_some_and(|deadline| deadline > Instant::now())
        {
            self.abort_current_request();
            return Ok(true);
        }

        self.abort_confirmation_deadline = Some(Instant::now() + Duration::from_secs(3));
        self.last_notice =
            Some("Press Esc again within 3 seconds to stop the current request".to_string());
        Ok(true)
    }

    fn is_active_request(&self, request_id: u64) -> bool {
        request_id == self.active_request_id
    }

    fn abort_current_request(&mut self) {
        self.active_request_id = self.active_request_id.wrapping_add(1);
        self.abort_confirmation_deadline = None;
        self.pending_request = false;
        self.pending_tool_execution = None;
        self.permission_dialog = None;

        if let Some(running) = self.running_tool_execution.take() {
            running.cancel_requested.store(true, Ordering::SeqCst);
        }

        self.streaming_markdown = None;
        self.streaming_preview_lines.clear();

        if let Some(message) = self.conversation.messages.last_mut()
            && message.streaming
            && matches!(message.role, MessageRole::Assistant)
        {
            message.role = MessageRole::Error;
            message.streaming = false;
            message.content = "Request cancelled".to_string();
            let persisted = message.clone();
            let _ = self
                .store
                .append_message(self.conversation.session_id, &persisted);
        }

        self.last_notice = Some("Request cancelled".to_string());
    }

    fn handle_theme_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        if let Some(panel) = &mut self.theme_panel {
            match key.code {
                KeyCode::Up => {
                    let previous_theme = panel.preview_theme;
                    panel.move_up();
                    if panel.preview_theme != previous_theme {
                        self.theme.set_mode(panel.preview_theme);
                    }
                }
                KeyCode::Down => {
                    let previous_theme = panel.preview_theme;
                    panel.move_down();
                    if panel.preview_theme != previous_theme {
                        self.theme.set_mode(panel.preview_theme);
                    }
                }
                KeyCode::Enter => {
                    let _ = self.close_theme_panel(true);
                }
                KeyCode::Esc => {
                    let _ = self.close_theme_panel(false);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_model_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(panel) = self.model_panel.clone() else {
            return Ok(());
        };

        match key.code {
            KeyCode::Up => {
                let items = self.model_panel_items();
                let mut next_panel = panel;
                next_panel.move_selection(&items, -1);
                self.model_panel = Some(next_panel);
            }
            KeyCode::Down => {
                let items = self.model_panel_items();
                let mut next_panel = panel;
                next_panel.move_selection(&items, 1);
                self.model_panel = Some(next_panel);
            }
            KeyCode::Enter => {
                let items = self.model_panel_items();
                if let Some(summary) = panel.selected_model(&items).cloned() {
                    self.switch_model(Some(&summary.label()))?;
                    self.close_model_panel();
                }
            }
            KeyCode::Esc => {
                self.close_model_panel();
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let items = self.model_panel_items();
                if let Some(summary) = panel.selected_model(&items).cloned() {
                    self.close_model_panel();
                    self.begin_provider_edit_for_model(summary.provider_id, summary.model_id)?;
                }
            }
            KeyCode::Tab => {}
            _ => {
                let previous_query = self.composer.text().to_string();
                let _ = self.composer.handle_key_with_history(key, false);
                if self.composer.text() != previous_query {
                    let items = self.model_panel_items();
                    let mut next_panel = panel;
                    next_panel.reset_selection(&items);
                    self.model_panel = Some(next_panel);
                }
            }
        }

        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent, runtime: &Runtime) -> Result<()> {
        if matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('d')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return Ok(());
        }

        if self.permission_dialog.is_some() {
            return self.handle_permission_dialog_key(key, runtime);
        }

        if let Some(dialog) = self.connect_dialog.clone() {
            self.handle_connect_dialog_key(key, dialog)?;
            return Ok(());
        }

        if self.theme_panel.is_some() {
            return self.handle_theme_panel_key(key);
        }

        if self.model_panel.is_some() {
            return self.handle_model_panel_key(key);
        }

        if self.session_panel.is_some() {
            return self.handle_session_panel_key(key);
        }

        if self.handle_request_abort_key(key)? {
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('v'))
            && (key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::SUPER))
            && !key.modifiers.contains(KeyModifiers::ALT)
            && !key.modifiers.contains(KeyModifiers::SHIFT)
        {
            self.handle_clipboard_paste()?;
            return Ok(());
        }

        if !self.command_palette.visible && key.code == KeyCode::Tab {
            self.mode = self.mode.toggle();
            self.last_notice = Some(format!("Mode switched to {}", self.mode.as_str()));
            return Ok(());
        }

        if self.command_palette.visible {
            match key.code {
                KeyCode::Esc => {
                    self.command_palette.clear();
                    return Ok(());
                }
                KeyCode::Up => {
                    self.command_palette.move_selection(-1);
                    return Ok(());
                }
                KeyCode::Down => {
                    self.command_palette.move_selection(1);
                    return Ok(());
                }
                KeyCode::Tab => {
                    if let Some(completion) = self.command_palette.completion() {
                        self.composer.set_text(completion);
                    }
                    self.command_palette
                        .sync(self.composer.text(), &self.commands);
                    return Ok(());
                }
                KeyCode::Enter
                    if !key.modifiers.contains(KeyModifiers::SHIFT)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    if let Some(selected) = self.command_palette.selected() {
                        let command_line = format!("/{}", selected.spec.name);
                        self.composer.remember_submission(&command_line);
                        self.composer.clear();
                        self.command_palette.clear();
                        self.execute_command_line(&command_line, runtime)?;
                        return Ok(());
                    }
                }
                _ => {}
            }
        }

        if !self.command_palette.visible && self.handle_message_scroll_key(key) {
            return Ok(());
        }

        if self.at_mention.visible {
            match key.code {
                KeyCode::Esc => {
                    self.at_mention.clear();
                    return Ok(());
                }
                KeyCode::Up => {
                    self.at_mention.move_selection(-1);
                    return Ok(());
                }
                KeyCode::Down => {
                    self.at_mention.move_selection(1);
                    return Ok(());
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.accept_at_mention();
                    return Ok(());
                }
                _ => {}
            }
        }

        if let Some(submission) = self.composer.handle_key_with_history(key, true) {
            self.handle_submission(submission, runtime)?;
            self.at_mention.clear();
        } else {
            if key.code == KeyCode::Enter && !self.draft_attachments.is_empty() {
                self.handle_submission(String::new(), runtime)?;
                self.at_mention.clear();
                self.command_palette
                    .sync(self.composer.text(), &self.commands);
                return Ok(());
            }
            self.refresh_at_mention_state();
        }

        self.command_palette
            .sync(self.composer.text(), &self.commands);
        Ok(())
    }

    fn handle_submission(&mut self, submission: String, runtime: &Runtime) -> Result<()> {
        let trimmed = submission.trim();
        if trimmed.starts_with('/') {
            self.execute_command_line(trimmed, runtime)?;
            self.at_mention.clear();
            self.draft_attachments.clear();
        } else {
            self.submit_prompt(submission, runtime)?;
        }

        Ok(())
    }

    fn handle_text_paste(&mut self, text: &str) -> Result<()> {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        self.composer.insert_str(&normalized);
        self.refresh_at_mention_state();
        self.command_palette
            .sync(self.composer.text(), &self.commands);
        Ok(())
    }

    fn handle_clipboard_paste(&mut self) -> Result<()> {
        let mut clipboard = match arboard::Clipboard::new() {
            Ok(clipboard) => clipboard,
            Err(error) => {
                self.last_notice = Some(format!("Clipboard unavailable: {error}"));
                return Ok(());
            }
        };

        if let Ok(text) = clipboard.get_text() {
            if !text.is_empty() {
                return self.handle_text_paste(&text);
            }
        }

        let image = match clipboard.get_image() {
            Ok(image) => image,
            Err(_) => {
                self.last_notice =
                    Some("Clipboard does not contain pasteable text or image".to_string());
                return Ok(());
            }
        };

        if !self.active_model.supports_images {
            self.last_notice = Some("This model does not support image attachments".to_string());
            return Ok(());
        }

        let data_url = match png_data_url_from_clipboard_image(image) {
            Ok(value) => value,
            Err(error) => {
                self.last_notice = Some(format!("Failed to decode clipboard image: {error}"));
                return Ok(());
            }
        };

        self.draft_attachments.push(MessageAttachment::Image {
            filename: format!("pasted-image-{}.png", Uuid::new_v4()),
            mime: "image/png".to_string(),
            data_url,
        });
        self.last_notice = Some("Image pasted into draft".to_string());
        Ok(())
    }

    fn refresh_at_mention_state(&mut self) {
        if self.command_palette.visible
            || self.connect_dialog.is_some()
            || self.theme_panel.is_some()
            || self.model_panel.is_some()
            || self.session_panel.is_some()
        {
            self.at_mention.clear();
            return;
        }

        let text = self.composer.text();
        let cursor = self.composer.cursor();
        self.at_mention
            .sync(self.workspace_root.as_path(), text, cursor);
    }

    fn accept_at_mention(&mut self) {
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
        self.command_palette
            .sync(self.composer.text(), &self.commands);
    }

    fn execute_command_line(&mut self, line: &str, runtime: &Runtime) -> Result<()> {
        let Some((name, args)) = self.commands.parse_invocation(line) else {
            self.last_notice = Some("Invalid command".to_string());
            return Ok(());
        };

        let Some(spec) = self.commands.command(&name).cloned() else {
            self.last_notice = Some(format!("Unknown command '/{name}'"));
            return Ok(());
        };

        self.run_command(spec.name, spec.action, &args, runtime)?;
        self.commands.mark_used(spec.name);
        Ok(())
    }

    fn run_command(
        &mut self,
        _command_name: &str,
        action: CommandAction,
        args: &[String],
        _runtime: &Runtime,
    ) -> Result<()> {
        if self.pending_request {
            match action {
                CommandAction::Help
                | CommandAction::Theme
                | CommandAction::Quit
                | CommandAction::Undo
                | CommandAction::Redo => {}
                _ => {
                    self.last_notice = Some(
                        "A response is still streaming. Wait for it to finish before changing sessions.".to_string(),
                    );
                    return Ok(());
                }
            }
        }

        match action {
            CommandAction::Help => {
                let help = self.help_message();
                self.push_system_message(help)?;
                self.scroll_messages_to_bottom();
                self.last_notice = Some("Help shown".to_string());
            }
            CommandAction::Connect => {
                if !args.is_empty() {
                    self.last_notice = Some("Ignoring arguments to /connect".to_string());
                }
                self.open_connect_dialog()?;
            }
            CommandAction::Model => {
                self.open_model_panel(args.join(" "));
            }
            CommandAction::Session => {
                self.open_session_panel(args.join(" "))?;
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
            CommandAction::Quit => {
                self.should_quit = true;
            }
        }

        Ok(())
    }

    fn apply_theme_command(&mut self, args: &[String]) -> Result<()> {
        let direct_theme = args.first().and_then(|v| ThemeName::parse(v));

        if let Some(theme) = direct_theme {
            self.apply_theme(theme)?;
            Ok(())
        } else {
            self.open_theme_panel();
            Ok(())
        }
    }

    fn open_theme_panel(&mut self) {
        self.theme_panel = Some(ThemePanelState::new(self.theme.palette().name));
    }

    fn open_model_panel(&mut self, initial_query: String) {
        self.command_palette.clear();
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.composer.clear();
        self.composer
            .set_placeholder("Search connected models by provider or model name");
        self.composer.set_text(initial_query);

        let mut panel = ModelPanelState::new();
        let items = self.model_panel_items();
        panel.reset_selection(&items);
        self.model_panel = Some(panel);
    }

    fn close_model_panel(&mut self) {
        self.model_panel = None;
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.composer.clear();
        self.composer
            .set_placeholder("Ask TiDev about your code, task, or question...");
    }

    fn close_theme_panel(&mut self, apply: bool) -> Result<()> {
        if let Some(panel) = self.theme_panel.take() {
            if apply {
                self.apply_theme(panel.preview_theme)?;
            } else {
                self.theme.set_mode(panel.original_theme);
            }
        }
        Ok(())
    }

    fn apply_theme(&mut self, theme: ThemeName) -> Result<()> {
        self.theme.set_mode(theme);
        self.config.set_theme(theme);
        self.config.save(&self.paths)?;
        self.last_notice = Some(format!("Theme switched to {}", self.theme.name()));
        Ok(())
    }

    fn switch_model(&mut self, selector: Option<&str>) -> Result<()> {
        let model = self.config.resolve_model(&self.auth, selector)?;
        self.active_model = model.clone();
        self.conversation.set_model(
            model.provider_id.clone(),
            model.provider_display_name.clone(),
            model.model_id.clone(),
            model.display_name.clone(),
        );
        self.store.update_session_model(
            self.conversation.session_id,
            &model.provider_id,
            &model.provider_display_name,
            &model.model_id,
            &model.display_name,
        )?;
        self.last_notice = Some(format!("Switched to {}", model.label()));
        Ok(())
    }

    fn start_new_session(&mut self) -> Result<()> {
        let session_id = Uuid::new_v4();
        let conversation = Conversation::new(
            session_id,
            self.workspace_root.display().to_string(),
            self.active_model.provider_id.clone(),
            self.active_model.provider_display_name.clone(),
            self.active_model.model_id.clone(),
            self.active_model.display_name.clone(),
            "Untitled session",
        );

        self.store.create_session(
            session_id,
            self.workspace_root.as_path(),
            &self.active_model.provider_id,
            &self.active_model.provider_display_name,
            &self.active_model.model_id,
            &self.active_model.display_name,
            &conversation.title,
        )?;

        self.conversation = conversation;
        self.context_manager = ContextManager::new();
        self.pending_request = false;
        self.pending_tool_execution = None;
        self.permission_dialog = None;
        self.running_tool_execution = None;
        self.abort_confirmation_deadline = None;
        self.active_request_id = self.active_request_id.wrapping_add(1);
        self.streaming_markdown = None;
        self.streaming_preview_lines.clear();
        self.screen = Screen::Welcome;
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.command_palette.clear();
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.composer.clear();
        self.composer
            .set_placeholder("Ask TiDev about your code, task, or question...");
        self.scroll_messages_to_bottom();
        self.last_notice = Some("Started a fresh session".to_string());

        Ok(())
    }

    fn submit_prompt(&mut self, prompt: String, runtime: &Runtime) -> Result<()> {
        let prompt = prompt.trim().to_string();
        if self.pending_request {
            self.last_notice = Some("A response is already in progress".to_string());
            return Ok(());
        }

        if prompt.is_empty() && self.draft_attachments.is_empty() {
            return Ok(());
        }

        self.screen = Screen::Chat;
        self.command_palette.clear();
        self.connect_dialog = None;

        if self.conversation.is_reverted() {
            self.discard_reverted_branch()?;
            self.context_manager = ContextManager::new();
        }

        let attachments = self.build_prompt_attachments(&prompt)?;
        if attachments.iter().any(MessageAttachment::is_image) && !self.active_model.supports_images
        {
            self.last_notice = Some("This model does not support image attachments".to_string());
            return Ok(());
        }

        let mut user_message = Message::new(MessageRole::User, prompt.clone());
        user_message.attachments = attachments;
        self.conversation.push(user_message.clone());
        self.store
            .append_message(self.conversation.session_id, &user_message)?;

        self.draft_attachments.clear();

        if let Err(error) = self.capture_prompt_snapshot(user_message.id) {
            self.last_notice = Some(format!("Workspace snapshot unavailable: {error}"));
        }

        if self.conversation.messages.len() == 1 || self.conversation.title == "Untitled session" {
            self.conversation.update_title_from_prompt(&prompt);
            self.store
                .update_session_title(self.conversation.session_id, &self.conversation.title)?;
        }

        self.scroll_messages_to_bottom();

        let compacted = {
            let llm = self.llm.clone();
            let active_model = self.active_model.clone();
            let conversation = self.conversation.clone();
            runtime.block_on(self.context_manager.compact_if_needed(
                &llm,
                &active_model,
                &conversation,
            ))
        };

        match compacted {
            Ok(true) => {
                self.last_notice = Some("Context compacted".to_string());
            }
            Ok(false) => {}
            Err(error) => {
                self.last_notice = Some(error.to_string());
            }
        }

        self.start_assistant_turn(runtime)
    }

    fn build_prompt_attachments(&self, prompt: &str) -> Result<Vec<MessageAttachment>> {
        let mut attachments = Vec::new();
        let mut seen_paths = std::collections::BTreeSet::new();

        for path in self.inline_file_references(prompt) {
            if !seen_paths.insert(path.clone()) {
                continue;
            }

            let absolute = self.resolve_workspace_path(&path);
            match self.build_attachment_for_path(&path, &absolute)? {
                Some(attachment) => attachments.push(attachment),
                None => continue,
            }
        }

        attachments.extend(self.draft_attachments.iter().cloned());
        Ok(attachments)
    }

    fn build_attachment_for_path(
        &self,
        path: &str,
        absolute: &Path,
    ) -> Result<Option<MessageAttachment>> {
        let metadata = match std::fs::metadata(absolute) {
            Ok(metadata) => metadata,
            Err(_error) => return Ok(None),
        };

        if metadata.is_dir() {
            let tree = build_directory_tree(absolute, 2, 80)?;
            return Ok(Some(MessageAttachment::DirectoryReference {
                path: path.trim_end_matches(['/', '\\']).to_string(),
                tree,
            }));
        }

        if let Some(mime) = image_mime_from_path(absolute) {
            let bytes = std::fs::read(absolute)?;
            let filename = absolute
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(path)
                .to_string();
            let data_url = format!("data:{mime};base64,{}", BASE64_STANDARD.encode(bytes));
            return Ok(Some(MessageAttachment::Image {
                filename,
                mime: mime.to_string(),
                data_url,
            }));
        }

        let content = match std::fs::read_to_string(absolute) {
            Ok(content) => content,
            Err(_error) => return Ok(None),
        };

        Ok(Some(MessageAttachment::FileReference {
            path: path.to_string(),
            content,
        }))
    }

    fn inline_file_references(&self, prompt: &str) -> Vec<String> {
        let mut paths = Vec::new();
        let mut seen = std::collections::BTreeSet::new();

        for token in prompt.split_whitespace() {
            let Some(path) = token.strip_prefix('@') else {
                continue;
            };
            let path = path.trim_matches(|ch: char| {
                matches!(ch, ',' | '.' | ';' | ':' | ')' | ']' | '"' | '\'')
            });
            if path.is_empty() {
                continue;
            }
            if !seen.insert(path.to_string()) {
                continue;
            }
            paths.push(path.to_string());
        }

        paths
    }

    fn resolve_workspace_path(&self, path: &str) -> PathBuf {
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            return candidate.to_path_buf();
        }
        self.workspace_root.join(path)
    }

    fn start_assistant_turn(&mut self, runtime: &Runtime) -> Result<()> {
        self.streaming_markdown = Some(MarkdownStreamCollector::new(
            None,
            self.workspace_root.as_path(),
        ));
        self.streaming_preview_lines.clear();
        self.pending_request = true;
        self.abort_confirmation_deadline = None;
        self.active_request_id = self.active_request_id.wrapping_add(1);
        let request_id = self.active_request_id;
        self.last_notice = Some(match self.mode {
            SessionMode::Plan => "Planning...".to_string(),
            SessionMode::Build => "Thinking...".to_string(),
        });

        let llm = self.llm.clone();
        let (system_prompt, instruction_sources) = self.compose_system_prompt();
        let mut model = self.active_model.clone();
        model.system_prompt = system_prompt;

        let _ = self.push_loaded_instruction_sources_message(&instruction_sources);

        let assistant_message = Message::streaming(MessageRole::Assistant, "");
        self.conversation.push(assistant_message);

        let messages = self.conversation.visible_messages().to_vec();
        let tools = self.tools.available_definitions(self.mode);
        let tx = self.backend_tx.clone();

        runtime.spawn(async move {
            llm.stream_chat(request_id, model, messages, tools, tx)
                .await;
        });

        Ok(())
    }

    fn compose_system_prompt(&self) -> (String, Vec<String>) {
        let base_prompt = self.active_model.system_prompt.trim();
        let mode_reminder = self.mode.reminder();
        let (instruction_prompt, sources) = instructions::system_prompt_and_sources(
            &self.workspace_root,
            &self.paths.config_dir,
            &self.config.instructions,
        )
        .unwrap_or_default();

        let mut prompt = String::new();
        if !base_prompt.is_empty() {
            prompt.push_str(base_prompt);
        }
        if !instruction_prompt.is_empty() {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str(&instruction_prompt);
        }
        if !prompt.is_empty() {
            prompt.push_str("\n\n");
        }
        prompt.push_str(mode_reminder);

        (prompt, sources)
    }

    fn push_loaded_instruction_sources_message(&mut self, sources: &[String]) -> Result<()> {
        if sources.is_empty() {
            return Ok(());
        }

        let already_present = self.conversation.messages.iter().any(|message| {
            matches!(message.role, MessageRole::System)
                && message.content.starts_with("Loaded instruction sources:")
        });

        if already_present {
            return Ok(());
        }

        let display_sources: Vec<String> = sources
            .iter()
            .map(|source| self.display_instruction_source(source))
            .collect();
        let content = format!("Loaded instruction sources: {}", display_sources.join(", "));
        self.push_system_message(content)
    }

    fn display_instruction_source(&self, source: &str) -> String {
        if source.starts_with("http://") || source.starts_with("https://") {
            return source.to_string();
        }

        let path = Path::new(source);
        if path.is_absolute() {
            if let Ok(rel) = path.strip_prefix(&self.workspace_root) {
                return rel.display().to_string();
            }
        }

        source.to_string()
    }

    fn push_system_message(&mut self, content: impl Into<String>) -> Result<()> {
        self.push_message(MessageRole::System, content)
    }

    fn push_message(&mut self, role: MessageRole, content: impl Into<String>) -> Result<()> {
        let message = Message::new(role, content);
        self.conversation.push(message.clone());
        self.store
            .append_message(self.conversation.session_id, &message)?;
        self.screen = Screen::Chat;
        Ok(())
    }

    fn process_backend_events(&mut self, runtime: &Runtime) -> Result<()> {
        while let Ok(event) = self.backend_rx.try_recv() {
            self.handle_backend_event(event, runtime)?;
        }

        Ok(())
    }

    fn handle_backend_event(&mut self, event: BackendEvent, runtime: &Runtime) -> Result<()> {
        match event {
            BackendEvent::Delta {
                request_id,
                content,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                if let Some(collector) = self.streaming_markdown.as_mut() {
                    collector.push_delta(&content);
                    self.streaming_preview_lines
                        .extend(collector.commit_complete_lines());
                }
                if let Some(message) = self.conversation.messages.last_mut()
                    && message.streaming
                    && matches!(message.role, MessageRole::Assistant)
                {
                    message.content.push_str(&content);
                }
            }
            BackendEvent::ReasoningDelta {
                request_id,
                content,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                if let Some(message) = self.conversation.messages.last_mut()
                    && message.streaming
                    && matches!(message.role, MessageRole::Assistant)
                {
                    message.reasoning.push_str(&content);
                }
            }
            BackendEvent::Finished { request_id, turn } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                self.finish_assistant_turn(turn, runtime)?;
            }
            BackendEvent::Failed { request_id, error } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                self.pending_request = false;
                self.pending_tool_execution = None;
                self.permission_dialog = None;
                self.running_tool_execution = None;
                self.abort_confirmation_deadline = None;
                self.streaming_markdown = None;
                self.streaming_preview_lines.clear();

                if let Some(message) = self.conversation.messages.last_mut()
                    && message.streaming
                    && matches!(message.role, MessageRole::Assistant)
                {
                    message.role = MessageRole::Error;
                    message.streaming = false;
                    message.content = format!("Request failed: {error}");
                    let persisted = message.clone();
                    self.store
                        .append_message(self.conversation.session_id, &persisted)?;
                    self.last_notice = Some(error);
                    return Ok(());
                }

                let message = Message::new(MessageRole::Error, format!("Request failed: {error}"));
                self.conversation.push(message.clone());
                self.store
                    .append_message(self.conversation.session_id, &message)?;
                self.last_notice = Some(error);
            }
            BackendEvent::ToolCompleted {
                request_id,
                tool_call,
                output,
            } => {
                if !self.is_active_request(request_id) {
                    return Ok(());
                }

                let Some(running) = self.running_tool_execution.take() else {
                    return Ok(());
                };

                if running.request_id != request_id || running.tool_call.id != tool_call.id {
                    self.running_tool_execution = Some(running);
                    return Ok(());
                }

                self.record_tool_result(tool_call, output)?;
                self.advance_pending_tool_execution();
                self.process_pending_tool_execution(runtime)?;
            }
        }

        Ok(())
    }

    fn finish_assistant_turn(&mut self, turn: AssistantTurn, runtime: &Runtime) -> Result<()> {
        let mut persisted_message = None;

        if let Some(message) = self.conversation.messages.last_mut()
            && message.streaming
            && matches!(message.role, MessageRole::Assistant)
        {
            message.content = turn.content.clone();
            message.reasoning = turn.reasoning.clone();
            message.tool_calls = turn.tool_calls.clone();
            message.streaming = false;
            persisted_message = Some(message.clone());
        }

        if let Some(message) = persisted_message {
            self.store
                .append_message(self.conversation.session_id, &message)?;
        }

        if let Some(collector) = self.streaming_markdown.as_mut() {
            self.streaming_preview_lines
                .extend(collector.finalize_and_drain());
        }
        self.streaming_markdown = None;
        self.streaming_preview_lines.clear();

        if !turn.tool_calls.is_empty() {
            self.last_notice = Some(format!("Running {} tool call(s)...", turn.tool_calls.len()));

            self.begin_tool_execution(turn.tool_calls, runtime)?;
            return Ok(());
        }

        self.pending_request = false;
        self.abort_confirmation_deadline = None;
        self.last_notice = Some(match turn.finish_reason.as_deref() {
            Some(reason) if reason != "stop" => format!("Response finished ({reason})"),
            _ => "Response complete".to_string(),
        });

        Ok(())
    }

    fn resolve_fallback_model(config: &AppConfig, auth: &AuthStore) -> Result<ActiveModel> {
        if let Ok(model) = config.resolve_active_model(auth) {
            return Ok(model);
        }

        let summary = config
            .available_models()
            .into_iter()
            .next()
            .context("no models are configured")?;

        config.resolve_model_by_ids(auth, &summary.provider_id, &summary.model_id)
    }

    fn resolve_conversation_model(
        config: &AppConfig,
        auth: &AuthStore,
        conversation: &Conversation,
    ) -> Result<ActiveModel> {
        config.resolve_model_by_ids(auth, &conversation.provider_id, &conversation.model_id)
    }
}

fn png_data_url_from_clipboard_image(image: arboard::ImageData<'_>) -> Result<String> {
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png);
    let width = image.width as u32;
    let height = image.height as u32;
    let rgba = image.bytes.into_owned();
    encoder
        .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
        .context("failed to encode clipboard image")?;
    Ok(format!(
        "data:image/png;base64,{}",
        BASE64_STANDARD.encode(png)
    ))
}

fn build_directory_tree(path: &Path, max_depth: usize, max_entries: usize) -> Result<String> {
    let label = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| path.display().to_string());

    let mut lines = vec![format!("{label}/")];
    let mut entry_count = 0usize;
    append_directory_tree(
        path,
        1,
        max_depth,
        max_entries,
        &mut entry_count,
        &mut lines,
    )?;
    Ok(lines.join("\n"))
}

fn append_directory_tree(
    path: &Path,
    depth: usize,
    max_depth: usize,
    max_entries: usize,
    entry_count: &mut usize,
    lines: &mut Vec<String>,
) -> Result<()> {
    if depth > max_depth || *entry_count >= max_entries {
        return Ok(());
    }

    let mut entries = Vec::new();
    for entry in
        std::fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {}", path.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        entries.push((file_type.is_dir(), name, entry.path()));
    }

    entries.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)));

    for (is_dir, name, child_path) in entries {
        if *entry_count >= max_entries {
            lines.push(format!("{}...", "  ".repeat(depth)));
            break;
        }

        let indent = "  ".repeat(depth);
        if is_dir {
            lines.push(format!("{indent}{name}/"));
            *entry_count += 1;
            append_directory_tree(
                &child_path,
                depth + 1,
                max_depth,
                max_entries,
                entry_count,
                lines,
            )?;
        } else {
            lines.push(format!("{indent}{name}"));
            *entry_count += 1;
        }
    }

    Ok(())
}

fn image_mime_from_path(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

struct TerminalSession;

impl TerminalSession {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw mode")?;
        crossterm::execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
            crossterm::cursor::Hide,
        )
        .context("failed to enter alternate screen")?;

        Ok(Self)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableBracketedPaste,
            DisableMouseCapture,
            Show,
        );
    }
}
