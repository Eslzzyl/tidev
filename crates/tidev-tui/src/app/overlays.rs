use super::*;

use crate::theme::ThemeName;
use tidev_core::agent_type::AgentType;

use crate::action::{Action, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::components::overlays::agents::AgentsPanel;
use crate::components::overlays::connect::ConnectDialog;
use crate::components::overlays::fork::ForkConfirmDialog;
use crate::components::overlays::image::ImageViewer;
use crate::components::overlays::mcp::McpServerPanel;

use crate::components::overlays::message::{MessagePanel, MessagePanelMessage};
use crate::components::overlays::model::ModelPanel;
use crate::components::overlays::panel_launcher::PanelLauncher;
use crate::components::overlays::rename::RenameDialog;
use crate::components::overlays::search::SearchPanel;
use crate::components::overlays::session::SessionPanel;
use crate::components::overlays::settings::SettingsPanel;
use crate::components::overlays::skills::{SkillItem, SkillsPanel};
use crate::components::overlays::theme::ThemePanel;
use crate::components::overlays::undo::UndoConfirmDialog;
use crate::context::{InitContext, UpdateContext};
use crate::utils::strip_system_reminder_tags;

impl App {
    pub(crate) fn open_overlay(&mut self, kind: OverlayKind) {
        let kind_for_update = kind.clone();
        let component: Option<Box<dyn Component>> = match kind {
            OverlayKind::ThemePanel => {
                let current =
                    ThemeName::parse(self.current_palette.name.as_str()).unwrap_or(ThemeName::Dark);
                Some(Box::new(ThemePanel::new(current)))
            }
            OverlayKind::AgentsPanel => Some(Box::new(AgentsPanel::new())),
            OverlayKind::SkillsPanel => {
                let catalog = &self.runtime.skills;
                let skills: Vec<SkillItem> = catalog
                    .all()
                    .iter()
                    .map(|s| SkillItem {
                        name: s.name.clone(),
                        content: catalog.render_skill(&s.name).unwrap_or_default(),
                        is_bundled: s.directory.starts_with("__builtin__"),
                    })
                    .collect();
                Some(Box::new(SkillsPanel::new(skills)))
            }
            OverlayKind::SettingsPanel => {
                let config = self.runtime.config();
                Some(Box::new(SettingsPanel::new(&config)))
            }
            OverlayKind::McpServerPanel => {
                let mcp = self.runtime.tool_registry.mcp_manager();
                Some(Box::new(McpServerPanel::new(mcp)))
            }
            OverlayKind::SearchPanel => {
                let config = self.runtime.config();
                let auth = self.runtime.auth();
                Some(Box::new(SearchPanel::new(
                    &config.websearch.default_provider,
                    &auth,
                )))
            }
            OverlayKind::MessagePanel => {
                let messages = self
                    .message_list
                    .as_ref()
                    .and_then(|ml| ml.active_chat_context())
                    .map(|ctx| {
                        ctx.visible_messages()
                            .iter()
                            .filter(|m| matches!(m.role, tidev_llm::message::MessageRole::User))
                            .enumerate()
                            .map(|(i, m)| MessagePanelMessage {
                                message_id: m.id,
                                content: strip_system_reminder_tags(&m.content),
                                created_at: m.created_at,
                                mode: ctx
                                    .app_data(m.id)
                                    .and_then(|data| data.mode.as_deref())
                                    .and_then(|value| value.parse::<tidev_core::Mode>().ok()),
                                original_index: i,
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                Some(Box::new(MessagePanel::new(messages)))
            }
            OverlayKind::ModelPanel => {
                use crate::components::overlays::model::ModelPanelTab;
                let config = self.runtime.config();
                let auth = self.runtime.auth();
                let active_model = match self.runtime.resolve_active_model() {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("Failed to resolve active model: {e}");
                        return;
                    }
                };

                let mut tabs = vec![ModelPanelTab::new(
                    "general",
                    "General",
                    &active_model.label(),
                )];
                for agent_type in AgentType::all() {
                    if *agent_type == AgentType::General {
                        continue;
                    }
                    let ty = agent_type.display_name();
                    let label = config.agent_model_display(ty);
                    tabs.push(ModelPanelTab::new(ty, agent_type.display_name(), &label));
                }

                let connected_models = config.connected_models(&auth);
                Some(Box::new(ModelPanel::new(
                    tabs,
                    connected_models,
                    active_model,
                )))
            }
            OverlayKind::SessionPanel => {
                use crate::components::overlays::session::SessionViewMode;
                let store = self.runtime.session_manager().store();
                let workspace_root = self.runtime.workspace_root().display().to_string();
                let sessions = store
                    .list_sessions_for_workspace(&workspace_root, 1000, 0)
                    .unwrap_or_default();
                let current_session_id = self
                    .current_session_id
                    .or_else(|| {
                        self.message_list
                            .as_ref()
                            .and_then(|ml| ml.active_chat_context())
                            .map(|ctx| ctx.session_id)
                    })
                    .unwrap_or(uuid::Uuid::nil());
                Some(Box::new(SessionPanel::new(
                    sessions,
                    SessionViewMode::CurrentWorkspace,
                    current_session_id,
                    self.active_sessions.clone(),
                )))
            }
            OverlayKind::ForkConfirmDialog {
                message_id,
                message_count,
            } => Some(Box::new(ForkConfirmDialog::new(message_id, message_count))),
            OverlayKind::UndoConfirmDialog {
                message_id,
                content,
            } => Some(Box::new(UndoConfirmDialog::new(message_id, content))),
            OverlayKind::RenameDialog => {
                let session_id = self.current_session_id.unwrap_or(uuid::Uuid::nil());
                let title = self
                    .runtime
                    .session_manager()
                    .load_session(session_id)
                    .ok()
                    .flatten()
                    .map(|r| r.title)
                    .unwrap_or_default();
                Some(Box::new(RenameDialog::new(session_id, title)))
            }
            OverlayKind::ImageViewer { data, filename } => {
                ImageViewer::from_raw(data, filename, self.image_picker.clone())
                    .map(|v| Box::new(v) as Box<dyn Component>)
            }
            OverlayKind::ConnectDialog => Some(Box::new(ConnectDialog::new())),
            OverlayKind::PanelLauncher => Some(Box::new(PanelLauncher::new())),
            // Permission/security dialogs are triggered by handle_tui_request,
            // not by user keystrokes. These branches exist as fallback placeholders.
            OverlayKind::QuestionDialog
            | OverlayKind::WorkspaceBoundaryDialog
            | OverlayKind::SensitiveFileDialog => None,
        };
        if let Some(mut component) = component {
            let config = self.runtime.config();
            let auth = self.runtime.auth();
            let init_ctx = InitContext {
                config: &config,
                auth: &auth,
            };
            let _ = component.init(&init_ctx);
            self.overlays.push(component);

            // Trigger initial lazy-load for the new overlay (e.g. populate preview cache)
            if let Some(top) = self.overlays.last_mut() {
                let ctx = UpdateContext {
                    runtime: &mut self.runtime,
                };
                let _ = top.update(&Action::Overlay(OverlayAction::Open(kind_for_update)), &ctx);
            }
        }
    }

    pub(crate) fn close_overlay(&mut self, kind: OverlayKind, queue: &mut Vec<Action>) {
        if let Some(mut overlay) = self.overlays.pop() {
            let ctx = UpdateContext {
                runtime: &mut self.runtime,
            };
            queue.extend(overlay.update(&Action::Overlay(OverlayAction::Close(kind)), &ctx));
        }
    }
}
