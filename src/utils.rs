//! Utility functions for formatting and common operations.

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

/// Sum token usage from a slice of TokenUsage.
///
/// # Examples
///
/// ```
/// use tidev::utils::{sum_token_usage, TokenUsage};
///
/// let items = [
///     TokenUsage::new(100, 50, 0, 0),
///     TokenUsage::new(200, 100, 0, 0),
/// ];
///
/// let total = sum_token_usage(&items);
/// assert_eq!(total.input_tokens, 300);
/// assert_eq!(total.output_tokens, 150);
/// ```
pub fn sum_token_usage(items: &[TokenUsage]) -> TokenUsage {
    let mut total = TokenUsage::default();
    for item in items {
        total.add(*item);
    }
    total
}

/// Token count units: K (thousand), M (million), B (billion), T (trillion).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenUnit {
    /// Thousand (10^3)
    K,
    /// Million (10^6)
    M,
    /// Billion (10^9)
    B,
    /// Trillion (10^12)
    T,
}

/// Format a token count with appropriate unit suffix.
///
/// - Below 1000: raw number
/// - 1K-999K: "X.XK"
/// - 1M-999M: "X.XM"
/// - 1B-999B: "X.XB"
/// - 1T+: "X.XT"
///
/// # Examples
///
/// ```
/// use tidev::utils::format_token_count;
///
/// assert_eq!(format_token_count(500), "500");
/// assert_eq!(format_token_count(1500), "1.5K");
/// assert_eq!(format_token_count(1_500_000), "1.5M");
/// ```
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
///
/// # Examples
///
/// ```
/// use tidev::utils::shorten;
///
/// assert_eq!(shorten("hello", 10), "hello");
/// assert_eq!(shorten("hello world!", 10), "hello worl...");
/// ```
pub fn shorten(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let mut shortened = value.chars().take(max_chars).collect::<String>();
    shortened.push_str("...");
    shortened
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_token_count() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(999), "999");
        assert_eq!(format_token_count(1000), "1.0K");
        assert_eq!(format_token_count(1500), "1.5K");
        assert_eq!(format_token_count(999_999), "1000.0K");
        assert_eq!(format_token_count(1_000_000), "1.0M");
        assert_eq!(format_token_count(1_500_000), "1.5M");
        assert_eq!(format_token_count(999_999_999), "1000.0M");
        assert_eq!(format_token_count(1_000_000_000), "1.0B");
        assert_eq!(format_token_count(999_999_999_999), "1000.0B");
        assert_eq!(format_token_count(1_000_000_000_000), "1.0T");
    }

    #[test]
    fn test_shorten() {
        assert_eq!(shorten("abc", 10), "abc");
        assert_eq!(shorten("hello world!", 10), "hello worl...");
        assert_eq!(shorten("short", 5), "short");
        assert_eq!(shorten("exactly", 7), "exactly");
        assert_eq!(shorten("toolong", 4), "tool...");
        assert_eq!(shorten("tiny", 0), "...");
        assert_eq!(shorten("tiny", 1), "t...");
        assert_eq!(shorten("tiny", 2), "ti...");
    }
}
