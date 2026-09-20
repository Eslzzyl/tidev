//! Runtime LaTeX rendering for the terminal message view.
//!
//! RaTeX produces a transparent PNG in memory. `ratatui-image` then converts
//! that image to the terminal's selected graphics protocol. Markdown keeps
//! the measured geometry immediately and requests the expensive PNG/protocol
//! conversion asynchronously when the formula is visible.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use image::ImageFormat;
use ratatui::layout::Size;
use ratatui::style::Color as TuiColor;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::sliced::SlicedProtocol;
use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_render::{RenderOptions, render_to_png};
use ratex_types::color::Color as RatexColor;
use ratex_types::math_style::MathStyle;

const MAX_FORMULA_BYTES: usize = 16 * 1024;
const FORMULA_CACHE_MAX_ENTRIES: usize = 512;
const INLINE_FONT_SCALE: f32 = 0.82;
const DISPLAY_FONT_SCALE: f32 = 0.96;
const INLINE_PADDING: f32 = 1.0;
const DISPLAY_PADDING: f32 = 2.0;
const DEVICE_PIXEL_RATIO: f32 = 2.0;
const MIN_FONT_SIZE: f32 = 6.0;

static RENDER_WAKEUP: AtomicBool = AtomicBool::new(false);

/// Whether a formula appears in prose or in a standalone display block.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum FormulaKind {
    Inline,
    Display,
}

impl FormulaKind {
    fn math_style(self) -> MathStyle {
        match self {
            Self::Inline => MathStyle::Text,
            Self::Display => MathStyle::Display,
        }
    }

    fn font_scale(self) -> f32 {
        match self {
            Self::Inline => INLINE_FONT_SCALE,
            Self::Display => DISPLAY_FONT_SCALE,
        }
    }

