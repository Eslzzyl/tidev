//! Utility types and functions for the TUI.

/// Token count units: K (thousand), M (million), B (billion), T (trillion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenUsage {
    /// Input tokens (prompt tokens)
    pub input_tokens: u32,
    /// Output tokens (completion tokens)
    pub output_tokens: u32,
    /// Cache read tokens (cached prompt tokens)
    pub cache_read_tokens: u32,
    /// Cache write tokens (cache creation tokens)
    pub cache_write_tokens: u32,
}

impl TokenUsage {
    /// Total tokens (input + output)
    pub fn total(&self) -> u64 {
        self.input_tokens as u64 + self.output_tokens as u64
    }

    /// Total cache tokens (read + write)
    pub fn total_cache(&self) -> u64 {
        self.cache_read_tokens as u64 + self.cache_write_tokens as u64
    }

    /// Context usage percentage given a context window size
    pub fn context_usage_pct(&self, context_window: usize) -> f64 {
        if context_window == 0 {
            return 0.0;
        }
        let total = self.total() as f64;
        (total / context_window as f64 * 100.0).min(100.0)
    }

    /// Calculate tokens per second given duration in milliseconds
    pub fn tokens_per_second(&self, duration_ms: Option<u64>) -> Option<f32> {
        let ms = duration_ms?;
        if ms > 0 {
            Some(self.output_tokens as f32 / (ms as f32 / 1000.0))
        } else {
            None
        }
    }

    /// Create from individual values
    pub fn new(
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
    ) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        }
    }

    /// Add two token usages together.
    pub fn add(&mut self, other: Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
    }
}

/// Format a token count with appropriate unit suffix.
pub fn format_token_count(count: u64) -> String {
    if count >= 1_000_000_000_000 {
        let value = count as f64 / 1_000_000_000_000.0;
        format!("{:.1}T", value)
    } else if count >= 1_000_000_000 {
        let value = count as f64 / 1_000_000_000.0;
        format!("{:.1}B", value)
    } else if count >= 1_000_000 {
        let value = count as f64 / 1_000_000.0;
        format!("{:.1}M", value)
    } else if count >= 1_000 {
        let value = count as f64 / 1_000.0;
        format!("{:.1}K", value)
    } else {
        count.to_string()
    }
}

/// Format a token count, accepting u32 input.
pub fn format_token_count_u32(count: u32) -> String {
    format_token_count(count as u64)
}

/// Shorten a string to fit within max_chars, appending "..." if truncated.
pub fn shorten(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let mut shortened = value.chars().take(max_chars).collect::<String>();
    shortened.push_str("...");
    shortened
}
