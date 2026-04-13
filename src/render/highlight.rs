use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use std::sync::OnceLock;
use std::sync::RwLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::Color as SyntectColor;
use syntect::highlighting::FontStyle;
use syntect::highlighting::Style as SyntectStyle;
use syntect::highlighting::Theme;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;
const MAX_HIGHLIGHT_LINES: usize = 10_000;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();
static THEME: OnceLock<RwLock<Theme>> = OnceLock::new();

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

fn default_theme() -> Theme {
    let themes = &theme_set().themes;
    themes
        .get("base16-ocean.dark")
        .or_else(|| themes.get("InspiredGitHub"))
        .or_else(|| themes.values().next())
        .cloned()
        .unwrap_or_default()
}

fn theme_lock() -> &'static RwLock<Theme> {
    THEME.get_or_init(|| RwLock::new(default_theme()))
}

#[allow(dead_code)]
pub(crate) fn set_syntax_theme(theme: Theme) {
    let mut guard = match theme_lock().write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = theme;
}

pub(crate) fn current_syntax_theme() -> Theme {
    match theme_lock().read() {
        Ok(theme) => theme.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

pub(crate) fn highlight_code_to_lines(code: &str, lang: &str) -> Vec<Line<'static>> {
    if let Some(lines) = highlight_to_spans(code, lang) {
        return lines.into_iter().map(Line::from).collect();
    }

    let mut out: Vec<Line<'static>> = code
        .lines()
        .map(|line| Line::from(line.to_string()))
        .collect();
    if out.is_empty() {
        out.push(Line::from(String::new()));
    }
    out
}

#[allow(dead_code)]
pub(crate) fn highlight_code_to_styled_spans(
    code: &str,
    lang: &str,
) -> Option<Vec<Vec<Span<'static>>>> {
    highlight_to_spans(code, lang)
}

fn highlight_to_spans(code: &str, lang: &str) -> Option<Vec<Vec<Span<'static>>>> {
    if code.len() > MAX_HIGHLIGHT_BYTES || code.lines().count() > MAX_HIGHLIGHT_LINES {
        return None;
    }

    let syntax = syntax_set()
        .find_syntax_by_token(lang)
        .or_else(|| syntax_set().find_syntax_by_name(lang))
        .unwrap_or_else(|| syntax_set().find_syntax_plain_text());
    let theme = current_syntax_theme();
    let mut highlighter = HighlightLines::new(syntax, &theme);

    let mut out = Vec::new();
    let mut saw_any_line = false;

    for raw_line in code.split_inclusive('\n') {
        saw_any_line = true;
        let normalized = raw_line.trim_end_matches('\n').trim_end_matches('\r');
        let ranges = highlighter.highlight_line(normalized, syntax_set()).ok()?;
        out.push(
            ranges
                .into_iter()
                .map(|(style, text)| Span::styled(text.to_string(), convert_style(style)))
                .collect(),
        );
    }

    if !saw_any_line {
        out.push(Vec::new());
    }

    Some(out)
}

fn convert_style(style: SyntectStyle) -> Style {
    let mut out = Style::default();

    if let Some(color) = convert_color(style.foreground) {
        out = out.fg(color);
    }

    if style.font_style.contains(FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        out = out.add_modifier(Modifier::UNDERLINED);
    }

    out
}

fn convert_color(color: SyntectColor) -> Option<Color> {
    match color.a {
        1 => None,
        0 => Some(ansi_palette_color(color.r)),
        _ => Some(Color::Rgb(color.r, color.g, color.b)),
    }
}

fn ansi_palette_color(index: u8) -> Color {
    match index {
        0x00 => Color::Black,
        0x01 => Color::Red,
        0x02 => Color::Green,
        0x03 => Color::Yellow,
        0x04 => Color::Blue,
        0x05 => Color::Magenta,
        0x06 => Color::Cyan,
        0x07 => Color::Gray,
        0x08 => Color::DarkGray,
        0x09 => Color::Red,
        0x0a => Color::Green,
        0x0b => Color::Yellow,
        0x0c => Color::Blue,
        0x0d => Color::Magenta,
        0x0e => Color::Cyan,
        0x0f => Color::White,
        other => Color::Indexed(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_known_language() {
        let lines = highlight_code_to_lines("fn main() {}", "rust");
        assert_eq!(lines.len(), 1);
        let rendered = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(rendered, "fn main() {}");
    }

    #[test]
    fn unknown_language_falls_back_to_plain_text() {
        let lines = highlight_code_to_lines("hello world\n", "xyzlang");
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert_eq!(rendered, vec!["hello world"]);
    }
}
