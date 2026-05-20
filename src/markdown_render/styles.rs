use ratatui::style::Style;

pub(super) struct MarkdownStyles {
    pub(super) h1: Style,
    pub(super) h2: Style,
    pub(super) h3: Style,
    pub(super) h4: Style,
    pub(super) h5: Style,
    pub(super) h6: Style,
    pub(super) code: Style,
    pub(super) emphasis: Style,
    pub(super) strong: Style,
    pub(super) strikethrough: Style,
    pub(super) ordered_list_marker: Style,
    pub(super) unordered_list_marker: Style,
    pub(super) link: Style,
    pub(super) blockquote: Style,
    pub(super) rule: Style,
}

impl Default for MarkdownStyles {
    fn default() -> Self {
        Self {
            h1: Style::default().cyan().bold().underlined(),
            h2: Style::default().yellow().bold(),
            h3: Style::default().magenta().bold().italic(),
            h4: Style::default().blue().italic(),
            h5: Style::default().green().italic(),
            h6: Style::default().gray().italic(),
            code: Style::default().cyan(),
            emphasis: Style::default().italic(),
            strong: Style::default().bold(),
            strikethrough: Style::default().crossed_out(),
            ordered_list_marker: Style::default().light_blue(),
            unordered_list_marker: Style::default(),
            link: Style::default().cyan().underlined(),
            blockquote: Style::default().green(),
            rule: Style::default().dark_gray(),
        }
    }
}
