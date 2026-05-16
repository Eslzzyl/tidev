use crate::theme::ThemeName;

/// An item in the theme panel's display list.
#[derive(Clone, Debug)]
pub enum DisplayItem {
    Header(&'static str),
    Theme(ThemeName),
}

/// Panel state for the theme selector.
///
/// Features:
/// - Search is always active — type to filter
/// - Light/Dark section grouping
/// - Live preview on selection
#[derive(Clone, Debug)]
pub struct ThemePanelState {
    /// Flat display list (headers + theme items, filtered by query).
    /// Rebuilt on every query change.
    pub display_items: Vec<DisplayItem>,
    /// Selected index into display_items.
    pub selected_index: usize,
    /// Currently previewed theme.
    pub preview_theme: ThemeName,
    /// Theme when the panel was opened (for cancel/restore).
    pub original_theme: ThemeName,
    /// Search query text.
    pub query: String,
}

impl ThemePanelState {
    pub fn new(current: ThemeName) -> Self {
        let display_items = Self::build_display("");
        let selected_index = display_items
            .iter()
            .position(|item| matches!(item, DisplayItem::Theme(t) if *t == current))
            .unwrap_or(0);

        Self {
            display_items,
            selected_index,
            preview_theme: current,
            original_theme: current,
            query: String::new(),
        }
    }

    /// Build the flat display list from all themes, filtered by `query`.
    fn build_display(query: &str) -> Vec<DisplayItem> {
        let all = ThemeName::all();
        let q = query.trim().to_lowercase();
        let matches_query = |t: &ThemeName| -> bool { q.is_empty() || t.as_str().contains(&q) };

        let mut items = Vec::new();

        // Light themes section
        let light: Vec<_> = all
            .iter()
            .filter(|t| !t.is_dark() && matches_query(t))
            .collect();
        if !light.is_empty() {
            items.push(DisplayItem::Header("Light"));
            for t in light {
                items.push(DisplayItem::Theme(*t));
            }
        }

        // Dark themes section
        let dark: Vec<_> = all
            .iter()
            .filter(|t| t.is_dark() && matches_query(t))
            .collect();
        if !dark.is_empty() {
            items.push(DisplayItem::Header("Dark"));
            for t in dark {
                items.push(DisplayItem::Theme(*t));
            }
        }

        // If query yields nothing, fall back to showing all
        if items.is_empty() {
            return Self::build_display("");
        }

        items
    }

    /// Rebuild the display list after a query change, preserving selection if possible.
    fn rebuild(&mut self) {
        let old_preview = self.preview_theme;
        self.display_items = Self::build_display(&self.query);
        self.selected_index = self
            .display_items
            .iter()
            .position(|item| matches!(item, DisplayItem::Theme(t) if *t == old_preview))
            .unwrap_or(0);
        if let Some(DisplayItem::Theme(t)) = self.display_items.get(self.selected_index) {
            self.preview_theme = *t;
        }
    }

    /// Move selection up, skipping section headers.
    pub fn move_up(&mut self) {
        let mut idx = self.selected_index;
        loop {
            if idx == 0 {
                return;
            }
            idx -= 1;
            if matches!(self.display_items[idx], DisplayItem::Theme(_)) {
                self.selected_index = idx;
                if let DisplayItem::Theme(t) = self.display_items[idx] {
                    self.preview_theme = t;
                }
                return;
            }
        }
    }

    /// Move selection down, skipping section headers.
    pub fn move_down(&mut self) {
        let len = self.display_items.len();
        let mut idx = self.selected_index;
        loop {
            if idx + 1 >= len {
                return;
            }
            idx += 1;
            if matches!(self.display_items[idx], DisplayItem::Theme(_)) {
                self.selected_index = idx;
                if let DisplayItem::Theme(t) = self.display_items[idx] {
                    self.preview_theme = t;
                }
                return;
            }
        }
    }

    /// Append a char to the search query and refilter.
    pub fn append_query(&mut self, ch: char) {
        self.query.push(ch);
        self.rebuild();
    }

    /// Remove last char from query.
    pub fn backspace_query(&mut self) {
        if !self.query.is_empty() {
            self.query.pop();
            self.rebuild();
        }
    }
}
