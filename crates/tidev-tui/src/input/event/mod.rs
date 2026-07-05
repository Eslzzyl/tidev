use super::*;
use crossterm::event::{Event, KeyEventKind};

mod actions;
mod completion;
mod keyboard;
mod mouse;
mod panels;
mod request;
mod scroll;

impl App {
    pub(crate) fn handle_event(&mut self, event: Event) -> Result<()> {
        self.ui.dirty = true;
        match event {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.handle_key_event(key)?;
            }
            Event::Paste(text) => {
                if self.ui.model_panel.is_some() {
                    self.handle_model_panel_paste(&text)?;
                } else if text.is_empty() {
                    self.handle_clipboard_paste()?;
                } else {
                    self.handle_text_paste(&text)?;
                }
            }
            Event::Mouse(mouse) => {
                self.handle_mouse_event(mouse);
            }
            Event::Resize(_, _) => {
                self.clear_mouse_selection();
                self.ui.message_content_area = None;
                self.ui.sidebar_area = None;
                self.clear_message_render_cache();
            }
            Event::FocusGained => {
                log::debug!("Event::FocusGained received");
                self.ui.notifications.set_focused(true);
            }
            Event::FocusLost => {
                log::debug!("Event::FocusLost received");
                self.ui.notifications.set_focused(false);
            }
            _ => {}
        }

        Ok(())
    }
}
