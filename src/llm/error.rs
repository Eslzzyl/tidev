use std::time::Duration;

/// Categorizes network errors by whether they are retryable.
#[derive(Debug, Clone)]
pub enum NetworkError {
    /// Retryable: server-side errors (5xx), timeout, rate limit.
    Retryable { message: String },
    /// Non-retryable: auth failure, not found, payload too large, etc.
    NonRetryable { message: String },
}

impl NetworkError {
    pub fn message(&self) -> &str {
        match self {
            Self::Retryable { message } => message,
            Self::NonRetryable { message } => message,
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable { .. })
    }
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for NetworkError {}

/// Classifies a reqwest `Response` status code into a `NetworkError`.
pub fn classify_response_status(status: reqwest::StatusCode, _body: Option<String>) -> NetworkError {
    let code = status.as_u16();
    let message = status.canonical_reason().unwrap_or("Unknown").to_string();

    match code {
        // Retryable: server errors and rate limits
        408 | 429 | 500 | 502 | 503 | 504 => NetworkError::Retryable {
            message,
        },
        // Non-retryable: auth, not found, payload too large
        401 | 403 | 404 | 413 => NetworkError::NonRetryable {
            message,
        },
        // Unexpected but treat as non-retryable
        _ => NetworkError::NonRetryable {
            message,
        },
    }
}

/// Classifies reqwest/IO errors into a `NetworkError`.
fn classify_reqwest_error(error: &reqwest::Error) -> NetworkError {
    // Check for timeout errors
    if error.is_timeout() {
        return NetworkError::Retryable {
            message: "Request timed out".to_string(),
        };
    }

    // Check for connect errors
    if error.is_connect() {
        return NetworkError::Retryable {
            message: "Connection failed".to_string(),
        };
    }

    // Check for status code if available
    if let Some(status) = error.status() {
        return classify_response_status(status, None);
    }

    // Default: treat as non-retryable
    NetworkError::NonRetryable {
        message: error.to_string(),
    }
}

/// Classifies an `anyhow::Error` (typically wrapping reqwest errors) into a `NetworkError`.
///
/// Attempts to downcast the inner error to `reqwest::Error` for classification.
/// Falls back to non-retryable if downcast fails.
pub fn classify_anyhow_error(error: anyhow::Error) -> NetworkError {
    // Try to downcast to reqwest::Error
    if let Some(reqwest_error) = error.downcast_ref::<reqwest::Error>() {
        return classify_reqwest_error(reqwest_error);
    }

    // Non-retryable fallback
    NetworkError::NonRetryable {
        message: error.to_string(),
    }
}

/// Calculates exponential backoff delay for retry attempts.
///
/// Base delay is 2 seconds, multiplied by 2^(attempt-1).
/// Caps at 30 seconds.
pub fn backoff_delay(attempt: u32) -> Duration {
    let delay_secs = 2u64.saturating_pow(attempt).min(30);
    Duration::from_secs(delay_secs)
}

/// Async sleep helper for backoff.
pub async fn backoff_sleep(attempt: u32) {
    tokio::time::sleep(backoff_delay(attempt)).await;
}

/// Maximum number of retry attempts.
pub const MAX_RETRIES: u32 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_delay() {
        assert_eq!(backoff_delay(1), Duration::from_secs(2));
        assert_eq!(backoff_delay(2), Duration::from_secs(4));
        assert_eq!(backoff_delay(3), Duration::from_secs(8));
        assert_eq!(backoff_delay(4), Duration::from_secs(16));
        assert_eq!(backoff_delay(5), Duration::from_secs(30)); // capped
    }

    #[test]
    fn test_classify_status() {
        let retryable = classify_response_status(reqwest::StatusCode::INTERNAL_SERVER_ERROR, None);
        assert!(retryable.is_retryable());

        let non_retryable = classify_response_status(reqwest::StatusCode::UNAUTHORIZED, None);
        assert!(!non_retryable.is_retryable());

        let rate_limit = classify_response_status(reqwest::StatusCode::TOO_MANY_REQUESTS, None);
        assert!(rate_limit.is_retryable());
    }
}