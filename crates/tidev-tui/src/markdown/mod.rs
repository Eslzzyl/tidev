mod highlight;
mod line;
mod links;
mod styles;
mod table;
mod wrap;

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;

use pulldown_cmark::{
    Alignment, CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use unicode_width::UnicodeWidthStr;

pub(crate) use highlight::highlight_code_to_lines;
pub use highlight::highlight_code_to_lines_for_path;
pub(crate) use highlight::set_syntax_theme_by_key;

use line::line_to_static;
use wrap::RtOptions;
pub use wrap::{RtOptions as WrapOptions, adaptive_wrap_line, adaptive_wrap_lines, word_wrap_line};

pub use links::is_local_path_like_link;

use crate::ansi::strip_ansi;
use crate::hyperlink::{
    HyperlinkLine, HyperlinkRange, annotate_web_urls_in_line, remap_wrapped_line, web_destination,
};
use crate::utils::expand_tabs;
use links::{render_local_link_target, should_render_link_destination};
use styles::MarkdownStyles;
use table::TableState;

#[cfg(test)]
pub fn render_markdown_text(input: &str) -> Text<'static> {
    render_markdown_text_with_width(input, None)
}

#[cfg(test)]
pub(crate) fn render_markdown_text_with_width(input: &str, width: Option<usize>) -> Text<'static> {
    let cwd = std::env::current_dir().ok();
    let arc = render_markdown_text_with_width_and_cwd(input, width, cwd.as_deref());
    arc.text.clone()
}

/// Rendered markdown output: the styled lines plus per-line hyperlink ranges.
///
/// `line_links` is index-aligned with `text.lines`: each entry lists the
/// hyperlink ranges of the corresponding line in display columns.
pub(crate) struct MarkdownRender {
    pub text: Text<'static>,
    pub line_links: Vec<Vec<HyperlinkRange>>,
}

impl Deref for MarkdownRender {
    type Target = Text<'static>;

    fn deref(&self) -> &Text<'static> {
        &self.text
    }
}

/// Convert a rendered markdown output into hyperlink-annotated lines.
pub(crate) fn markdown_to_hyperlink_lines(md: &MarkdownRender) -> Vec<HyperlinkLine> {
    md.text
        .lines
        .iter()
        .zip(md.line_links.iter())
        .map(|(line, links)| HyperlinkLine {
            line: line.clone(),
            hyperlinks: links.clone(),
        })
        .collect()
}

/// Cache key for the markdown render cache: (content_hash, wrap_width, cwd_hash)
type MarkdownCacheKey = (blake3::Hash, Option<usize>, blake3::Hash);

/// Content-hash based cache for rendered markdown output.
/// Keyed by (blake3::Hash of input, width, cwd_hash) to avoid re-parsing markdown
/// when neither content, terminal width, nor workspace has changed.
/// Wrapped in `Arc` so repeated cache hits share the same allocation.
static MARKDOWN_RENDER_CACHE: LazyLock<
    Mutex<std::collections::HashMap<MarkdownCacheKey, Arc<MarkdownRender>>>,
> = LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

/// Maximum number of entries in the markdown render cache.
const MARKDOWN_RENDER_CACHE_MAX_ENTRIES: usize = 256;

pub fn render_markdown_text_with_width_and_cwd(
    input: &str,
    width: Option<usize>,
    cwd: Option<&Path>,
) -> Arc<MarkdownRender> {
    let content_hash = blake3::hash(input.as_bytes());
    let cwd_hash = blake3::hash(cwd.map(|p| p.as_os_str().as_encoded_bytes()).unwrap_or(b""));

    // Check cache
    {
        let cache = MARKDOWN_RENDER_CACHE.lock().unwrap();
        if let Some(cached) = cache.get(&(content_hash, width, cwd_hash)) {
            return cached.clone();
        }
    }

    // Cache miss - render
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(input, options);
    let mut writer = Writer::new(parser, cwd);
    writer.wrap_width = width;
    writer.run();

    // Cache the result
    let result = Arc::new(MarkdownRender {
        text: writer.text,
        line_links: writer.line_links,
    });
    {
        let mut cache = MARKDOWN_RENDER_CACHE.lock().unwrap();
        // Evict oldest entry if cache is full
        if cache.len() >= MARKDOWN_RENDER_CACHE_MAX_ENTRIES {
            // Remove a random-ish entry (cheapest eviction)
            if let Some(key) = cache.keys().next().cloned() {
                cache.remove(&key);
            }
        }
        cache.insert((content_hash, width, cwd_hash), result.clone());
    }

    // Return Arc (callers share via clone or clone inner on demand)
    result
}

