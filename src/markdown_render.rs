use crate::render::highlight::highlight_code_to_lines;
use crate::render::line_utils::push_owned_lines;
use crate::wrapping::adaptive_wrap_line;
use crate::wrapping::word_wrap_line;
use crate::wrapping::RtOptions;
use pulldown_cmark::CodeBlockKind;
use pulldown_cmark::Alignment;
use pulldown_cmark::CowStr;
use pulldown_cmark::Event;
use pulldown_cmark::HeadingLevel;
use pulldown_cmark::Options;
use pulldown_cmark::Parser;
use pulldown_cmark::Tag;
use pulldown_cmark::TagEnd;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use unicode_width::UnicodeWidthStr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;
use url::Url;

struct MarkdownStyles {
    h1: Style,
    h2: Style,
    h3: Style,
    h4: Style,
    h5: Style,
    h6: Style,
    code: Style,
    emphasis: Style,
    strong: Style,
    strikethrough: Style,
    ordered_list_marker: Style,
    unordered_list_marker: Style,
    link: Style,
    blockquote: Style,
}

impl Default for MarkdownStyles {
    fn default() -> Self {
        Self {
            h1: Style::default().bold().underlined(),
            h2: Style::default().bold(),
            h3: Style::default().bold().italic(),
            h4: Style::default().italic(),
            h5: Style::default().italic(),
            h6: Style::default().italic(),
            code: Style::default().cyan(),
            emphasis: Style::default().italic(),
            strong: Style::default().bold(),
            strikethrough: Style::default().crossed_out(),
            ordered_list_marker: Style::default().light_blue(),
            unordered_list_marker: Style::default(),
            link: Style::default().cyan().underlined(),
            blockquote: Style::default().green(),
        }
    }
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

#[derive(Clone, Debug)]
struct TableRowState {
    is_header: bool,
    cells: Vec<Line<'static>>,
}

#[derive(Clone, Debug)]
struct TableState {
    prefix: Vec<Span<'static>>,
    base_style: Style,
    alignments: Vec<Alignment>,
    rows: Vec<TableRowState>,
    current_row: Option<TableRowState>,
    in_head: bool,
}

impl TableState {
    fn new(prefix: Vec<Span<'static>>, base_style: Style, alignments: Vec<Alignment>) -> Self {
        Self {
            prefix,
            base_style,
            alignments,
            rows: Vec::new(),
            current_row: None,
            in_head: false,
        }
    }

    fn start_head(&mut self) {
        self.in_head = true;
    }

    fn end_head(&mut self) {
        self.in_head = false;
    }

    fn start_row(&mut self) {
        self.finish_row();
        self.current_row = Some(TableRowState {
            is_header: self.in_head,
            cells: Vec::new(),
        });
    }

    fn finish_row(&mut self) {
        if let Some(row) = self.current_row.take() {
            self.rows.push(row);
        }
    }

    fn push_cell(&mut self, cell: Line<'static>) {
        if let Some(row) = self.current_row.as_mut() {
            row.cells.push(cell);
        }
    }

    fn render(mut self, wrap_width: Option<usize>) -> Vec<Line<'static>> {
        self.finish_row();
        if self.rows.is_empty() {
            return Vec::new();
        }

        let prefix_width = display_line_width(&Line::from(self.prefix.clone()));
        let available_width = wrap_width.map(|width| width.saturating_sub(prefix_width));

        let mut rows = std::mem::take(&mut self.rows);
        let header_index = rows.iter().position(|row| row.is_header).unwrap_or(0);
        let header_row = rows.remove(header_index);
        let body_rows = rows;

        let column_count = header_row
            .cells
            .len()
            .max(body_rows.iter().map(|row| row.cells.len()).max().unwrap_or(0))
            .max(self.alignments.len());

        if column_count == 0 {
            return Vec::new();
        }

        let natural_widths = self.measure_column_widths(&header_row, &body_rows, column_count);
        let widths = match available_width {
            Some(available_width) => {
                let min_cell_width = 3usize;
                let min_total = table_border_overhead(column_count)
                    .saturating_add(column_count.saturating_mul(min_cell_width));

                if available_width < min_total && !body_rows.is_empty() {
                    return self.render_stacked_rows(&header_row, &body_rows, available_width);
                }

                let content_budget = available_width.saturating_sub(table_border_overhead(column_count));
                match shrink_table_widths(natural_widths, content_budget, min_cell_width) {
                    Some(widths) => widths,
                    None => return self.render_stacked_rows(&header_row, &body_rows, available_width),
                }
            }
            None => natural_widths,
        };

