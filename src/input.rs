use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Debug)]
pub struct Composer {
    text: String,
    cursor: usize,
    history: Vec<String>,
    history_cursor: Option<usize>,
    draft: String,
    placeholder: String,
}

impl Composer {
    pub fn new(placeholder: impl Into<String>) -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_cursor: None,
            draft: String::new(),
            placeholder: placeholder.into(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn placeholder(&self) -> &str {
        &self.placeholder
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<String>) {
        self.placeholder = placeholder.into();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.history_cursor = None;
        self.draft.clear();
    }

    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
        self.history_cursor = None;
    }

    pub fn remember_submission(&mut self, submission: &str) {
        if submission.trim().is_empty() {
            self.history_cursor = None;
            self.draft.clear();
            return;
        }

        if self
            .history
            .last()
            .is_none_or(|previous| previous != submission)
        {
            self.history.push(submission.to_string());
        }

        self.history_cursor = None;
        self.draft.clear();
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<String> {
        self.handle_key_with_history(key, true)
    }

    pub fn handle_key_with_history(
        &mut self,
        key: KeyEvent,
        record_history: bool,
    ) -> Option<String> {
        let allow_history_navigation = record_history;

        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        match key.code {
            KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => match c {
                'a' => {
                    self.cursor = 0;
                }
                'e' => {
                    self.cursor = self.text.len();
                }
                'j' => {
                    self.insert_char('\n');
                }
                'u' => {
                    self.text.clear();
                    self.cursor = 0;
                }
                'k' => {
                    self.text.truncate(self.cursor);
                }
                'p' if allow_history_navigation => {
                    self.select_previous_history();
                }
                'n' if allow_history_navigation => {
                    self.select_next_history();
                }
                _ => {}
            },
            KeyCode::Char(c) => {
                self.insert_char(c);
            }
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    || key.modifiers.contains(KeyModifiers::ALT)
                {
                    self.insert_char('\n');
                } else {
                    let submission = self.text.trim_end().to_string();
                    if submission.is_empty() {
                        return None;
                    }

                    if record_history {
                        self.remember_submission(&submission);
                    }
                    self.clear();
                    return Some(submission);
                }
            }
            KeyCode::Backspace => {
                if key.modifiers.contains(KeyModifiers::SUPER) {
                    self.delete_to_line_start();
                } else if key.modifiers.contains(KeyModifiers::ALT)
                    || key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    self.delete_previous_word();
                } else {
                    self.delete_previous_char();
                }
            }
            KeyCode::Delete => {
                if key.modifiers.contains(KeyModifiers::SUPER) {
                    self.delete_to_line_start();
                } else if key.modifiers.contains(KeyModifiers::ALT)
                    || key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    self.delete_previous_word();
                } else {
                    self.delete_next_char();
                }
            }
            KeyCode::Left => {
                self.move_left();
            }
            KeyCode::Right => {
                self.move_right();
            }
            KeyCode::Home => {
                self.cursor = self.line_start(self.cursor);
            }
            KeyCode::End => {
                self.cursor = self.line_end(self.cursor);
            }
            KeyCode::Tab => {
                self.insert_str("    ");
            }
            _ => {}
        }

        None
    }

    pub fn preferred_height(&self, width: u16, max_lines: u16) -> u16 {
        let visible_lines = display_line_count(&self.text, width as usize) as u16;

        visible_lines.min(max_lines).saturating_add(2)
    }

    pub fn cursor_position(&self, width: u16) -> (u16, u16) {
        let width = width as usize;
        if width == 0 {
            return (0, 0);
        }

        let mut line = 0u16;
        let mut column = 0u16;

        for ch in self.text[..self.cursor].chars() {
            if ch == '\n' {
                line += 1;
                column = 0;
                continue;
            }

            let char_width = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
            if column + char_width > width as u16 && column > 0 {
                line += 1;
                column = char_width;
            } else {
                column += char_width;
            }
        }

        (line, column)
    }

    pub fn display_line_count(&self, width: usize) -> usize {
        display_line_count(&self.text, width)
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn insert_char(&mut self, ch: char) {
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.history_cursor = None;
    }

    pub fn insert_str(&mut self, value: &str) {
        self.text.insert_str(self.cursor, value);
        self.cursor += value.len();
        self.history_cursor = None;
    }

    pub fn replace_range(&mut self, start: usize, end: usize, replacement: &str) {
        let start = start.min(self.text.len());
        let end = end.min(self.text.len()).max(start);
        self.text.replace_range(start..end, replacement);
        self.cursor = start + replacement.len();
        self.history_cursor = None;
    }

    fn delete_previous_char(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let previous = self.previous_char_boundary(self.cursor);
        self.text.drain(previous..self.cursor);
        self.cursor = previous;
        self.history_cursor = None;
    }

    fn delete_previous_word(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let mut boundary = self.cursor;
        while boundary > 0 {
            let previous = self.previous_char_boundary(boundary);
            let ch = self.text[previous..boundary].chars().next().unwrap();
            if !ch.is_whitespace() {
                break;
            }
            boundary = previous;
        }

        while boundary > 0 {
            let previous = self.previous_char_boundary(boundary);
            let ch = self.text[previous..boundary].chars().next().unwrap();
            if ch.is_whitespace() {
                break;
            }
            boundary = previous;
        }

        self.text.drain(boundary..self.cursor);
        self.cursor = boundary;
        self.history_cursor = None;
    }

    fn delete_to_line_start(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let start = self.line_start(self.cursor);
        self.text.drain(start..self.cursor);
        self.cursor = start;
        self.history_cursor = None;
    }

    fn delete_next_char(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }

        let next = self.next_char_boundary(self.cursor);
        self.text.drain(self.cursor..next);
        self.history_cursor = None;
    }

    fn move_left(&mut self) {
        self.cursor = self.previous_char_boundary(self.cursor);
    }

    fn move_right(&mut self) {
        self.cursor = self.next_char_boundary(self.cursor);
    }

    fn select_previous_history(&mut self) {
        if self.history.is_empty() {
            return;
        }

        if self.history_cursor.is_none() {
            self.draft = self.text.clone();
            self.history_cursor = Some(self.history.len().saturating_sub(1));
        } else if let Some(index) = self.history_cursor
            && index > 0
        {
            self.history_cursor = Some(index - 1);
        }

        if let Some(index) = self.history_cursor {
            self.text = self.history[index].clone();
            self.cursor = self.text.len();
        }
    }

    fn select_next_history(&mut self) {
        let Some(index) = self.history_cursor else {
            return;
        };

        if index + 1 < self.history.len() {
            self.history_cursor = Some(index + 1);
            self.text = self.history[index + 1].clone();
        } else {
            self.history_cursor = None;
            self.text = self.draft.clone();
        }

        self.cursor = self.text.len();
    }

    fn previous_char_boundary(&self, index: usize) -> usize {
        if index == 0 {
            return 0;
        }

        self.text
            .char_indices()
            .take_while(|(byte_index, _)| *byte_index < index)
            .map(|(byte_index, _)| byte_index)
            .last()
            .unwrap_or(0)
    }

    fn next_char_boundary(&self, index: usize) -> usize {
        if index >= self.text.len() {
            return self.text.len();
        }

        self.text[index..]
            .char_indices()
            .nth(1)
            .map(|(relative_index, _)| index + relative_index)
            .unwrap_or(self.text.len())
    }

    fn line_start(&self, index: usize) -> usize {
        self.text[..index]
            .rfind('\n')
            .map(|position| position + 1)
            .unwrap_or(0)
    }

    fn line_end(&self, index: usize) -> usize {
        self.text[index..]
            .find('\n')
            .map(|position| index + position)
            .unwrap_or(self.text.len())
    }
}

