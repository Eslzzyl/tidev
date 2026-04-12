use crate::theme::ThemeName;

#[derive(Clone, Debug)]
pub struct ThemePanelState {
    pub visible: bool,
    pub selected_index: usize,
    pub preview_theme: ThemeName,
    pub confirmed_theme: ThemeName,
}

impl ThemePanelState {
    pub fn new(current: ThemeName) -> Self {
        Self {
            visible: true,
            selected_index: 0,
            preview_theme: current,
            confirmed_theme: current,
        }
    }

    pub fn themes() -> &'static [ThemeName] {
        &[ThemeName::Dark, ThemeName::Light]
    }

    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.preview_theme = Self::themes()[self.selected_index];
        }
    }

    pub fn move_down(&mut self) {
        let len = Self::themes().len();
        if self.selected_index < len - 1 {
            self.selected_index += 1;
            self.preview_theme = Self::themes()[self.selected_index];
        }
    }

    pub fn apply(&mut self) {
        self.confirmed_theme = self.preview_theme;
    }

    pub fn cancel(&mut self) {
        self.preview_theme = self.confirmed_theme;
    }
}