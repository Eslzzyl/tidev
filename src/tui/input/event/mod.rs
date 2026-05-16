use super::*;

mod actions;
mod completion;
mod keyboard;
mod mouse;
mod panels;
mod request;
mod scroll;

impl App {
    pub(crate) fn handle_event(&mut self, event: Event, runtime: &Runtime) -> Result<()> {
        self.dirty = true;
        match event {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.handle_key_event(key, runtime)?;
            }
            Event::Paste(text) => {
                if self.model_panel.is_some() {
                    self.handle_model_panel_paste(&text)?;
                } else {
                    self.handle_text_paste(&text)?;
                }
            }
            Event::Mouse(mouse) => {
                self.handle_mouse_event(mouse, runtime);
            }
            Event::Resize(_, _) => {
                self.clear_mouse_selection();
                self.message_content_area = None;
                self.sidebar_area = None;
                self.clear_message_render_cache();
            }
            Event::FocusGained => {
                crate::log_debug!("Event::FocusGained received");
                self.notifications.set_focused(true);
            }
            Event::FocusLost => {
                crate::log_debug!("Event::FocusLost received");
                self.notifications.set_focused(false);
            }
            _ => {}
        }

        Ok(())
    }
}