fn display_line_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return text.lines().count().max(1);
    }

    text.lines()
        .map(|line| wrap_line_count(line, width))
        .sum::<usize>()
        .max(1)
}

fn wrap_line_count(line: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }

    let mut count = 1;
    let mut current_width = 0;

    for ch in line.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width + char_width > width && current_width > 0 {
            count += 1;
            current_width = char_width;
        } else {
            current_width += char_width;
        }
    }

    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_j_inserts_newline() {
        let mut composer = Composer::new("placeholder");

        let result = composer.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));

        assert!(result.is_none());
        assert_eq!(composer.text(), "\n");
    }

    #[test]
    fn preferred_height_counts_trailing_newline() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("hello\n".to_string());

        assert_eq!(composer.preferred_height(10, 10), 4);
    }

    #[test]
    fn preferred_height_wraps_long_lines() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("abcdefghij".to_string());

        assert_eq!(composer.preferred_height(4, 10), 4);
    }

    #[test]
    fn cursor_position_wraps_long_lines() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("abcdefg".to_string());
        composer.replace_range(0, 7, "abcdefg");
        composer.cursor = 7;

        assert_eq!(composer.cursor_position(4), (1, 3));
    }

    #[test]
    fn ctrl_backspace_deletes_previous_word() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("hello world".to_string());
        composer.cursor = composer.text().len();

        let result = composer.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL));

        assert!(result.is_none());
        assert_eq!(composer.text(), "hello ");
        assert_eq!(composer.cursor(), 6);
    }

    #[test]
    fn alt_backspace_deletes_previous_word() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("hello world".to_string());
        composer.cursor = composer.text().len();

        let result = composer.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));

        assert!(result.is_none());
        assert_eq!(composer.text(), "hello ");
        assert_eq!(composer.cursor(), 6);
    }

    #[test]
    fn super_backspace_deletes_to_line_start() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("hello world".to_string());
        composer.cursor = composer.text().len();

        let result = composer.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::SUPER));

        assert!(result.is_none());
        assert_eq!(composer.text(), "");
        assert_eq!(composer.cursor(), 0);
    }

    #[test]
    fn ctrl_backspace_skips_trailing_whitespace() {
        let mut composer = Composer::new("placeholder");
        composer.set_text("hello world   ".to_string());
        composer.cursor = composer.text().len();

        let result = composer.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL));

        assert!(result.is_none());
        assert_eq!(composer.text(), "hello ");
        assert_eq!(composer.cursor(), 6);
    }
}