    fn padding(self) -> f32 {
        match self {
            Self::Inline => INLINE_PADDING,
            Self::Display => DISPLAY_PADDING,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FormulaKey {
    source: String,
    kind: FormulaKind,
    foreground: [u8; 3],
    font_size: (u16, u16),
    protocol: u8,
    max_width: Option<usize>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct LayoutKey {
    source: String,
    kind: FormulaKind,
    foreground: [u8; 3],
}

/// A rendered formula retained in memory for repeated frame redraws.
pub(crate) struct FormulaImage {
    pub(crate) size: Size,
    key: FormulaKey,
    display_list: Arc<ratex_types::display_item::DisplayList>,
    picker: Picker,
    font_size: f32,
    padding: f32,
    render_started: AtomicBool,
    protocol: OnceLock<Option<Arc<SlicedProtocol>>>,
}

impl FormulaImage {
    /// Start rendering this formula away from the TUI thread.
    pub(crate) fn request_render(self: &Arc<Self>) {
        if self.protocol.get().is_some() || self.render_started.swap(true, Ordering::AcqRel) {
            return;
        }

        let image = Arc::clone(self);
        rayon::spawn(move || {
            let rendered = image.render_protocol().ok().map(Arc::new);
            let _ = image.protocol.set(rendered);
            RENDER_WAKEUP.store(true, Ordering::Release);
        });
    }

    /// Render synchronously for the protocol-construction test.
    #[cfg(test)]
    pub(crate) fn render_now(&self) -> Option<&SlicedProtocol> {
        self.protocol
            .get_or_init(|| self.render_protocol().ok().map(Arc::new));
        self.protocol.get().and_then(|protocol| protocol.as_deref())
    }

    #[cfg(test)]
    pub(crate) fn protocol(&self) -> Option<&SlicedProtocol> {
        self.protocol.get().and_then(|protocol| protocol.as_deref())
    }

    pub(crate) fn protocol_arc(&self) -> Option<Arc<SlicedProtocol>> {
        self.protocol.get().and_then(Clone::clone)
    }

    fn render_protocol(&self) -> Result<SlicedProtocol, String> {
        let render_options = RenderOptions {
            font_size: self.font_size,
            padding: self.padding,
            background_color: RatexColor::new(0.0, 0.0, 0.0, 0.0),
            font_dir: String::new(),
            device_pixel_ratio: DEVICE_PIXEL_RATIO,
        };
        let png = render_to_png(&self.display_list, &render_options)?;
        let image = image::load_from_memory_with_format(&png, ImageFormat::Png)
            .map_err(|err| format!("formula PNG decode failed: {err}"))?;
        SlicedProtocol::new(&self.picker, image, None)
            .map_err(|err| format!("terminal image protocol failed: {err}"))
    }
}

impl fmt::Debug for FormulaImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FormulaImage")
            .field("size", &self.size)
            .field("source", &self.key.source)
            .field("kind", &self.key.kind)
            .finish()
    }
}

impl PartialEq for FormulaImage {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for FormulaImage {}

struct FormulaRuntime {
    picker: Option<Picker>,
    foreground: [u8; 3],
    cache: HashMap<FormulaKey, Arc<FormulaImage>>,
    layout_cache: HashMap<LayoutKey, Arc<ratex_types::display_item::DisplayList>>,
}

static RUNTIME: LazyLock<Mutex<FormulaRuntime>> = LazyLock::new(|| {
    Mutex::new(FormulaRuntime {
        picker: None,
        foreground: [255, 255, 255],
        cache: HashMap::new(),
        layout_cache: HashMap::new(),
    })
});

/// Install the terminal capability snapshot used by formula image creation.
pub(crate) fn configure_picker(picker: Option<Picker>) {
    let mut runtime = RUNTIME.lock().unwrap();
    let changed = match (&runtime.picker, &picker) {
        (Some(old), Some(new)) => {
            let old_size = old.font_size();
            let new_size = new.font_size();
            old.protocol_type() != new.protocol_type()
                || old_size.width != new_size.width
                || old_size.height != new_size.height
        }
        (None, None) => false,
        _ => true,
    };
    runtime.picker = picker;
    if changed {
        runtime.cache.clear();
    }
}

/// Set the foreground used by newly rendered formula pixels.
pub(crate) fn set_foreground(color: TuiColor) {
    let mut runtime = RUNTIME.lock().unwrap();
    runtime.foreground = tui_color_rgb(color);
}

/// Return the foreground cache component used by MarkdownRender's cache key.
pub(crate) fn foreground_key() -> [u8; 3] {
    RUNTIME.lock().unwrap().foreground
}

pub(crate) fn foreground_color() -> TuiColor {
    let [r, g, b] = foreground_key();
    TuiColor::Rgb(r, g, b)
}

pub(crate) fn render_wakeup_pending() -> bool {
    RENDER_WAKEUP.load(Ordering::Acquire)
}

pub(crate) fn acknowledge_render_wakeup() {
    RENDER_WAKEUP.store(false, Ordering::Release);
}

/// Render one formula, reducing its font size when a terminal width limit is set.
pub(crate) fn render_with_width(
    source: &str,
    kind: FormulaKind,
    foreground: TuiColor,
    max_width: Option<usize>,
) -> Option<Arc<FormulaImage>> {
    let source = source.trim();
    if source.is_empty()
        || source.len() > MAX_FORMULA_BYTES
        || source
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
    {
        return None;
    }

    let foreground = tui_color_rgb(foreground);
    let (picker, key) = {
        let runtime = RUNTIME.lock().unwrap();
        let picker = runtime.picker.clone()?;
        let key = FormulaKey {
            source: source.to_string(),
            kind,
            foreground,
            font_size: (picker.font_size().width, picker.font_size().height),
            protocol: protocol_key(picker.protocol_type()),
            max_width,
        };
        if let Some(image) = runtime.cache.get(&key) {
            return Some(image.clone());
        }
        (picker, key)
    };

    let image = build_image(source, kind, foreground, &picker, key.clone(), max_width).ok()?;
    let image = Arc::new(image);

    let mut runtime = RUNTIME.lock().unwrap();
    if let Some(existing) = runtime.cache.get(&key) {
        return Some(existing.clone());
    }
    if runtime.cache.len() >= FORMULA_CACHE_MAX_ENTRIES
        && let Some(oldest) = runtime.cache.keys().next().cloned()
    {
        runtime.cache.remove(&oldest);
    }
    runtime.cache.insert(key, image.clone());
    Some(image)
}

fn build_image(
    source: &str,
    kind: FormulaKind,
    foreground: [u8; 3],
    picker: &Picker,
    key: FormulaKey,
    max_width: Option<usize>,
) -> Result<FormulaImage, String> {
    let display_list = layout_for(source, kind, foreground)?;
    let font_size = font_size_for_width(&display_list, kind, picker, max_width);
    let size = natural_size(&display_list, kind, picker, font_size);
    if size.width == 0 || size.height == 0 {
        return Err("formula image has no terminal cells".to_string());
    }

    Ok(FormulaImage {
        size,
        key,
        display_list,
        picker: picker.clone(),
        font_size,
        padding: kind.padding(),
        render_started: AtomicBool::new(false),
        protocol: OnceLock::new(),
    })
}

fn natural_size(
    display_list: &ratex_types::display_item::DisplayList,
    kind: FormulaKind,
    picker: &Picker,
    font_size: f32,
) -> Size {
    let dpr = DEVICE_PIXEL_RATIO;
    let padding = kind.padding() * dpr;
    let width = (display_list.width as f32 * font_size * dpr + 2.0 * padding)
        .ceil()
        .max(1.0) as u32;
    let height = (display_list.total_height() as f32 * font_size * dpr + 2.0 * padding)
        .ceil()
        .max(1.0) as u32;
    let font_size = picker.font_size();
    Size::new(
        width
            .div_ceil(u32::from(font_size.width.max(1)))
            .min(u32::from(u16::MAX)) as u16,
        height
            .div_ceil(u32::from(font_size.height.max(1)))
            .min(u32::from(u16::MAX)) as u16,
    )
}

fn layout_for(
    source: &str,
    kind: FormulaKind,
    foreground: [u8; 3],
) -> Result<Arc<ratex_types::display_item::DisplayList>, String> {
    let layout_key = LayoutKey {
        source: source.to_string(),
        kind,
        foreground,
    };
    {
        let runtime = RUNTIME.lock().unwrap();
        if let Some(display_list) = runtime.layout_cache.get(&layout_key) {
            return Ok(display_list.clone());
        }
    }

    let nodes = parse(source).map_err(|err| format!("formula parse failed: {err}"))?;
    let ratex_foreground = RatexColor::rgb(
        f32::from(foreground[0]) / 255.0,
        f32::from(foreground[1]) / 255.0,
        f32::from(foreground[2]) / 255.0,
    );
    let options = LayoutOptions::default()
        .with_style(kind.math_style())
        .with_color(ratex_foreground);
    let display_list = Arc::new(to_display_list(&layout(&nodes, &options)));

    let mut runtime = RUNTIME.lock().unwrap();
    if let Some(existing) = runtime.layout_cache.get(&layout_key) {
        return Ok(existing.clone());
    }
    if runtime.layout_cache.len() >= FORMULA_CACHE_MAX_ENTRIES
        && let Some(oldest) = runtime.layout_cache.keys().next().cloned()
    {
        runtime.layout_cache.remove(&oldest);
    }
    runtime
        .layout_cache
        .insert(layout_key, display_list.clone());
    Ok(display_list)
}

fn font_size_for_width(
    display_list: &ratex_types::display_item::DisplayList,
    kind: FormulaKind,
    picker: &Picker,
    max_width: Option<usize>,
) -> f32 {
    let initial = f32::from(picker.font_size().height).max(1.0) * kind.font_scale();
    let Some(max_width) = max_width else {
        return initial;
    };
    if display_list.width <= 0.0 || max_width == 0 {
        return MIN_FONT_SIZE;
    }

    let cell_width = f32::from(picker.font_size().width.max(1));
    let pixel_budget = max_width as f32 * cell_width;
    let padding_pixels = 2.0 * kind.padding() * DEVICE_PIXEL_RATIO;
    let content_budget = pixel_budget - padding_pixels;
    if content_budget <= 0.0 {
        return MIN_FONT_SIZE;
    }

    let max_font_size = content_budget / (display_list.width as f32 * DEVICE_PIXEL_RATIO);
    initial.min(max_font_size.max(MIN_FONT_SIZE))
}

fn tui_color_rgb(color: TuiColor) -> [u8; 3] {
    match color {
        TuiColor::Rgb(r, g, b) => [r, g, b],
        TuiColor::Black => [0, 0, 0],
        TuiColor::Red => [205, 49, 49],
        TuiColor::Green => [13, 188, 121],
        TuiColor::Yellow => [229, 229, 16],
        TuiColor::Blue => [36, 114, 200],
        TuiColor::Magenta => [188, 63, 188],
        TuiColor::Cyan => [17, 168, 205],
        TuiColor::Gray => [229, 229, 229],
        TuiColor::DarkGray => [102, 102, 102],
        TuiColor::LightRed => [241, 76, 76],
        TuiColor::LightGreen => [35, 209, 139],
        TuiColor::LightYellow => [245, 245, 67],
        TuiColor::LightBlue => [59, 142, 234],
        TuiColor::LightMagenta => [214, 112, 214],
        TuiColor::LightCyan => [41, 184, 219],
        TuiColor::White => [255, 255, 255],
        TuiColor::Indexed(index) => xterm_indexed_rgb(index),
        TuiColor::Reset => [255, 255, 255],
    }
}

fn protocol_key(protocol: ProtocolType) -> u8 {
    match protocol {
        ProtocolType::Halfblocks => 0,
        ProtocolType::Sixel => 1,
        ProtocolType::Kitty => 2,
        ProtocolType::Iterm2 => 3,
    }
}

fn xterm_indexed_rgb(index: u8) -> [u8; 3] {
    const BASIC: [[u8; 3]; 16] = [
        [0, 0, 0],
        [205, 49, 49],
        [13, 188, 121],
        [229, 229, 16],
        [36, 114, 200],
        [188, 63, 188],
        [17, 168, 205],
        [229, 229, 229],
        [102, 102, 102],
        [241, 76, 76],
        [35, 209, 139],
        [245, 245, 67],
        [59, 142, 234],
        [214, 112, 214],
        [41, 184, 219],
        [255, 255, 255],
    ];
    if index < 16 {
        return BASIC[index as usize];
    }
    if index < 232 {
        let value = index - 16;
        let component = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
        return [
            component(value / 36),
            component((value / 6) % 6),
            component(value % 6),
        ];
    }
    let gray = 8 + (index - 232) * 10;
    [gray, gray, gray]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_inline_and_multiline_formulas() {
        configure_picker(Some(Picker::halfblocks()));
        let foreground = TuiColor::Rgb(230, 230, 230);

        let inline = render_with_width("x^2 + y^2", FormulaKind::Inline, foreground, None)
            .expect("inline formula should render");
        let multiline = render_with_width(
            r"\begin{aligned} a &= b + c \\ d &= e + f \end{aligned}",
            FormulaKind::Display,
            foreground,
            None,
        )
        .expect("multiline formula should render");

        assert!(inline.size.width > 0);
        assert!(inline.size.height > 0);
        assert!(multiline.size.width > 0);
        assert!(multiline.size.height >= inline.size.height);
    }

    #[test]
    fn renders_protocol_asynchronously() {
        configure_picker(Some(Picker::halfblocks()));
        let image = render_with_width(
            r"\frac{a+b}{c+d}",
            FormulaKind::Display,
            TuiColor::White,
            Some(80),
        )
        .expect("formula should be prepared");

        image.request_render();
        for _ in 0..200 {
            if image.protocol().is_some() {
                assert!(render_wakeup_pending());
                acknowledge_render_wakeup();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("formula protocol was not rendered asynchronously");
    }

    #[test]
    fn rejects_invalid_or_oversized_formulas() {
        configure_picker(Some(Picker::halfblocks()));
        let foreground = TuiColor::White;
        assert!(
            render_with_width("\\unknowncommand{", FormulaKind::Inline, foreground, None).is_none()
        );
        assert!(
            render_with_width(
                &"x".repeat(MAX_FORMULA_BYTES + 1),
                FormulaKind::Inline,
                foreground,
                None,
            )
            .is_none()
        );
    }

    #[test]
    fn builds_a_kitty_protocol_image_when_kitty_is_selected() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui_image::sliced::{SignedPosition, SlicedImage};

        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(ProtocolType::Kitty);
        let key = FormulaKey {
            source: "x + 1".to_string(),
            kind: FormulaKind::Inline,
            foreground: [255, 255, 255],
            font_size: (picker.font_size().width, picker.font_size().height),
            protocol: protocol_key(ProtocolType::Kitty),
            max_width: None,
        };
        let image = build_image(
            "x + 1",
            FormulaKind::Inline,
            [255, 255, 255],
            &picker,
            key,
            None,
        )
        .expect("Kitty image protocol should be constructible");
        let protocol = image
            .render_now()
            .expect("Kitty image protocol should be renderable");
        assert!(matches!(protocol, SlicedProtocol::Kitty(_)));
        assert_eq!(protocol.size(), image.size);

        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| {
                frame.render_widget(
                    SlicedImage::new(protocol, SignedPosition { x: 0, y: 0 }),
                    frame.area(),
                );
            })
            .expect("Kitty widget should render");
        let symbol = terminal
            .backend()
            .buffer()
            .cell((0, 0))
            .expect("Kitty image origin cell")
            .symbol();
        assert!(symbol.contains('\u{10EEEE}'));
    }
}