        let mut out = Vec::new();
        out.push(self.render_border_line('┌', '┬', '┐', &widths));
        out.extend(self.render_row_block(&header_row, &widths, true));

        if !body_rows.is_empty() {
            out.push(self.render_border_line('├', '┼', '┤', &widths));

            for (index, row) in body_rows.iter().enumerate() {
                out.extend(self.render_row_block(row, &widths, false));
                if index + 1 < body_rows.len() {
                    out.push(self.render_border_line('├', '┼', '┤', &widths));
                }
            }
        }

        out.push(self.render_border_line('└', '┴', '┘', &widths));
        out
    }

    fn measure_column_widths(
        &self,
        header_row: &TableRowState,
        body_rows: &[TableRowState],
        column_count: usize,
    ) -> Vec<usize> {
        let mut widths = vec![1usize; column_count];

        for row in std::iter::once(header_row).chain(body_rows.iter()) {
            for (index, cell) in row.cells.iter().enumerate().take(column_count) {
                widths[index] = widths[index].max(display_line_width(cell).max(1));
            }
        }

        widths
    }

    fn render_row_block(
        &self,
        row: &TableRowState,
        widths: &[usize],
        is_header: bool,
    ) -> Vec<Line<'static>> {
        let wrapped_cells: Vec<Vec<Line<'static>>> = row
            .cells
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                let width = widths.get(index).copied().unwrap_or(1).max(1);
                let wrapped = word_wrap_line(cell, RtOptions::new(width).break_words(true));
                let mut owned = Vec::new();
                push_owned_lines(&wrapped, &mut owned);
                if owned.is_empty() {
                    vec![Line::default()]
                } else {
                    owned
                }
            })
            .collect();

        let row_height = wrapped_cells.iter().map(Vec::len).max().unwrap_or(1);
        let row_style = if is_header {
            self.base_style.add_modifier(Modifier::BOLD)
        } else {
            self.base_style
        };

        let mut out = Vec::with_capacity(row_height);
        for line_index in 0..row_height {
            let mut spans = self.prefix.clone();
            spans.push(Span::raw("│"));

            for column_index in 0..widths.len() {
                spans.push(Span::raw(" "));
                let cell_line = wrapped_cells
                    .get(column_index)
                    .and_then(|lines| lines.get(line_index))
                    .cloned()
                    .unwrap_or_default();
                spans.extend(pad_cell_spans(
                    cell_line,
                    widths[column_index],
                    self.alignments
                        .get(column_index)
                        .copied()
                        .unwrap_or(Alignment::Left),
                ));
                spans.push(Span::raw(" "));
                spans.push(Span::raw("│"));
            }

            out.push(Line::from_iter(spans).style(row_style));
        }

        out
    }

    fn render_border_line(&self, left: char, middle: char, right: char, widths: &[usize]) -> Line<'static> {
        let mut spans = self.prefix.clone();
        spans.push(Span::raw(left.to_string()));

        for index in 0..widths.len() {
            spans.push(Span::raw("─".repeat(widths[index] + 2)));
            if index + 1 < widths.len() {
                spans.push(Span::raw(middle.to_string()));
            }
        }

        spans.push(Span::raw(right.to_string()));
        Line::from_iter(spans).style(self.base_style)
    }

    fn render_stacked_rows(
        &self,
        header_row: &TableRowState,
        body_rows: &[TableRowState],
        available_width: usize,
    ) -> Vec<Line<'static>> {
        if body_rows.is_empty() {
            return Vec::new();
        }

        let card_width = available_width.saturating_sub(4).max(1);
        let mut out = Vec::new();
        for (row_index, row) in body_rows.iter().enumerate() {
            if row_index > 0 {
                out.push(Line::default());
            }

            out.push(self.render_border_line('┌', '─', '┐', &[card_width]));
            out.extend(self.render_stacked_row(header_row, row, card_width));
            out.push(self.render_border_line('└', '─', '┘', &[card_width]));
        }

        out
    }

    fn render_stacked_row(
        &self,
        header_row: &TableRowState,
        row: &TableRowState,
        card_width: usize,
    ) -> Vec<Line<'static>> {
        let label_style = self.base_style.add_modifier(Modifier::BOLD);
        let mut out = Vec::new();
        let column_count = header_row.cells.len().max(row.cells.len());

        for index in 0..column_count {
            let label = header_row
                .cells
                .get(index)
                .map(line_to_plain_text)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| format!("Column {}", index + 1));
            let value = row.cells.get(index).cloned().unwrap_or_default();

            let mut field = Line::from(vec![Span::styled(format!("{label}: "), label_style)]);
            field.spans.extend(value.spans);

            let wrapped = word_wrap_line(&field, RtOptions::new(card_width).break_words(true));
            let mut owned = Vec::new();
            push_owned_lines(&wrapped, &mut owned);

            for line in owned {
                let mut spans = self.prefix.clone();
                spans.push(Span::raw("│"));
                spans.push(Span::raw(" "));
                spans.extend(pad_cell_spans(line, card_width, Alignment::Left));
                spans.push(Span::raw(" "));
                spans.push(Span::raw("│"));
                out.push(Line::from_iter(spans).style(self.base_style));
            }
        }

        out
    }
}

