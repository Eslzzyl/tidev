use ratatui::text::Line;
use std::path::Path;
use std::path::PathBuf;

use crate::markdown_render::{is_blank_line_spaces_only, render_markdown_text_with_width_and_cwd};

pub(crate) struct MarkdownStreamCollector {
    buffer: String,
    committed_line_count: usize,
    width: Option<usize>,
    cwd: PathBuf,
}

impl MarkdownStreamCollector {
    pub fn new(width: Option<usize>, cwd: &Path) -> Self {
        Self {
            buffer: String::new(),
            committed_line_count: 0,
            width,
            cwd: cwd.to_path_buf(),
        }
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.committed_line_count = 0;
    }

    pub fn push_delta(&mut self, delta: &str) {
        self.buffer.push_str(delta);
    }

    fn render_to_lines(&self, source: &str) -> Vec<Line<'static>> {
        let text =
            render_markdown_text_with_width_and_cwd(source, self.width, Some(self.cwd.as_path()));
        text.lines
    }

    pub fn commit_complete_lines(&mut self) -> Vec<Line<'static>> {
        let Some(last_newline) = self.buffer.rfind('\n') else {
            return Vec::new();
        };

        let source = self.buffer[..=last_newline].to_string();
        let rendered = self.render_to_lines(&source);

        let mut complete_line_count = rendered.len();
        if complete_line_count > 0 && is_blank_line_spaces_only(&rendered[complete_line_count - 1])
        {
            complete_line_count -= 1;
        }

        if self.committed_line_count >= complete_line_count {
            return Vec::new();
        }

        let out = rendered[self.committed_line_count..complete_line_count].to_vec();
        self.committed_line_count = complete_line_count;
        out
    }

    pub fn finalize_and_drain(&mut self) -> Vec<Line<'static>> {
        let mut source = self.buffer.clone();
        if !source.ends_with('\n') {
            source.push('\n');
        }

        let rendered = self.render_to_lines(&source);

        let out = if self.committed_line_count >= rendered.len() {
            Vec::new()
        } else {
            rendered[self.committed_line_count..].to_vec()
        };

        self.clear();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cwd() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn no_commit_until_newline() {
        let mut collector = MarkdownStreamCollector::new(None, &test_cwd());
        collector.push_delta("Hello");
        assert!(collector.commit_complete_lines().is_empty());
        collector.push_delta(" world\n");
        assert_eq!(collector.commit_complete_lines().len(), 1);
    }

    #[test]
    fn finalize_commits_partial_line() {
        let mut collector = MarkdownStreamCollector::new(None, &test_cwd());
        collector.push_delta("Line without newline");
        assert_eq!(collector.finalize_and_drain().len(), 1);
    }
}
