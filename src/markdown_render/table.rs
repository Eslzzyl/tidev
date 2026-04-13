use pulldown_cmark::Alignment;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use super::line::push_owned_lines;
use super::wrap::{word_wrap_line, RtOptions};

#[derive(Clone, Debug)]
pub(super) struct TableRowState {
    pub(super) is_header: bool,
    pub(super) cells: Vec<Line<'static>>,
}

#[derive(Clone, Debug)]
pub(super) struct TableState {
    pub(super) prefix: Vec<Span<'static>>,
    pub(super) base_style: Style,
    pub(super) alignments: Vec<Alignment>,
    pub(super) rows: Vec<TableRowState>,
    pub(super) current_row: Option<TableRowState>,
    pub(super) in_head: bool,
}

impl TableState {
    pub(super) fn new(
        prefix: Vec<Span<'static>>,
        base_style: Style,
        alignments: Vec<Alignment>,
    ) -> Self {
        Self {
            prefix,
            base_style,
            alignments,
            rows: Vec::new(),
            current_row: None,
            in_head: false,
        }
    }

    pub(super) fn start_head(&mut self) {
        self.in_head = true;
    }

    pub(super) fn end_head(&mut self) {
        self.in_head = false;
    }

    pub(super) fn start_row(&mut self) {
        self.finish_row();
        self.current_row = Some(TableRowState {
            is_header: self.in_head,
            cells: Vec::new(),
        });
    }

    pub(super) fn finish_row(&mut self) {
        if let Some(row) = self.current_row.take() {
            self.rows.push(row);
        }
    }

    pub(super) fn push_cell(&mut self, cell: Line<'static>) {
        if let Some(row) = self.current_row.as_mut() {
            row.cells.push(cell);
        }
    }

    pub(super) fn render(mut self, wrap_width: Option<usize>) -> Vec<Line<'static>> {
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
            .max(
                body_rows
                    .iter()
                    .map(|row| row.cells.len())
                    .max()
                    .unwrap_or(0),
            )
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

                let content_budget =
                    available_width.saturating_sub(table_border_overhead(column_count));
                match shrink_table_widths(natural_widths, content_budget, min_cell_width) {
                    Some(widths) => widths,
                    None => {
                        return self.render_stacked_rows(&header_row, &body_rows, available_width);
                    }
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

            for (column_index, &width) in widths.iter().enumerate() {
                spans.push(Span::raw(" "));
                let cell_line = wrapped_cells
                    .get(column_index)
                    .and_then(|lines| lines.get(line_index))
                    .cloned()
                    .unwrap_or_default();
                spans.extend(pad_cell_spans(
                    cell_line,
                    width,
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

    fn render_border_line(
        &self,
        left: char,
        middle: char,
        right: char,
        widths: &[usize],
    ) -> Line<'static> {
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

        let index = chosen_index?;

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

pub(super) fn display_line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn pad_cell_spans(cell: Line<'static>, width: usize, alignment: Alignment) -> Vec<Span<'static>> {
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