fn table_border_overhead(column_count: usize) -> usize {
    column_count.saturating_mul(3).saturating_add(1)
}

fn shrink_table_widths(
    mut widths: Vec<usize>,
    target_total: usize,
    min_width: usize,
) -> Option<Vec<usize>> {
    let mut total: usize = widths.iter().sum();
    if total <= target_total {
        return Some(widths);
    }

    let minimum_total = widths.len().saturating_mul(min_width);
    if target_total < minimum_total {
        return None;
    }

    while total > target_total {
        let mut chosen_index = None;
        let mut chosen_room = 0usize;

        for (index, width) in widths.iter().enumerate() {
            let room = width.saturating_sub(min_width);
            if room > chosen_room {
                chosen_room = room;
                chosen_index = Some(index);
            }
        }

        let Some(index) = chosen_index else {
            return None;
        };

        if widths[index] <= min_width {
            return None;
        }

        widths[index] -= 1;
        total -= 1;
    }

    Some(widths)
}

fn line_to_plain_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

pub fn render_markdown_text(input: &str) -> Text<'static> {
    render_markdown_text_with_width(input, None)
}

pub(crate) fn render_markdown_text_with_width(
    input: &str,
    width: Option<usize>,
) -> Text<'static> {
    let cwd = std::env::current_dir().ok();
    render_markdown_text_with_width_and_cwd(input, width, cwd.as_deref())
}

pub(crate) fn render_markdown_text_with_width_and_cwd(
    input: &str,
    width: Option<usize>,
    cwd: Option<&Path>,
) -> Text<'static> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(input, options);
    let mut writer = Writer::new(parser, cwd);
    writer.wrap_width = width;
    writer.run();
    writer.text
}

