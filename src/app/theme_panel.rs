use crate::theme::ThemeName;

#[derive(Clone, Debug)]
pub struct ThemePanelState {
    pub selected_index: usize,
    pub preview_theme: ThemeName,
    pub original_theme: ThemeName,
}

impl ThemePanelState {
    pub fn new(current: ThemeName) -> Self {
        let selected_index = Self::themes()
            .iter()
            .position(|theme| *theme == current)
            .unwrap_or(0);

        Self {
            selected_index,
            preview_theme: current,
            original_theme: current,
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
}
