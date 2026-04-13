use anyhow::{Context, Result};
use crossterm::{
    cursor::Show,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{env, io, path::PathBuf, time::Duration};
use tokio::{
    runtime::Runtime,
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
};
use uuid::Uuid;

mod connect;
mod render;
mod theme_panel;

use crate::{
    commands::{CommandAction, CommandPaletteState, CommandRegistry},
    config::{ActiveModel, AppConfig, AuthStore, ConfigPaths},
    context::ContextManager,
    input::Composer,
    llm::LlmClient,
    prompts::SessionMode,
    provider_setup::ConnectDialog,
    session::{AssistantTurn, BackendEvent, Conversation, Message, MessageRole},
    storage::SessionStore,
    theme::{ThemeManager, ThemeName},
    tools::ToolRegistry,
    app::theme_panel::ThemePanelState,
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
    composer: Composer,
    pending_request: bool,
    last_notice: Option<String>,
    backend_tx: UnboundedSender<BackendEvent>,
    backend_rx: UnboundedReceiver<BackendEvent>,
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
        let (mut conversation, active_model) = match store.load_latest_session()? {
            Some(record) => match store.load_conversation(record.session_id)? {
                Some(conversation) => {
                    let active_model =
                        Self::resolve_conversation_model(&config, &auth, &conversation)
                            .unwrap_or_else(|_| fallback_model.clone());
                    (conversation, active_model)
                }
                None => {
                    let session_id = Uuid::new_v4();
                    let conversation = Conversation::new(
                        session_id,
                        fallback_model.provider_id.clone(),
                        fallback_model.model_id.clone(),
                        "Untitled session",
                    );
                    store.create_session(
                        session_id,
                        &fallback_model.provider_id,
                        &fallback_model.model_id,
                        &conversation.title,
                    )?;
                    (conversation, fallback_model.clone())
                }
            },
            None => {
                let session_id = Uuid::new_v4();
                let conversation = Conversation::new(
                    session_id,
                    fallback_model.provider_id.clone(),
                    fallback_model.model_id.clone(),
                    "Untitled session",
                );
                store.create_session(
                    session_id,
                    &fallback_model.provider_id,
                    &fallback_model.model_id,
                    &conversation.title,
                )?;
                (conversation, fallback_model.clone())
            }
        };

        if conversation.provider_id != active_model.provider_id
            || conversation.model_id != active_model.model_id
        {
            conversation.provider_id = active_model.provider_id.clone();
            conversation.model_id = active_model.model_id.clone();
            store.update_session_model(
                conversation.session_id,
                &active_model.provider_id,
                &active_model.model_id,
            )?;
        }

        let screen = if conversation.messages.is_empty() {
            Screen::Welcome
        } else {
            Screen::Chat
        };

        Ok(Self {
            should_quit: false,
            screen,
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
            composer,
            pending_request: false,
            last_notice: None,
            backend_tx,
            backend_rx,
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
            Event::Resize(_, _) => {}
            _ => {}
        }

        Ok(())
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

    fn handle_key_event(&mut self, key: KeyEvent, runtime: &Runtime) -> Result<()> {
        if matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('d')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return Ok(());
        }

        if let Some(dialog) = self.connect_dialog.clone() {
            self.handle_connect_dialog_key(key, dialog)?;
            return Ok(());
        }

        if self.theme_panel.is_some() {
            return self.handle_theme_panel_key(key);
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

        if let Some(submission) = self.composer.handle_key_with_history(key, true) {
            self.handle_submission(submission, runtime)?;
        }

        self.command_palette
            .sync(self.composer.text(), &self.commands);
        Ok(())
    }

    fn handle_submission(&mut self, submission: String, runtime: &Runtime) -> Result<()> {
        let trimmed = submission.trim();
        if trimmed.starts_with('/') {
            self.execute_command_line(trimmed, runtime)?;
        } else {
            self.submit_prompt(submission, runtime)?;
        }

        Ok(())
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
                CommandAction::Help | CommandAction::Theme | CommandAction::Quit => {}
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
                self.last_notice = Some("Help shown".to_string());
            }
            CommandAction::Connect => {
                self.open_connect_dialog(args.first().map(String::as_str))?;
            }
            CommandAction::Model => {
                if let Some(model_selector) = args.first() {
                    self.switch_model(Some(model_selector))?;
                } else {
                    let catalog = self.model_catalog_message();
                    self.push_system_message(catalog)?;
                    self.last_notice = Some("Model catalog shown".to_string());
                }
            }
            CommandAction::Clear => {
                self.start_new_session()?;
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
        self.conversation.provider_id = model.provider_id.clone();
        self.conversation.model_id = model.model_id.clone();
        self.store.update_session_model(
            self.conversation.session_id,
            &model.provider_id,
            &model.model_id,
        )?;
        self.last_notice = Some(format!("Switched to {}", model.label()));
        Ok(())
    }

    fn start_new_session(&mut self) -> Result<()> {
        let session_id = Uuid::new_v4();
        let conversation = Conversation::new(
            session_id,
            self.active_model.provider_id.clone(),
            self.active_model.model_id.clone(),
            "Untitled session",
        );

        self.store.create_session(
            session_id,
            &self.active_model.provider_id,
            &self.active_model.model_id,
            &conversation.title,
        )?;

        self.conversation = conversation;
        self.context_manager = ContextManager::new();
        self.pending_request = false;
        self.screen = Screen::Welcome;
        self.connect_dialog = None;
        self.command_palette.clear();
        self.composer.clear();
        self.composer
            .set_placeholder("Ask TiDev about your code, task, or question...");
        self.last_notice = Some("Started a fresh session".to_string());

        Ok(())
    }

    fn submit_prompt(&mut self, prompt: String, runtime: &Runtime) -> Result<()> {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return Ok(());
        }

        if self.pending_request {
            self.last_notice = Some("A response is already in progress".to_string());
            return Ok(());
        }

        self.screen = Screen::Chat;
        self.command_palette.clear();
        self.connect_dialog = None;

        let user_message = Message::new(MessageRole::User, prompt.clone());
        self.conversation.push(user_message.clone());
        self.store
            .append_message(self.conversation.session_id, &user_message)?;

        if self.conversation.messages.len() == 1 || self.conversation.title == "Untitled session" {
            self.conversation.update_title_from_prompt(&prompt);
            self.store
                .update_session_title(self.conversation.session_id, &self.conversation.title)?;
        }

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

    fn start_assistant_turn(&mut self, runtime: &Runtime) -> Result<()> {
        let assistant_message = Message::streaming(MessageRole::Assistant, "");
        self.conversation.push(assistant_message);
        self.pending_request = true;
        self.last_notice = Some(match self.mode {
            SessionMode::Plan => "Planning...".to_string(),
            SessionMode::Build => "Thinking...".to_string(),
        });

        let llm = self.llm.clone();
        let model = self.request_model();
        let messages = self.conversation.messages.clone();
        let tools = if self.mode.is_read_only() {
            Vec::new()
        } else {
            self.tools.definitions().to_vec()
        };
        let tx = self.backend_tx.clone();

        runtime.spawn(async move {
            llm.stream_chat(model, messages, tools, tx).await;
        });

        Ok(())
    }

    fn request_model(&self) -> ActiveModel {
        let mut model = self.active_model.clone();
        model.system_prompt = self.compose_system_prompt();
        model
    }

    fn compose_system_prompt(&self) -> String {
        let base_prompt = self.active_model.system_prompt.trim();
        let mode_reminder = self.mode.reminder();

        if base_prompt.is_empty() {
            mode_reminder.to_string()
        } else {
            format!("{base_prompt}\n\n{mode_reminder}")
        }
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
            BackendEvent::Delta(delta) => {
                if let Some(message) = self.conversation.messages.last_mut() {
                    if message.streaming && matches!(message.role, MessageRole::Assistant) {
                        message.content.push_str(&delta);
                    }
                }
            }
            BackendEvent::ReasoningDelta(delta) => {
                if let Some(message) = self.conversation.messages.last_mut() {
                    if message.streaming && matches!(message.role, MessageRole::Assistant) {
                        message.reasoning.push_str(&delta);
                    }
                }
            }
            BackendEvent::Finished(turn) => {
                self.finish_assistant_turn(turn, runtime)?;
            }
            BackendEvent::Failed(error) => {
                self.pending_request = false;

                if let Some(message) = self.conversation.messages.last_mut() {
                    if message.streaming && matches!(message.role, MessageRole::Assistant) {
                        message.role = MessageRole::Error;
                        message.streaming = false;
                        message.content = format!("Request failed: {error}");
                        let persisted = message.clone();
                        self.store
                            .append_message(self.conversation.session_id, &persisted)?;
                        self.last_notice = Some(error);
                        return Ok(());
                    }
                }

                let message = Message::new(MessageRole::Error, format!("Request failed: {error}"));
                self.conversation.push(message.clone());
                self.store
                    .append_message(self.conversation.session_id, &message)?;
                self.last_notice = Some(error);
            }
        }

        Ok(())
    }

    fn finish_assistant_turn(&mut self, turn: AssistantTurn, runtime: &Runtime) -> Result<()> {
        let mut persisted_message = None;

        if let Some(message) = self.conversation.messages.last_mut() {
            if message.streaming && matches!(message.role, MessageRole::Assistant) {
                message.content = turn.content.clone();
                message.reasoning = turn.reasoning.clone();
                message.tool_calls = turn.tool_calls.clone();
                message.streaming = false;
                persisted_message = Some(message.clone());
            }
        }

        if let Some(message) = persisted_message {
            self.store
                .append_message(self.conversation.session_id, &message)?;
        }

        if !self.mode.is_read_only() && !turn.tool_calls.is_empty() {
            self.last_notice = Some(format!("Running {} tool call(s)...", turn.tool_calls.len()));

            for tool_call in turn.tool_calls {
                let output = self
                    .tools
                    .execute_call(&tool_call)
                    .unwrap_or_else(|error| format!("Tool failed: {error}"));
                let message = Message::tool_result(tool_call.id, tool_call.name, output);
                self.conversation.push(message.clone());
                self.store
                    .append_message(self.conversation.session_id, &message)?;
            }

            self.start_assistant_turn(runtime)?;
            return Ok(());
        }

        self.pending_request = false;
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

struct TerminalSession;

impl TerminalSession {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw mode")?;
        crossterm::execute!(
            io::stdout(),
            EnterAlternateScreen,
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
            DisableMouseCapture,
            Show,
        );
    }
}