struct Writer<'a, I>
where
    I: Iterator<Item = Event<'a>>,
{
    iter: I,
    text: Text<'static>,
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
    current_line_content: Option<Line<'static>>,
    current_initial_indent: Vec<Span<'static>>,
    current_subsequent_indent: Vec<Span<'static>>,
    current_line_style: Style,
    current_line_in_code_block: bool,
    wrap_width: Option<usize>,
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
                self.push_line(Line::from("———"));
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
        let content = format!("{} ", "#".repeat(level as usize));
        self.push_line(Line::from(vec![Span::styled(content, heading_style)]));
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
        self.indent_stack.push(IndentContext::new(
            vec![Span::from("> ")],
            None,
            false,
        ));
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
            self.text.lines.extend(table.render(self.wrap_width));
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
        if let Some(table) = self.table_state.as_mut() {
            if !(table.in_head && table.current_row.is_some()) {
                table.start_row();
            }
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
        self.current_line_content = Some(Line::default());
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
        let mut parts = text.split('\n').peekable();
        while let Some(part) = parts.next() {
            if !part.is_empty() {
                self.push_span(Span::styled(part.to_string(), style));
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
            self.code_block_buffer.push_str(&text);
            return;
        }

        for (index, line) in text.lines().enumerate() {
            if self.needs_newline {
                self.push_line(Line::default());
                self.needs_newline = false;
            }
            if index > 0 {
                self.push_line(Line::default());
            }
            let span = Span::styled(
                line.to_string(),
                self.inline_styles.last().copied().unwrap_or_default(),
            );
            self.push_span(span);
        }
        self.needs_newline = false;
    }

    fn code(&mut self, code: CowStr<'a>) {
        if self.suppressing_local_link_label() {
            return;
        }
        self.line_ends_with_local_link_target = false;
        if self.in_table_cell {
            self.push_span(Span::from(code.into_string()).style(self.styles.code));
            return;
        }
        if self.pending_marker_line {
            self.push_line(Line::default());
            self.pending_marker_line = false;
        }
        let span = Span::from(code.into_string()).style(self.styles.code);
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
        for (index, line) in html.lines().enumerate() {
            if self.needs_newline {
                self.push_line(Line::default());
                self.needs_newline = false;
            }
            if index > 0 {
                self.push_line(Line::default());
            }
            let style = self.inline_styles.last().copied().unwrap_or_default();
            self.push_span(Span::styled(line.to_string(), style));
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
        self.indent_stack.push(IndentContext::new(indent_prefix, marker, true));
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
                let highlighted = highlight_code_to_lines(&code, &lang);
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
                self.push_span(Span::styled(line.to_string(), self.styles.code));
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
                self.push_span(" (".into());
                self.push_span(Span::styled(link.destination, self.styles.link));
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
            let line = line.style(style);

            let should_wrap = self
                .wrap_width
                .is_some_and(|width| width > 0)
                && !self.current_line_in_code_block
                && !line.spans.is_empty();

            if should_wrap {
                let width = self.wrap_width.expect("wrap_width checked above");
                let wrapped = adaptive_wrap_line(
                    &line,
                    RtOptions::new(width)
                        .initial_indent(Line::from(self.current_initial_indent.clone()))
                        .subsequent_indent(Line::from(self.current_subsequent_indent.clone())),
                );
                push_owned_lines(&wrapped, &mut self.text.lines);
            } else {
                let mut spans = self.current_initial_indent.clone();
                let mut line = line;
                spans.append(&mut line.spans);
                self.text.lines.push(Line::from_iter(spans).style(style));
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
        self.current_line_content = Some(line);
        self.current_line_in_code_block = self.in_code_block;
        self.line_ends_with_local_link_target = false;

        self.pending_marker_line = false;
    }

    fn push_span(&mut self, span: Span<'static>) {
        if self.in_table_cell && self.current_line_content.is_none() {
            self.current_line_content = Some(Line::default());
        }
        if let Some(line) = self.current_line_content.as_mut() {
            line.push_span(span);
        } else {
            self.push_line(Line::from(vec![span]));
        }
    }

    fn push_blank_line(&mut self) {
        self.flush_current_line();
        if self.indent_stack.iter().all(|context| context.is_list) {
            self.text.lines.push(Line::default());
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
        let last_list_index = self.indent_stack.iter().rposition(|context| context.is_list);

        for (index, context) in self.indent_stack.iter().enumerate() {
            if pending_marker_line {
                if Some(index) == last_marker_index
                    && let Some(marker) = &context.marker
                {
                    prefix.extend(marker.iter().cloned());
                    continue;
                }
                if context.is_list && last_marker_index.is_some_and(|marker_index| marker_index > index)
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

fn display_line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn pad_cell_spans(
    cell: Line<'static>,
    width: usize,
    alignment: Alignment,
) -> Vec<Span<'static>> {
    let cell_width = display_line_width(&cell);
    let padding = width.saturating_sub(cell_width);
    let (left_pad, right_pad) = match alignment {
        Alignment::Center => (padding / 2, padding.saturating_sub(padding / 2)),
        Alignment::Right => (padding, 0),
        Alignment::Left | Alignment::None => (0, padding),
    };

    let mut spans = Vec::new();
    if left_pad > 0 {
        spans.push(Span::from(" ".repeat(left_pad)));
    }
    spans.extend(cell.spans);
    if right_pad > 0 {
        spans.push(Span::from(" ".repeat(right_pad)));
    }

    spans
}

fn should_render_link_destination(dest_url: &str) -> bool {
    !is_local_path_like_link(dest_url)
}

static HASH_LOCATION_SUFFIX_RE: LazyLock<()> = LazyLock::new(|| ());

fn is_local_path_like_link(dest_url: &str) -> bool {
    dest_url.starts_with("file://")
        || dest_url.starts_with('/')
        || dest_url.starts_with("~/")
        || dest_url.starts_with("./")
        || dest_url.starts_with("../")
        || dest_url.starts_with("\\\\")
        || matches!(
            dest_url.as_bytes(),
            [drive, b':', separator, ..]
                if drive.is_ascii_alphabetic() && matches!(separator, b'/' | b'\\')
        )
}

fn render_local_link_target(dest_url: &str, cwd: Option<&Path>) -> Option<String> {
    let (path_text, location_suffix) = parse_local_link_target(dest_url)?;
    let mut rendered = display_local_link_path(&path_text, cwd);
    if let Some(location_suffix) = location_suffix {
        rendered.push_str(&location_suffix);
    }
    Some(rendered)
}

fn parse_local_link_target(dest_url: &str) -> Option<(String, Option<String>)> {
    if dest_url.starts_with("file://") {
        let url = Url::parse(dest_url).ok()?;
        let path_text = file_url_to_local_path_text(&url)?;
        let location_suffix = url.fragment().and_then(normalize_hash_location_suffix_fragment);
        return Some((path_text, location_suffix));
    }

    let mut path_text = dest_url;
    let mut location_suffix = None;

    if let Some((candidate_path, fragment)) = dest_url.rsplit_once('#')
        && let Some(normalized) = normalize_hash_location_suffix_fragment(fragment)
    {
        path_text = candidate_path;
        location_suffix = Some(normalized);
    }

    if location_suffix.is_none()
        && let Some(suffix) = extract_colon_location_suffix(path_text)
    {
        let path_len = path_text.len().saturating_sub(suffix.len());
        path_text = &path_text[..path_len];
        location_suffix = Some(suffix);
    }

    let decoded_path_text = urlencoding::decode(path_text)
        .unwrap_or_else(|_| std::borrow::Cow::Borrowed(path_text));
    Some((
        expand_local_link_path(&decoded_path_text),
        location_suffix,
    ))
}

fn normalize_hash_location_suffix_fragment(fragment: &str) -> Option<String> {
    if fragment.is_empty() {
        return None;
    }
    let _ = &*HASH_LOCATION_SUFFIX_RE;
    Some(format!("#{fragment}"))
}

fn extract_colon_location_suffix(path_text: &str) -> Option<String> {
    let (prefix, last) = path_text.rsplit_once(':')?;
    if !last.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    if let Some((_, second_last)) = prefix.rsplit_once(':')
        && second_last.chars().all(|ch| ch.is_ascii_digit())
    {
        return Some(format!(":{second_last}:{last}"));
    }

    Some(format!(":{last}"))
}

fn expand_local_link_path(path_text: &str) -> String {
    if let Some(rest) = path_text.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return normalize_local_link_path_text(&home.join(rest).to_string_lossy());
    }

    normalize_local_link_path_text(path_text)
}

fn file_url_to_local_path_text(url: &Url) -> Option<String> {
    if let Ok(path) = url.to_file_path() {
        return Some(normalize_local_link_path_text(&path.to_string_lossy()));
    }

    let mut path_text = url.path().to_string();
    if let Some(host) = url.host_str()
        && !host.is_empty()
        && host != "localhost"
    {
        path_text = format!("//{host}{path_text}");
    } else if matches!(
        path_text.as_bytes(),
        [b'/', drive, b':', b'/', ..] if drive.is_ascii_alphabetic()
    ) {
        path_text.remove(0);
    }

    Some(normalize_local_link_path_text(&path_text))
}

fn normalize_local_link_path_text(path_text: &str) -> String {
    if let Some(rest) = path_text.strip_prefix("\\\\") {
        format!("//{}", rest.replace('\\', "/").trim_start_matches('/'))
    } else {
        path_text.replace('\\', "/")
    }
}

fn is_absolute_local_link_path(path_text: &str) -> bool {
    path_text.starts_with('/')
        || path_text.starts_with("//")
        || matches!(
            path_text.as_bytes(),
            [drive, b':', b'/', ..] if drive.is_ascii_alphabetic()
        )
}

fn trim_trailing_local_path_separator(path_text: &str) -> &str {
    if path_text == "/" || path_text == "//" {
        return path_text;
    }
    if matches!(path_text.as_bytes(), [drive, b':', b'/'] if drive.is_ascii_alphabetic()) {
        return path_text;
    }
    path_text.trim_end_matches('/')
}

fn strip_local_path_prefix<'a>(path_text: &'a str, cwd_text: &str) -> Option<&'a str> {
    let path_text = trim_trailing_local_path_separator(path_text);
    let cwd_text = trim_trailing_local_path_separator(cwd_text);
    if path_text == cwd_text {
        return None;
    }

    if cwd_text == "/" || cwd_text == "//" {
        return path_text.strip_prefix('/');
    }

    path_text
        .strip_prefix(cwd_text)
        .and_then(|rest| rest.strip_prefix('/'))
}

fn display_local_link_path(path_text: &str, cwd: Option<&Path>) -> String {
    let path_text = normalize_local_link_path_text(path_text);
    if !is_absolute_local_link_path(&path_text) {
        return path_text;
    }

    if let Some(cwd) = cwd {
        let cwd_text = normalize_local_link_path_text(&cwd.to_string_lossy());
        if let Some(stripped) = strip_local_path_prefix(&path_text, &cwd_text) {
            return stripped.to_string();
        }
    }

    path_text
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
        assert_eq!(rendered, vec!["# Title".to_string()]);
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
        assert!(rendered.iter().any(|line| line.contains("a long value")));
        assert!(rendered.last().is_some_and(|line| line.starts_with('└')));
    }
}
