use super::*;

impl App {
    pub(crate) fn can_scroll_conversation(&self) -> bool {
        self.ui.screen == Screen::Chat
            && self.ui.permission_dialog.is_none()
            && self.ui.fork_confirm_dialog.is_none()
            && self.ui.connect_dialog.is_none()
            && self.ui.theme_panel.is_none()
            && self.ui.model_panel.is_none()
            && self.ui.agents_panel.is_none()
            && !self.ui.command_palette.visible
    }

    pub(crate) fn scroll_messages_to_bottom(&mut self) {
        self.clear_mouse_selection();
        self.ui.message_scroll_offset = 0;
        self.ui.message_follow_tail = true;
    }

    pub(crate) fn message_scroll_max(&self) -> usize {
        self.ui.message_total_lines
            .saturating_sub(self.ui.message_viewport_lines)
    }

    pub(crate) fn message_scroll_page(&self) -> usize {
        self.ui.message_viewport_lines.saturating_sub(1).max(1)
    }

    pub(crate) fn scroll_messages_up(&mut self, lines: usize) {
        self.clear_mouse_selection();
        self.scroll_messages_up_internal(lines);
    }

    pub(crate) fn scroll_messages_up_internal(&mut self, lines: usize) {
        let max_scroll = self.message_scroll_max();
        let current = if self.ui.message_follow_tail {
            max_scroll
        } else {
            self.ui.message_scroll_offset.min(max_scroll)
        };

        self.ui.message_scroll_offset = current.saturating_sub(lines);
        self.ui.message_follow_tail = self.ui.message_scroll_offset >= max_scroll;
    }

    pub(crate) fn scroll_messages_down(&mut self, lines: usize) {
        self.clear_mouse_selection();
        self.scroll_messages_down_internal(lines);
    }

    pub(crate) fn scroll_messages_down_internal(&mut self, lines: usize) {
        let max_scroll = self.message_scroll_max();
        let current = if self.ui.message_follow_tail {
            max_scroll
        } else {
            self.ui.message_scroll_offset.min(max_scroll)
        };

        self.ui.message_scroll_offset = current.saturating_add(lines).min(max_scroll);
        self.ui.message_follow_tail = self.ui.message_scroll_offset >= max_scroll;
    }

    fn sidebar_scroll_max(&self) -> usize {
        if let Some(area) = self.ui.sidebar_area {
            let viewport = area.height.saturating_sub(2) as usize;
            self.ui.sidebar_total_lines.saturating_sub(viewport)
        } else {
            0
        }
    }

    fn scroll_sidebar_up(&mut self, lines: usize) {
        self.ui.sidebar_scroll_offset = self.ui.sidebar_scroll_offset.saturating_sub(lines);
    }

    fn scroll_sidebar_down(&mut self, lines: usize) {
        let max_scroll = self.sidebar_scroll_max();
        self.ui.sidebar_scroll_offset = self.ui.sidebar_scroll_offset
            .saturating_add(lines)
            .min(max_scroll);
    }

    pub(crate) fn handle_sidebar_scroll_up(&mut self, position: Position) -> bool {
        if let Some(area) = self.ui.sidebar_area
            && area.contains(position)
        {
            if self.ui.sidebar_scroll_offset > 0 {
                let speed = self.runtime.config().ui.scroll_speed as usize;
                self.scroll_sidebar_up(speed);
            }
            true // Always consume scroll event when in sidebar area
        } else {
            false
        }
    }

    pub(crate) fn handle_sidebar_scroll_down(&mut self, position: Position) -> bool {
        if let Some(area) = self.ui.sidebar_area
            && area.contains(position)
        {
            let max_scroll = self.sidebar_scroll_max();
            if self.ui.sidebar_scroll_offset < max_scroll {
                let speed = self.runtime.config().ui.scroll_speed as usize;
                self.scroll_sidebar_down(speed);
            }
            true // Always consume scroll event when in sidebar area
        } else {
            false
        }
    }

    pub(crate) fn handle_message_scroll_key(&mut self, key: KeyEvent) -> bool {
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

    pub(crate) fn scroll_messages_to_message(&mut self, message_id: Uuid) {
        self.ui.message_scroll_target = Some(message_id);
        self.ui.message_follow_tail = false;
    }
}
