//! Session utility functions.

/// Maximum characters for a session title derived from user prompt.
const MAX_TITLE_CHARS: usize = 48;

/// Extract a session title from a user prompt (first line, trimmed, max 48 chars).
///
/// Returns `"Untitled session"` when the prompt is empty or contains only
/// whitespace.
pub fn title_from_prompt(prompt: &str) -> String {
    let first_line = prompt.lines().next().unwrap_or("Untitled session").trim();
    if first_line.is_empty() {
        return "Untitled session".to_string();
    }
    let mut title: String = first_line.chars().take(MAX_TITLE_CHARS).collect();
    if first_line.chars().count() > MAX_TITLE_CHARS {
        title.push_str("...");
    }
    title
}