#[derive(Clone, Debug)]
struct IndentContext {
    prefix: Vec<Span<'static>>,
    marker: Option<Vec<Span<'static>>>,
    is_list: bool,
}

impl IndentContext {
    fn new(prefix: Vec<Span<'static>>, marker: Option<Vec<Span<'static>>>, is_list: bool) -> Self {
        Self {
            prefix,
            marker,
            is_list,
        }
    }
}

#[derive(Clone, Debug)]
struct LinkState {
    destination: String,
    show_destination: bool,
    local_target_display: Option<String>,
}

struct Writer<'a, I>
where
    I: Iterator<Item = Event<'a>>,
{
    iter: I,
    text: Text<'static>,
    /// Per-line hyperlink ranges, index-aligned with `text.lines`.
    line_links: Vec<Vec<HyperlinkRange>>,
    styles: MarkdownStyles,
    inline_styles: Vec<Style>,
    indent_stack: Vec<IndentContext>,
    list_indices: Vec<Option<u64>>,
    link: Option<LinkState>,
    needs_newline: bool,
    pending_marker_line: bool,
    in_paragraph: bool,
    in_code_block: bool,
    code_block_lang: Option<String>,
    code_block_buffer: String,
    cwd: Option<PathBuf>,
    line_ends_with_local_link_target: bool,
    pending_local_link_soft_break: bool,
    current_line_content: Option<HyperlinkLine>,
    current_initial_indent: Vec<Span<'static>>,
    current_subsequent_indent: Vec<Span<'static>>,
    current_line_style: Style,
    current_line_in_code_block: bool,
    wrap_width: Option<usize>,
    tab_width: usize,
    table_state: Option<TableState>,
    in_table_cell: bool,
}

