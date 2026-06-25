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
                } else if text.is_empty() {
                    // Empty bracketed paste — the terminal may have intercepted
                    // Ctrl+V for a paste that contains only image data (no text).
                    // Fall back to reading the clipboard directly via arboard,
                    // which can detect and handle image content.
                    self.handle_clipboard_paste()?;
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
                log::debug!("Event::FocusGained received");
                self.notifications.set_focused(true);
            }
            Event::FocusLost => {
                log::debug!("Event::FocusLost received");
                self.notifications.set_focused(false);
            }
            _ => {}
        }

        Ok(())
    }
}
