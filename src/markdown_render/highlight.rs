use lru::LruCache;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use syntect::easy::HighlightLines;
use syntect::highlighting::Color as SyntectColor;
use syntect::highlighting::FontStyle;
use syntect::highlighting::Style as SyntectStyle;
use syntect::highlighting::Theme;
use syntect::highlighting::ThemeSet;
use syntect::parsing::{SyntaxReference, SyntaxSet};

use crate::theme::ThemeName;
const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;
const MAX_HIGHLIGHT_LINES: usize = 10_000;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();
static THEME: OnceLock<RwLock<Theme>> = OnceLock::new();
static HIGHLIGHT_CACHE: OnceLock<RwLock<LruCache<HighlightCacheKey, Vec<Line<'static>>>>> =
    OnceLock::new();
static HIGHLIGHT_CACHE_GEN: AtomicU64 = AtomicU64::new(0);

static THEME_SET_LOADING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct HighlightCacheKey {
    theme_gen: u64,
    lang: String,
    code_hash: [u8; 32],
}

pub fn spawn_background_load() {
    if THEME_SET_LOADING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        thread::spawn(|| {
            let set = ThemeSet::load_defaults();
            let _ = THEME_SET.set(set);
        });
    }
}

fn highlight_cache() -> &'static RwLock<LruCache<HighlightCacheKey, Vec<Line<'static>>>> {
    HIGHLIGHT_CACHE.get_or_init(|| {
        RwLock::new(LruCache::new(
            NonZeroUsize::new(100).expect("cache size must be non-zero"),
        ))
    })
}

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(two_face::syntax::extra_newlines)
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

fn theme_name_to_syntax_theme(name: ThemeName) -> Theme {
    let themes = &theme_set().themes;
    let theme_key = match name {
        ThemeName::Dark => "base16-ocean.dark",
        ThemeName::Light => "InspiredGitHub",
        ThemeName::Nord => "base16-ocean.dark",
        ThemeName::OneDark => "base16-ocean.dark",
        ThemeName::Catppuccin => "base16-mocha.dark",
        ThemeName::Solarized => "Solarized (dark)",
        ThemeName::Orng => "InspiredGitHub",
        ThemeName::Github => "InspiredGitHub",
        ThemeName::Material => "InspiredGitHub",
    };
    themes.get(theme_key).cloned().unwrap_or_else(default_theme)
}

pub fn set_syntax_theme_by_name(name: ThemeName) {
    let theme = theme_name_to_syntax_theme(name);
    set_syntax_theme(theme);
}

fn theme_lock() -> &'static RwLock<Theme> {
    THEME.get_or_init(|| RwLock::new(default_theme()))
}

#[allow(dead_code)]
pub(crate) fn set_syntax_theme(theme: Theme) {
    {
        let mut guard = match theme_lock().write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = theme;
    }
    HIGHLIGHT_CACHE_GEN.fetch_add(1, Ordering::SeqCst);
}

pub(crate) fn current_syntax_theme() -> Theme {
    match theme_lock().read() {
        Ok(theme) => theme.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

pub(crate) fn highlight_code_to_lines(code: &str, lang: &str) -> Vec<Line<'static>> {
    if code.len() > MAX_HIGHLIGHT_BYTES || code.lines().count() > MAX_HIGHLIGHT_LINES {
        return code_to_plain_lines(code);
    }

    let theme_gen = HIGHLIGHT_CACHE_GEN.load(Ordering::SeqCst);
    let code_hash = blake3::hash(code.as_bytes());
    let key = HighlightCacheKey {
        theme_gen,
        lang: lang.to_string(),
        code_hash: *code_hash.as_bytes(),
    };

    if let Ok(mut cache) = highlight_cache().write()
        && let Some(cached) = cache.get(&key)
    {
        return cached.clone();
    }

    let lines = if let Some(spans) = highlight_to_spans(code, lang) {
        spans.into_iter().map(Line::from).collect()
    } else {
        code_to_plain_lines(code)
    };

    if let Ok(mut cache) = highlight_cache().write() {
        cache.put(key, lines.clone());
    }

    lines
}

fn code_to_plain_lines(code: &str) -> Vec<Line<'static>> {
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

pub(crate) fn highlight_code_to_lines_for_path(
    code: &str,
    path: Option<&Path>,
) -> Option<Vec<Line<'static>>> {
    let syntax = syntax_for_path(path)?;
    let lines = highlight_to_spans_with_syntax(code, syntax)?;
    Some(lines.into_iter().map(Line::from).collect())
}

fn highlight_to_spans(code: &str, lang: &str) -> Option<Vec<Vec<Span<'static>>>> {
    let syntax = syntax_set()
        .find_syntax_by_token(lang)
        .or_else(|| syntax_set().find_syntax_by_name(lang))
        .unwrap_or_else(|| syntax_set().find_syntax_plain_text());
    highlight_to_spans_with_syntax(code, syntax)
}

fn highlight_to_spans_with_syntax(
    code: &str,
    syntax: &SyntaxReference,
) -> Option<Vec<Vec<Span<'static>>>> {
    if code.len() > MAX_HIGHLIGHT_BYTES || code.lines().count() > MAX_HIGHLIGHT_LINES {
        return None;
    }

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

fn syntax_for_path(path: Option<&Path>) -> Option<&'static SyntaxReference> {
    let path = path?;

    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        if let Some(syntax) = syntax_set().find_syntax_by_extension(extension) {
            return Some(syntax);
        }

        if let Some(syntax) = syntax_set().find_syntax_by_token(extension) {
            return Some(syntax);
        }
    }

    let file_name = path.file_name().and_then(|value| value.to_str())?;
    syntax_set()
        .find_syntax_by_token(file_name)
        .or_else(|| syntax_set().find_syntax_by_name(file_name))
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

    #[test]
    fn cache_returns_same_result() {
        let code = "fn foo() { 42 }";
        let lines1 = highlight_code_to_lines(code, "rust");
        let lines2 = highlight_code_to_lines(code, "rust");
        assert_eq!(lines1.len(), lines2.len());
    }

    #[test]
    fn large_code_bypasses_highlighting() {
        let large_code = "x".repeat(MAX_HIGHLIGHT_BYTES + 1);
        let lines = highlight_code_to_lines(&large_code, "rust");
        assert!(!lines.is_empty());
    }

    #[test]
    fn highlights_typescript_and_jsx() {
        let ts_code = "const x: number = 42;";
        let tsx_code = "const elem = <div>Hello</div>;";
        let jsx_code = "const elem = <span>World</span>;";

        for (code, lang) in [
            (ts_code, "typescript"),
            (ts_code, "ts"),
            (tsx_code, "tsx"),
            (jsx_code, "jsx"),
        ] {
            let lines = highlight_code_to_lines(code, lang);
            let rendered = lines
                .iter()
                .map(|line| {
                    line.spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(
                rendered, code,
                "content should be preserved for lang={}",
                lang
            );
        }
    }
}