impl<'a, I> Writer<'a, I>
where
    I: Iterator<Item = Event<'a>>,
{
    fn new(iter: I, cwd: Option<&Path>) -> Self {
        Self {
            iter,
            text: Text::default(),
            line_links: Vec::new(),
            styles: MarkdownStyles::default(),
            inline_styles: Vec::new(),
            indent_stack: Vec::new(),
            list_indices: Vec::new(),
            link: None,
            needs_newline: false,
            pending_marker_line: false,
            in_paragraph: false,
            in_code_block: false,
            code_block_lang: None,
            code_block_buffer: String::new(),
            cwd: cwd.map(Path::to_path_buf),
            line_ends_with_local_link_target: false,
            pending_local_link_soft_break: false,
            current_line_content: None,
            current_initial_indent: Vec::new(),
            current_subsequent_indent: Vec::new(),
            current_line_style: Style::default(),
            current_line_in_code_block: false,
            wrap_width: None,
            tab_width: 4,
            table_state: None,
            in_table_cell: false,
        }
    }

    fn run(&mut self) {
        while let Some(event) = self.iter.next() {
            self.handle_event(event);
        }
        self.flush_current_line();
    }

    fn handle_event(&mut self, event: Event<'a>) {
        self.prepare_for_event(&event);
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.text(text),
            Event::Code(code) => self.code(code),
            Event::InlineMath(math) => self.text(math),
            Event::DisplayMath(math) => self.text(math),
            Event::SoftBreak => self.soft_break(),
            Event::HardBreak => self.hard_break(),
            Event::Rule => {
                self.flush_current_line();
                if !self.text.lines.is_empty() {
                    self.push_blank_line();
                }
                // Draw a centered horizontal rule using ─ characters.
                // No explicit color — inherits terminal/theme default foreground,
                // just like bold text does, so it follows theme changes naturally.
                let width = self.wrap_width.unwrap_or(80);
                let rule_len = ((width as f64) * 2.0 / 3.0) as usize;
                let rule_len = rule_len.max(8);
                let padding = (width.saturating_sub(rule_len)) / 2;
                let line_str = format!("{}{}", " ".repeat(padding), "─".repeat(rule_len),);
                self.push_line(Line::from(line_str));
                self.needs_newline = true;
            }
            Event::Html(html) => self.html(html, false),
            Event::InlineHtml(html) => self.html(html, true),
            Event::FootnoteReference(_) => {}
            Event::TaskListMarker(_) => {}
        }
    }

    fn prepare_for_event(&mut self, event: &Event<'a>) {
        if !self.pending_local_link_soft_break {
            return;
        }

        if matches!(event, Event::Text(text) if text.trim_start().starts_with(':')) {
            self.pending_local_link_soft_break = false;
            return;
        }

        self.pending_local_link_soft_break = false;
        self.push_line(Line::default());
    }

    fn start_tag(&mut self, tag: Tag<'a>) {
        match tag {
            Tag::Paragraph => self.start_paragraph(),
            Tag::Heading { level, .. } => self.start_heading(level),
            Tag::BlockQuote(_) => self.start_blockquote(),
            Tag::CodeBlock(kind) => {
                let indent = match kind {
                    CodeBlockKind::Fenced(_) => None,
                    CodeBlockKind::Indented => Some(Span::from(" ".repeat(4))),
                };
                let lang = match kind {
                    CodeBlockKind::Fenced(lang) => Some(lang.to_string()),
                    CodeBlockKind::Indented => None,
                };
                self.start_codeblock(lang, indent)
            }
            Tag::List(start) => self.start_list(start),
            Tag::Item => self.start_item(),
            Tag::Table(alignments) => self.start_table(alignments),
            Tag::TableHead => self.start_table_head(),
            Tag::TableRow => self.start_table_row(),
            Tag::TableCell => self.start_table_cell(),
            Tag::Emphasis => self.push_inline_style(self.styles.emphasis),
            Tag::Strong => self.push_inline_style(self.styles.strong),
            Tag::Strikethrough => self.push_inline_style(self.styles.strikethrough),
            Tag::Link { dest_url, .. } => self.push_link(dest_url.to_string()),
            Tag::HtmlBlock
            | Tag::FootnoteDefinition(_)
            | Tag::Image { .. }
            | Tag::MetadataBlock(_) => {}
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.end_paragraph(),
            TagEnd::Heading(_) => self.end_heading(),
            TagEnd::BlockQuote(_) => self.end_blockquote(),
            TagEnd::CodeBlock => self.end_codeblock(),
            TagEnd::List(_) => self.end_list(),
            TagEnd::Item => {
                self.indent_stack.pop();
                self.pending_marker_line = false;
            }
            TagEnd::Table => self.end_table(),
            TagEnd::TableHead => self.end_table_head(),
            TagEnd::TableRow => self.end_table_row(),
            TagEnd::TableCell => self.end_table_cell(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_inline_style(),
            TagEnd::Link => self.pop_link(),
            TagEnd::HtmlBlock
            | TagEnd::FootnoteDefinition
            | TagEnd::Image
            | TagEnd::MetadataBlock(_) => {}
            _ => {}
        }
    }

    fn start_paragraph(&mut self) {
        if self.needs_newline {
            self.push_blank_line();
        }
        self.push_line(Line::default());
        self.needs_newline = false;
        self.in_paragraph = true;
    }

    fn end_paragraph(&mut self) {
        self.needs_newline = true;
        self.in_paragraph = false;
        self.pending_marker_line = false;
    }

    fn start_heading(&mut self, level: HeadingLevel) {
        if self.needs_newline {
            self.push_line(Line::default());
            self.needs_newline = false;
        }
        let heading_style = match level {
            HeadingLevel::H1 => self.styles.h1,
            HeadingLevel::H2 => self.styles.h2,
            HeadingLevel::H3 => self.styles.h3,
            HeadingLevel::H4 => self.styles.h4,
            HeadingLevel::H5 => self.styles.h5,
            HeadingLevel::H6 => self.styles.h6,
        };
        self.push_line(Line::default());
        self.push_inline_style(heading_style);
        self.needs_newline = false;
    }

    fn end_heading(&mut self) {
        self.needs_newline = true;
        self.pop_inline_style();
    }

    fn start_blockquote(&mut self) {
        if self.needs_newline {
            self.push_blank_line();
            self.needs_newline = false;
        }
        self.indent_stack
            .push(IndentContext::new(vec![Span::from("> ")], None, false));
    }

    fn end_blockquote(&mut self) {
        self.indent_stack.pop();
        self.needs_newline = true;
    }

    fn start_table(&mut self, alignments: Vec<Alignment>) {
        self.flush_current_line();
        if !self.text.lines.is_empty() {
            self.push_blank_line();
        }

        self.pending_marker_line = false;
        self.in_table_cell = false;
        self.table_state = Some(TableState::new(
            self.prefix_spans(false),
            self.current_line_style,
            alignments,
        ));
        self.needs_newline = false;
    }

    fn end_table(&mut self) {
        self.flush_current_line();
        if let Some(table) = self.table_state.take() {
            for line in table.render(self.wrap_width) {
                self.push_output_line(line);
            }
        }
        self.in_table_cell = false;
        self.needs_newline = true;
    }

    fn start_table_head(&mut self) {
        if let Some(table) = self.table_state.as_mut() {
            table.start_head();
            table.start_row();
        }
    }

    fn end_table_head(&mut self) {
        if let Some(table) = self.table_state.as_mut() {
            table.finish_row();
            table.end_head();
        }
    }

    fn start_table_row(&mut self) {
        self.flush_current_line();
        if let Some(table) = self.table_state.as_mut()
            && !(table.in_head && table.current_row.is_some())
        {
            table.start_row();
        }
        self.in_table_cell = false;
    }

    fn end_table_row(&mut self) {
        self.flush_current_line();
        if let Some(table) = self.table_state.as_mut() {
            table.finish_row();
        }
        self.in_table_cell = false;
    }

    fn start_table_cell(&mut self) {
        self.flush_current_line();
        self.in_table_cell = true;
        self.current_line_content = Some(HyperlinkLine::new(Line::default()));
        self.current_initial_indent.clear();
        self.current_subsequent_indent.clear();
        self.current_line_style = self
            .table_state
            .as_ref()
            .map(|table| table.base_style)
            .unwrap_or_default();
        self.current_line_in_code_block = false;
    }

    fn end_table_cell(&mut self) {
        self.flush_current_line();
        self.in_table_cell = false;
    }

    fn table_text(&mut self, text: CowStr<'a>) {
        let style = self.inline_styles.last().copied().unwrap_or_default();
        let stripped = strip_ansi(&text);
        let mut parts = stripped.split('\n').peekable();
        while let Some(part) = parts.next() {
            if !part.is_empty() {
                self.push_text_spans(&expand_tabs(part, self.tab_width), style);
            }
            if parts.peek().is_some() {
                self.push_span(Span::from(" "));
            }
        }
    }

    fn text(&mut self, text: CowStr<'a>) {
        if self.suppressing_local_link_label() {
            return;
        }
        self.line_ends_with_local_link_target = false;
        if self.in_table_cell {
            self.table_text(text);
            return;
        }
        if self.pending_marker_line {
            self.push_line(Line::default());
        }
        self.pending_marker_line = false;

        if self.in_code_block {
            let stripped = strip_ansi(&text);
            self.code_block_buffer.push_str(&stripped);
            return;
        }

        let stripped = strip_ansi(&text);
        for (index, line) in stripped.lines().enumerate() {
            if self.needs_newline {
                self.push_line(Line::default());
                self.needs_newline = false;
            }
            if index > 0 {
                self.push_line(Line::default());
            }
            let style = self.inline_styles.last().copied().unwrap_or_default();
            self.push_text_spans(&expand_tabs(line, self.tab_width), style);
        }
        self.needs_newline = false;
    }

    fn code(&mut self, code: CowStr<'a>) {
        if self.suppressing_local_link_label() {
            return;
        }
        self.line_ends_with_local_link_target = false;
        let stripped = strip_ansi(&code);
        if self.in_table_cell {
            self.push_span(Span::styled(
                expand_tabs(&stripped, self.tab_width),
                self.styles.code,
            ));
            return;
        }
        if self.pending_marker_line {
            self.push_line(Line::default());
            self.pending_marker_line = false;
        }
        let span = Span::styled(expand_tabs(&stripped, self.tab_width), self.styles.code);
        self.push_span(span);
    }

    fn html(&mut self, html: CowStr<'a>, inline: bool) {
        if self.suppressing_local_link_label() {
            return;
        }
        self.line_ends_with_local_link_target = false;
        if self.in_table_cell {
            self.table_text(html);
            return;
        }
        self.pending_marker_line = false;
        let stripped = strip_ansi(&html);
        for (index, line) in stripped.lines().enumerate() {
            if self.needs_newline {
                self.push_line(Line::default());
                self.needs_newline = false;
            }
            if index > 0 {
                self.push_line(Line::default());
            }
            let style = self.inline_styles.last().copied().unwrap_or_default();
            self.push_span(Span::styled(expand_tabs(line, self.tab_width), style));
        }
        self.needs_newline = !inline;
    }

    fn hard_break(&mut self) {
        if self.suppressing_local_link_label() {
            return;
        }
        if self.in_table_cell {
            self.push_span(Span::from(" "));
            return;
        }
        if self.in_code_block {
            self.code_block_buffer.push('\n');
            return;
        }
        self.line_ends_with_local_link_target = false;
        self.push_line(Line::default());
    }

    fn soft_break(&mut self) {
        if self.suppressing_local_link_label() {
            return;
        }
        if self.in_table_cell {
            self.push_span(Span::from(" "));
            return;
        }
        if self.in_code_block {
            self.code_block_buffer.push('\n');
            return;
        }
        if self.line_ends_with_local_link_target {
            self.pending_local_link_soft_break = true;
            self.line_ends_with_local_link_target = false;
            return;
        }
        self.line_ends_with_local_link_target = false;
        self.push_line(Line::default());
    }

    fn start_list(&mut self, index: Option<u64>) {
        if self.list_indices.is_empty() && self.needs_newline {
            self.push_line(Line::default());
        }
        self.list_indices.push(index);
    }

    fn end_list(&mut self) {
        self.list_indices.pop();
        self.needs_newline = true;
    }

    fn start_item(&mut self) {
        self.pending_marker_line = true;
        let depth = self.list_indices.len();
        let is_ordered = self
            .list_indices
            .last()
            .map(Option::is_some)
            .unwrap_or(false);
        let width = depth * 4 - 3;
        let marker = if let Some(last_index) = self.list_indices.last_mut() {
            match last_index {
                None => Some(vec![Span::styled(
                    " ".repeat(width - 1) + "- ",
                    self.styles.unordered_list_marker,
                )]),
                Some(index) => {
                    *index += 1;
                    Some(vec![Span::styled(
                        format!("{:width$}. ", *index - 1),
                        self.styles.ordered_list_marker,
                    )])
                }
            }
        } else {
            None
        };
        let indent_prefix = if depth == 0 {
            Vec::new()
        } else {
            let indent_len = if is_ordered { width + 2 } else { width + 1 };
            vec![Span::from(" ".repeat(indent_len))]
        };
        self.indent_stack
            .push(IndentContext::new(indent_prefix, marker, true));
        self.needs_newline = false;
    }

    fn start_codeblock(&mut self, lang: Option<String>, indent: Option<Span<'static>>) {
        self.flush_current_line();
        if !self.text.lines.is_empty() {
            self.push_blank_line();
        }
        self.in_code_block = true;
        self.code_block_lang = lang
            .as_deref()
            .and_then(|value| value.split([',', ' ', '\t']).next())
            .filter(|value| !value.is_empty())
            .map(std::string::ToString::to_string);
        self.code_block_buffer.clear();
        self.indent_stack.push(IndentContext::new(
            vec![indent.unwrap_or_default()],
            None,
            false,
        ));
        self.needs_newline = true;
    }

    fn end_codeblock(&mut self) {
        let code = std::mem::take(&mut self.code_block_buffer);
        if let Some(lang) = self.code_block_lang.take() {
            if !code.is_empty() {
                let expanded = expand_tabs(&code, self.tab_width);
                let highlighted = highlight_code_to_lines(&expanded, &lang);
                for line in highlighted {
                    self.push_line(Line::default());
                    for span in line.spans {
                        self.push_span(span);
                    }
                }
            } else {
                self.push_line(Line::default());
            }
        } else if !code.is_empty() {
            for line in code.lines() {
                self.push_line(Line::default());
                self.push_span(Span::styled(
                    expand_tabs(line, self.tab_width),
                    self.styles.code,
                ));
            }
        } else {
            self.push_line(Line::default());
        }

        self.needs_newline = true;
        self.in_code_block = false;
        self.indent_stack.pop();
    }

    fn push_inline_style(&mut self, style: Style) {
        let current = self.inline_styles.last().copied().unwrap_or_default();
        self.inline_styles.push(current.patch(style));
    }

    fn pop_inline_style(&mut self) {
        self.inline_styles.pop();
    }

    fn push_link(&mut self, dest_url: String) {
        let show_destination = should_render_link_destination(&dest_url);
        self.link = Some(LinkState {
            show_destination,
            local_target_display: if is_local_path_like_link(&dest_url) {
                render_local_link_target(&dest_url, self.cwd.as_deref())
            } else {
                None
            },
            destination: dest_url,
        });
    }

    fn pop_link(&mut self) {
        if let Some(link) = self.link.take() {
            if link.show_destination {
                // The visible destination suffix is annotated with its own
                // destination so the rendered URL is clickable.
                let destination = link.destination.clone();
                self.push_span(" (".into());
                let mut destination_line = HyperlinkLine::new(Line::default());
                destination_line.push_span(
                    Span::styled(destination.clone(), self.styles.link),
                    Some(&destination),
                );
                self.push_annotated(destination_line);
                self.push_span(")".into());
            } else if let Some(local_target_display) = link.local_target_display {
                if self.pending_marker_line {
                    self.push_line(Line::default());
                }
                let style = self
                    .inline_styles
                    .last()
                    .copied()
                    .unwrap_or_default()
                    .patch(self.styles.code);
                self.push_span(Span::styled(local_target_display, style));
                self.line_ends_with_local_link_target = true;
            }
        }
    }

    fn suppressing_local_link_label(&self) -> bool {
        self.link
            .as_ref()
            .and_then(|link| link.local_target_display.as_ref())
            .is_some()
    }

    fn flush_current_line(&mut self) {
        if let Some(line) = self.current_line_content.take() {
            if self.in_table_cell {
                if let Some(table) = self.table_state.as_mut() {
                    table.push_cell(line.style(self.current_line_style));
                }
                self.in_table_cell = false;
                self.current_initial_indent.clear();
                self.current_subsequent_indent.clear();
                self.current_line_in_code_block = false;
                self.line_ends_with_local_link_target = false;
                return;
            }

            let style = self.current_line_style;
            let mut line = line.style(style);

            let should_wrap =
                self.wrap_width.is_some_and(|width| width > 0) && !line.line.spans.is_empty();

            if should_wrap {
                let width = self.wrap_width.expect("wrap_width checked above");
                if self.current_line_in_code_block {
                    // Code blocks should be wrapped as a whole line to preserve multiple spans (colors)
                    let wrapped = word_wrap_line(
                        &line.line,
                        RtOptions::new(width)
                            .initial_indent(Line::from(self.current_initial_indent.clone()))
                            .subsequent_indent(Line::from(self.current_subsequent_indent.clone()))
                            .break_words(true),
                    );

                    for w_line in wrapped {
                        // Convert back to owned lines to satisfy 'static requirement
                        let owned_spans: Vec<Span<'static>> = w_line
                            .spans
                            .into_iter()
                            .map(|s| Span::styled(s.content.to_string(), s.style))
                            .collect();
                        self.push_output_line(HyperlinkLine::new(
                            Line::from(owned_spans).style(style),
                        ));
                    }
                } else {
                    let wrapped = adaptive_wrap_line(
                        &line.line,
                        RtOptions::new(width)
                            .initial_indent(Line::from(self.current_initial_indent.clone()))
                            .subsequent_indent(Line::from(self.current_subsequent_indent.clone())),
                    );
                    let owned: Vec<Line<'static>> = wrapped.iter().map(line_to_static).collect();
                    for remapped in remap_wrapped_line(&line, owned) {
                        self.push_output_line(remapped.style(style));
                    }
                }
            } else {
                let mut spans = self.current_initial_indent.clone();
                let shift: usize = spans
                    .iter()
                    .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                    .sum();
                spans.append(&mut line.line.spans);
                let mut line = HyperlinkLine {
                    line: Line::from_iter(spans).style(style),
                    hyperlinks: line.hyperlinks,
                };
                for hyperlink in &mut line.hyperlinks {
                    hyperlink.columns =
                        hyperlink.columns.start + shift..hyperlink.columns.end + shift;
                }
                self.push_output_line(line);
            }
            self.current_initial_indent.clear();
            self.current_subsequent_indent.clear();
            self.current_line_in_code_block = false;
            self.line_ends_with_local_link_target = false;
        }
    }

    fn push_line(&mut self, line: Line<'static>) {
        self.flush_current_line();
        let blockquote_active = self
            .indent_stack
            .iter()
            .any(|context| context.prefix.iter().any(|span| span.content.contains('>')));
        let style = if blockquote_active {
            self.styles.blockquote
        } else {
            line.style
        };
        let was_pending = self.pending_marker_line;

        self.current_initial_indent = self.prefix_spans(was_pending);
        self.current_subsequent_indent = self.prefix_spans(false);
        self.current_line_style = style;
        self.current_line_content = Some(HyperlinkLine::new(line));
        self.current_line_in_code_block = self.in_code_block;
        self.line_ends_with_local_link_target = false;

        self.pending_marker_line = false;
    }

    fn push_span(&mut self, span: Span<'static>) {
        if self.in_table_cell && self.current_line_content.is_none() {
            self.current_line_content = Some(HyperlinkLine::new(Line::default()));
        }
        if let Some(line) = self.current_line_content.as_mut() {
            line.line.push_span(span);
        } else {
            self.push_line(Line::from(vec![span]));
        }
    }

    /// Append `appended` to the current line, shifting its hyperlink columns
    /// by the current line width.
    fn push_annotated(&mut self, mut appended: HyperlinkLine) {
        if self.current_line_content.is_none() {
            self.push_line(Line::default());
        }
        if let Some(line) = self.current_line_content.as_mut() {
            let shift = line.width();
            line.line.spans.append(&mut appended.line.spans);
            line.hyperlinks
                .extend(appended.hyperlinks.into_iter().map(|mut link| {
                    link.columns = link.columns.start + shift..link.columns.end + shift;
                    link
                }));
        }
    }

    /// Push a text span, annotating hyperlinks according to the current
    /// context:
    /// - inside a link with a web destination → the span carries that
    ///   destination (both the label and the visible destination suffix);
    /// - inside a non-web link or a code block → plain text (no links);
    /// - otherwise → bare web URLs are detected and annotated.
    fn push_text_spans(&mut self, text: &str, style: Style) {
        let span = Span::styled(text.to_string(), style);
        let destination = self
            .link
            .as_ref()
            .and_then(|link| web_destination(&link.destination));
        let annotated = if let Some(destination) = destination {
            let mut annotated = HyperlinkLine::new(Line::default());
            annotated.push_span(span, Some(&destination));
            annotated
        } else if self.link.is_some() || self.in_code_block {
            HyperlinkLine::new(Line::from(span))
        } else {
            annotate_web_urls_in_line(Line::from(span))
        };
        self.push_annotated(annotated);
    }

    /// Push a finished output line, keeping `line_links` aligned with `text.lines`.
    fn push_output_line(&mut self, line: HyperlinkLine) {
        let links = line.hyperlinks;
        self.text.lines.push(line.line);
        self.line_links.push(links);
    }

    fn push_blank_line(&mut self) {
        self.flush_current_line();
        if self.indent_stack.iter().all(|context| context.is_list) {
            self.push_output_line(HyperlinkLine::new(Line::default()));
        } else {
            self.push_line(Line::default());
            self.flush_current_line();
        }
    }

    fn prefix_spans(&self, pending_marker_line: bool) -> Vec<Span<'static>> {
        let mut prefix = Vec::new();
        let last_marker_index = if pending_marker_line {
            self.indent_stack
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, context)| context.marker.as_ref().map(|_| index))
        } else {
            None
        };
        let last_list_index = self
            .indent_stack
            .iter()
            .rposition(|context| context.is_list);

        for (index, context) in self.indent_stack.iter().enumerate() {
            if pending_marker_line {
                if Some(index) == last_marker_index
                    && let Some(marker) = &context.marker
                {
                    prefix.extend(marker.iter().cloned());
                    continue;
                }
                if context.is_list
                    && last_marker_index.is_some_and(|marker_index| marker_index > index)
                {
                    continue;
                }
            } else if context.is_list && Some(index) != last_list_index {
                continue;
            }
            prefix.extend(context.prefix.iter().cloned());
        }

        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_to_strings(text: &Text<'_>) -> Vec<String> {
        text.lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.clone())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn renders_simple_heading() {
        let text = render_markdown_text("# Title\n");
        let rendered = lines_to_strings(&text);
        assert_eq!(rendered, vec!["Title".to_string()]);
    }

    #[test]
    fn renders_list_item() {
        let text = render_markdown_text("- item\n");
        let rendered = lines_to_strings(&text);
        assert_eq!(rendered, vec!["- item".to_string()]);
    }

    #[test]
    fn renders_code_block() {
        let text = render_markdown_text("```rust\nfn main() {}\n```\n");
        let rendered = lines_to_strings(&text);
        assert_eq!(rendered, vec!["fn main() {}".to_string()]);
    }

    #[test]
    fn renders_local_file_link() {
        let text = render_markdown_text_with_width_and_cwd(
            "See [file](/workspace/project/src/lib.rs:12) for details.",
            None,
            Some(Path::new("/workspace/project")),
        );
        let rendered = lines_to_strings(&text);
        assert_eq!(rendered, vec!["See src/lib.rs:12 for details.".to_string()]);
    }

    #[test]
    fn renders_markdown_table() {
        let text = render_markdown_text(
            "| Name | Count |\n|:-----|------:|\n| a | 1 |\n| longer | 23 |\n",
        );
        let rendered = lines_to_strings(&text);
        assert_eq!(
            rendered,
            vec![
                "┌────────┬───────┐".to_string(),
                "│ Name   │ Count │".to_string(),
                "├────────┼───────┤".to_string(),
                "│ a      │     1 │".to_string(),
                "├────────┼───────┤".to_string(),
                "│ longer │    23 │".to_string(),
                "└────────┴───────┘".to_string(),
            ]
        );
    }

    #[test]
    fn renders_markdown_table_in_narrow_space() {
        let text = render_markdown_text_with_width(
            "| Name | Count |\n|:-----|------:|\n| a long value | 12345 |\n| another row | 7 |\n",
            Some(12),
        );
        let rendered = lines_to_strings(&text);

        assert!(rendered.first().is_some_and(|line| line.starts_with('┌')));
        assert!(rendered.iter().any(|line| line.contains("Name:")));
        assert!(rendered.iter().any(|line| line.contains("Count:")));
        assert!(rendered.iter().any(|line| line.contains("long")));
        assert!(rendered.last().is_some_and(|line| line.starts_with('└')));
    }

    #[test]
    fn annotates_explicit_web_link_label_and_visible_destination() {
        let md = render_markdown_text_with_width_and_cwd(
            "[tidev](https://example.com/a)",
            Some(80),
            None,
        );
        let rendered = lines_to_strings(&md);
        assert_eq!(rendered, vec!["tidev (https://example.com/a)".to_string()]);

        let links = &md.line_links[0];
        // Both the label and the visible destination carry the destination.
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].columns, 0..5);
        assert_eq!(links[0].destination, "https://example.com/a");
        assert_eq!(links[1].columns, 7..28);
        assert_eq!(links[1].destination, "https://example.com/a");
    }

    #[test]
    fn annotates_bare_urls_in_plain_text() {
        let md = render_markdown_text_with_width_and_cwd(
            "See https://example.com/a now",
            Some(80),
            None,
        );
        let rendered = lines_to_strings(&md);
        assert_eq!(rendered, vec!["See https://example.com/a now".to_string()]);

        let links = &md.line_links[0];
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].columns, 4..25);
        assert_eq!(links[0].destination, "https://example.com/a");
    }

    #[test]
    fn does_not_annotate_inline_code() {
        let md = render_markdown_text_with_width_and_cwd("`https://example.com/a`", Some(80), None);
        assert!(md.line_links.iter().all(|links| links.is_empty()));
    }

    #[test]
    fn does_not_annotate_local_path_links() {
        let md = render_markdown_text_with_width_and_cwd(
            "See [file](/workspace/project/src/lib.rs:12) for details.",
            Some(80),
            Some(Path::new("/workspace/project")),
        );
        assert!(md.line_links.iter().all(|links| links.is_empty()));
    }

    #[test]
    fn link_ranges_remap_across_wrapped_rows() {
        let md = render_markdown_text_with_width_and_cwd(
            "word [label](https://example.com/very/long/path/that/wraps)",
            Some(20),
            None,
        );
        // The URL token is wider than the row, so adaptive wrapping hard-
        // breaks it across rows. Every rendered fragment must carry the full
        // destination.
        let rendered: Vec<String> = md
            .text
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        let full: String = rendered.iter().flat_map(|s| s.chars()).collect();
        assert!(
            full.contains("https://example.com/very/long/path/that/wraps"),
            "URL content must be preserved across rows: {rendered:?}"
        );

        let mut found = 0usize;
        for links in &md.line_links {
            for link in links {
                assert_eq!(
                    link.destination,
                    "https://example.com/very/long/path/that/wraps"
                );
                found += 1;
            }
        }
        assert!(
            found >= 2,
            "expected the URL to wrap across rows, got {found} range(s)"
        );
    }

    #[test]
    fn annotates_table_cell_links_with_column_shift() {
        let md = render_markdown_text_with_width_and_cwd(
            "| Site |\n|------|\n| [tidev](https://example.com/a) |\n",
            Some(80),
            None,
        );
        // The table renders as a bordered row; the cell link ranges must be
        // shifted past the border + padding.
        let rendered = lines_to_strings(&md);
        let row = rendered
            .iter()
            .find(|line| line.contains("tidev"))
            .expect("table row with the link");
        // `str::find` returns a byte offset; hyperlink columns are display
        // columns, so convert the prefix to display width.
        let url_byte = row.find("https://example.com/a").expect("visible URL");
        let url_col: usize = row[..url_byte]
            .chars()
            .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
            .sum();
        let url_width = unicode_width::UnicodeWidthStr::width("https://example.com/a");
        let links: Vec<_> = md.line_links.iter().flatten().collect();
        assert!(!links.is_empty(), "expected at least one annotated link");
        // The URL link covers exactly the visible URL columns.
        let url_link = links
            .iter()
            .find(|link| link.columns.start == url_col)
            .unwrap_or_else(|| panic!("no link at column {url_col}: {links:?} in {row:?}"));
        assert_eq!(url_link.columns.end, url_col + url_width);
        // The label link sits before the " (url)" suffix.
        assert!(
            links
                .iter()
                .any(|link| link.columns.end <= url_col
                    && link.destination == "https://example.com/a")
        );
    }
}
