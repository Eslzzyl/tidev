//! OverlayStack — a z-ordered stack of overlay components.

use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::action::Action;
use crate::component::Component;
use crate::context::{DrawContext, UpdateContext};

pub(crate) struct OverlayStack {
    overlays: Vec<Box<dyn Component>>,
}

impl OverlayStack {
    pub fn new() -> Self {
        Self {
            overlays: Vec::new(),
        }
    }

    pub fn push(&mut self, component: Box<dyn Component>) {
        self.overlays.push(component);
    }

    pub fn last_mut(&mut self) -> Option<&mut Box<dyn Component>> {
        self.overlays.last_mut()
    }

    pub fn pop(&mut self) -> Option<Box<dyn Component>> {
        self.overlays.pop()
    }

    pub fn is_empty(&self) -> bool {
        self.overlays.is_empty()
    }

    /// Route a key event top-first.
    pub fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        for overlay in self.overlays.iter_mut().rev() {
            if let Some(action) = overlay.handle_key_event(key) {
                return Some(action);
            }
            if overlay.blocks_input() {
                return Some(Action::Noop);
            }
        }
        None
    }

    /// Route a paste event to the topmost overlay.
    pub fn handle_paste(&mut self, text: &str) -> Option<Action> {
        for overlay in self.overlays.iter_mut().rev() {
            if let Some(action) = overlay.handle_paste(text) {
                return Some(action);
            }
            if overlay.blocks_input() {
                return Some(Action::Noop);
            }
        }
        None
    }

    /// Broadcast an Action to all overlays.
    pub fn update_all(&mut self, action: &Action, ctx: &UpdateContext) -> Vec<Action> {
        let mut follow_ups = Vec::new();
        for overlay in self.overlays.iter_mut() {
            follow_ups.extend(overlay.update(action, ctx));
        }
        follow_ups
    }

    /// Route a mouse event top-first.
    pub fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        for overlay in self.overlays.iter_mut().rev() {
            if let Some(action) = overlay.handle_mouse_event(mouse, area) {
                return Some(action);
            }
            if overlay.blocks_input() {
                return Some(Action::Noop);
            }
        }
        None
    }

    /// Draw overlays bottom-up.
    ///
    /// Each overlay is drawn in its appropriate area:
    /// - overlays that use the main area (e.g. composer extensions)
    ///   get `main_area` so they don't spill over the sidebar;
    /// - all other overlays (centered panels) get `full_area` so they
    ///   occupy the full terminal width including the sidebar.
    pub fn draw(&mut self, frame: &mut Frame, full_area: Rect, main_area: Rect, ctx: &DrawContext) {
        for overlay in self.overlays.iter_mut() {
            let area = if overlay.overlay_uses_main_area() {
                main_area
            } else {
                full_area
            };
            overlay.draw(frame, area, ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::HashSet;
    use std::path::Path;
    use std::path::PathBuf;
    use std::rc::Rc;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use tidev_config::AppConfig;
    use tidev_config::auth::{ActiveModel, AuthStore};
    use tidev_config::types::ApiType;
    use tidev_core::Mode as SessionMode;
    use tidev_llm::reasoning::ThinkingLevelType;
    use uuid::Uuid;

    use crate::components::overlays::agents::AgentsPanel;
    use crate::components::overlays::connect::ConnectDialog;
    use crate::components::overlays::fork::ForkConfirmDialog;
    use crate::components::overlays::image::ImageViewer;
    use crate::components::overlays::message::MessagePanel;
    use crate::components::overlays::model::ModelPanel;
    use crate::components::overlays::panel_launcher::PanelLauncher;
    use crate::components::overlays::question::QuestionDialog;
    use crate::components::overlays::rename::RenameDialog;
    use crate::components::overlays::search::SearchPanel;
    use crate::components::overlays::sensitive::SensitiveFileDialog;
    use crate::components::overlays::session::{SessionPanel, SessionViewMode};
    use crate::components::overlays::settings::SettingsPanel;
    use crate::components::overlays::skills::SkillsPanel;
    use crate::components::overlays::theme::ThemePanel;
    use crate::components::overlays::undo::UndoConfirmDialog;
    use crate::components::overlays::workspace::WorkspaceBoundaryDialog;
    use crate::theme::ThemePalette;

    // ---------------------------------------------------------------------------
    // Mock overlay for routing tests
    // ---------------------------------------------------------------------------

    struct MockOverlay {
        uses_main_area: bool,
        captured: Rc<Cell<Option<Rect>>>,
    }

    impl Component for MockOverlay {
        fn draw(&mut self, _frame: &mut Frame, rect: Rect, _ctx: &DrawContext) {
            self.captured.set(Some(rect));
        }

        fn overlay_uses_main_area(&self) -> bool {
            self.uses_main_area
        }
    }

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    fn test_palette() -> ThemePalette {
        ThemePalette {
            is_dark: true,
            background: Color::Rgb(0, 0, 0),
            panel: Color::Rgb(10, 10, 10),
            panel_alt: Color::Rgb(20, 20, 20),
            panel_light: Color::Rgb(30, 30, 30),
            text: Color::Rgb(255, 255, 255),
            muted: Color::Rgb(128, 128, 128),
            border: Color::Rgb(64, 64, 64),
            accent: Color::Rgb(0, 200, 200),
            accent_soft: Color::Rgb(100, 150, 150),
            success: Color::Rgb(0, 200, 0),
            warning: Color::Rgb(200, 200, 0),
            error: Color::Rgb(200, 0, 0),
            diff_add: Color::Rgb(0, 200, 0),
            diff_delete: Color::Rgb(200, 0, 0),
            selection_bg: Color::Rgb(0, 200, 200),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(0, 200, 200),
            mode_plan: Color::Rgb(100, 150, 150),
        }
    }

    fn test_draw_ctx(workspace_root: &Path) -> DrawContext<'_> {
        DrawContext {
            palette: test_palette(),
            focused: true,
            mode: SessionMode::Build,
            pending_mode: None,
            model_display: None,
            provider_display: None,
            thinking_level: None,
            subagent_disabled: false,
            collapse_thinking: false,
            collapse_diffs: false,
            workspace_root,
        }
    }

    // ---------------------------------------------------------------------------
    // Part 1: Component trait default
    // ---------------------------------------------------------------------------

    #[test]
    fn default_overlay_uses_main_area_returns_false() {
        struct NoOverride;

        impl Component for NoOverride {
            fn draw(&mut self, _frame: &mut Frame, _rect: Rect, _ctx: &DrawContext) {}
        }

        assert!(!NoOverride.overlay_uses_main_area());
    }

    // ---------------------------------------------------------------------------
    // Part 2: OverlayStack routing
    // ---------------------------------------------------------------------------

    #[test]
    fn draw_passes_main_area_to_overlay_that_uses_it() {
        let captured = Rc::new(Cell::new(None));
        let overlay = MockOverlay {
            uses_main_area: true,
            captured: captured.clone(),
        };

        let mut stack = OverlayStack::new();
        stack.push(Box::new(overlay));

        let backend = TestBackend::new(100, 50);
        let mut terminal = Terminal::new(backend).unwrap();

        let full_area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 50,
        };
        let main_area = Rect {
            x: 0,
            y: 0,
            width: 70,
            height: 50,
        };
        let ctx = test_draw_ctx(Path::new("/tmp"));

        terminal
            .draw(|frame| {
                stack.draw(frame, full_area, main_area, &ctx);
            })
            .unwrap();

        assert_eq!(captured.get(), Some(main_area));
    }

    #[test]
    fn draw_passes_full_area_to_overlay_that_does_not_use_main_area() {
        let captured = Rc::new(Cell::new(None));
        let overlay = MockOverlay {
            uses_main_area: false,
            captured: captured.clone(),
        };

        let mut stack = OverlayStack::new();
        stack.push(Box::new(overlay));

        let backend = TestBackend::new(100, 50);
        let mut terminal = Terminal::new(backend).unwrap();

        let full_area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 50,
        };
        let main_area = Rect {
            x: 0,
            y: 0,
            width: 70,
            height: 50,
        };
        let ctx = test_draw_ctx(Path::new("/tmp"));

        terminal
            .draw(|frame| {
                stack.draw(frame, full_area, main_area, &ctx);
            })
            .unwrap();

        assert_eq!(captured.get(), Some(full_area));
    }

    #[test]
    fn draw_routes_multiple_overlays_individually() {
        let captured_a = Rc::new(Cell::new(None));
        let captured_b = Rc::new(Cell::new(None));

        let overlay_a = MockOverlay {
            uses_main_area: true,
            captured: captured_a.clone(),
        };
        let overlay_b = MockOverlay {
            uses_main_area: false,
            captured: captured_b.clone(),
        };

        let mut stack = OverlayStack::new();
        stack.push(Box::new(overlay_a));
        stack.push(Box::new(overlay_b));

        let backend = TestBackend::new(100, 50);
        let mut terminal = Terminal::new(backend).unwrap();

        let full_area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 50,
        };
        let main_area = Rect {
            x: 10,
            y: 5,
            width: 70,
            height: 40,
        };
        let ctx = test_draw_ctx(Path::new("/tmp"));

        terminal
            .draw(|frame| {
                stack.draw(frame, full_area, main_area, &ctx);
            })
            .unwrap();

        assert_eq!(captured_a.get(), Some(main_area));
        assert_eq!(captured_b.get(), Some(full_area));
    }

    #[test]
    fn draw_empty_stack_does_not_panic() {
        let mut stack = OverlayStack::new();
        let backend = TestBackend::new(100, 50);
        let mut terminal = Terminal::new(backend).unwrap();
        let full_area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 50,
        };
        let main_area = Rect {
            x: 0,
            y: 0,
            width: 70,
            height: 50,
        };
        let ctx = test_draw_ctx(Path::new("/tmp"));

        terminal
            .draw(|frame| {
                stack.draw(frame, full_area, main_area, &ctx);
            })
            .unwrap();
        // no panic = pass
    }

    // ---------------------------------------------------------------------------
    // Part 3a: Overlay components that use main_area (return true)
    // ---------------------------------------------------------------------------

    #[test]
    fn question_dialog_uses_main_area() {
        let dialog = QuestionDialog::new(vec![]);
        assert!(dialog.overlay_uses_main_area());
    }

    #[test]
    fn sensitive_file_dialog_uses_main_area() {
        let dialog = SensitiveFileDialog::new(PathBuf::from("/f"), PathBuf::from("/w"), 0, 1);
        assert!(dialog.overlay_uses_main_area());
    }

    #[test]
    fn workspace_boundary_dialog_uses_main_area() {
        let dialog = WorkspaceBoundaryDialog::new(PathBuf::from("/f"), PathBuf::from("/w"), 0, 1);
        assert!(dialog.overlay_uses_main_area());
    }

    // ---------------------------------------------------------------------------
    // Part 3b: Overlay components that use full_area (return false)
    // ---------------------------------------------------------------------------

    #[test]
    fn session_panel_does_not_use_main_area() {
        let panel = SessionPanel::new(
            vec![],
            SessionViewMode::CurrentWorkspace,
            Uuid::nil(),
            HashSet::new(),
        );
        assert!(!panel.overlay_uses_main_area());
    }

    #[test]
    fn message_panel_does_not_use_main_area() {
        let panel = MessagePanel::new(vec![]);
        assert!(!panel.overlay_uses_main_area());
    }

    #[test]
    fn settings_panel_does_not_use_main_area() {
        let panel = SettingsPanel::new(&AppConfig::default());
        assert!(!panel.overlay_uses_main_area());
    }

    #[test]
    fn theme_panel_does_not_use_main_area() {
        let catalog = tidev_config::ThemeCatalog::load(std::path::Path::new("/nonexistent"))
            .expect("bundled themes parse");
        let panel = ThemePanel::new(catalog, "dark".to_string());
        assert!(!panel.overlay_uses_main_area());
    }

    #[test]
    fn skills_panel_does_not_use_main_area() {
        let panel = SkillsPanel::new(vec![]);
        assert!(!panel.overlay_uses_main_area());
    }

    #[test]
    fn search_panel_does_not_use_main_area() {
        let panel = SearchPanel::new("test", &AuthStore::default());
        assert!(!panel.overlay_uses_main_area());
    }

    #[test]
    fn model_panel_does_not_use_main_area() {
        let active_model = ActiveModel {
            provider_id: "test".into(),
            provider_display_name: "Test".into(),
            base_url: "https://test.com".into(),
            api_type: ApiType::OpenAiChatCompletions,
            model_id: "test-model".into(),
            request_model_id: "test-model".into(),
            display_name: "Test Model".into(),
            context_window: 4096,
            max_output_tokens: 1024,
            temperature: None,
            supports_images: false,
            supports_parallel_tool_calls: true,
            system_prompt: String::new(),
            api_key: None,
            extra_body: None,
            thinking_level: ThinkingLevelType::default(),
        };
        let panel = ModelPanel::new(vec![], vec![], active_model);
        assert!(!panel.overlay_uses_main_area());
    }

    #[test]
    fn agents_panel_does_not_use_main_area() {
        let panel = AgentsPanel::new();
        assert!(!panel.overlay_uses_main_area());
    }

    #[test]
    fn connect_dialog_does_not_use_main_area() {
        let dialog = ConnectDialog::new();
        assert!(!dialog.overlay_uses_main_area());
    }

    #[test]
    fn fork_confirm_dialog_does_not_use_main_area() {
        let dialog = ForkConfirmDialog::new(Uuid::nil(), 0);
        assert!(!dialog.overlay_uses_main_area());
    }

    #[test]
    fn undo_confirm_dialog_does_not_use_main_area() {
        let dialog = UndoConfirmDialog::new(Uuid::nil(), String::new());
        assert!(!dialog.overlay_uses_main_area());
    }

    #[test]
    fn rename_dialog_does_not_use_main_area() {
        let dialog = RenameDialog::new(Uuid::nil(), String::new());
        assert!(!dialog.overlay_uses_main_area());
    }

    #[test]
    fn panel_launcher_does_not_use_main_area() {
        let launcher = PanelLauncher::new();
        assert!(!launcher.overlay_uses_main_area());
    }

    #[test]
    fn image_viewer_does_not_use_main_area() {
        // Create a minimal 1x1 black PNG in-memory.
        let png_bytes = make_test_png();
        let viewer = ImageViewer::from_raw(png_bytes, "test.png".into(), None).unwrap();
        assert!(!viewer.overlay_uses_main_area());
    }

    /// Produce a valid 1×1 RGB PNG in memory.
    fn make_test_png() -> Vec<u8> {
        use image::ExtendedColorType;
        use image::ImageEncoder;
        use image::codecs::png::PngEncoder;
        let mut buf = std::io::Cursor::new(Vec::new());
        let encoder = PngEncoder::new(&mut buf);
        // One black pixel (R=0, G=0, B=0).
        encoder
            .write_image(&[0, 0, 0], 1, 1, ExtendedColorType::Rgb8)
            .unwrap();
        buf.into_inner()
    }
}
