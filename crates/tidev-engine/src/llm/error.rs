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
pub fn classify_response_status(status: reqwest::StatusCode, body: Option<String>) -> NetworkError {
    let code = status.as_u16();
    let message = if let Some(body_text) = body {
        parse_error_body(status, &body_text)
    } else {
        format!(
            "HTTP error {}: {}",
            code,
            status.canonical_reason().unwrap_or("Unknown")
        )
    };

    match code {
        // Retryable: server errors and rate limits
        408 | 429 | 500 | 502 | 503 | 504 | 520 | 529 => NetworkError::Retryable { message },
        // Non-retryable: auth, not found, payload too large
        401 | 403 | 404 | 413 => NetworkError::NonRetryable { message },
        // Unexpected but treat as non-retryable
        _ => NetworkError::NonRetryable { message },
    }
}

/// Parses error body to a user-friendly message, extracting status code, message and detail.
fn parse_error_body(status: reqwest::StatusCode, body: &str) -> String {
    // Try to parse as JSON first
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
        // Handle common error structures
        // 1. Nested { "error": { "message": "...", "detail": "...", "code": ... } }
        // 2. Flat { "message": "...", "detail": "..." }
        // 3. Simple { "error": "..." }

        let error_obj = val
            .get("error")
            .and_then(|e| e.as_object())
            .or_else(|| val.as_object());

        if let Some(obj) = error_obj {
            let msg = obj
                .get("message")
                .and_then(|m| m.as_str())
                .or_else(|| obj.get("error").and_then(|e| e.as_str()))
                .or_else(|| val.get("error").and_then(|e| e.as_str()))
                .unwrap_or_else(|| status.canonical_reason().unwrap_or("Unknown"));

            let mut detail = obj
                .get("detail")
                .and_then(|d| d.as_str())
                .or_else(|| obj.get("cause").and_then(|c| c.as_str()))
                .map(|s| s.to_string());

            // If detail/cause is itself a JSON string, try to parse it
            if let Some(d) = detail.as_ref()
                && let Ok(inner_val) = serde_json::from_str::<serde_json::Value>(d)
                && let Some(inner_detail) = inner_val.get("detail").and_then(|m| m.as_str())
            {
                detail = Some(inner_detail.to_string());
            }

            let mut result = format!("HTTP {} {}", status.as_u16(), msg);
            if let Some(d) = detail {
                result.push_str(&format!(" ({})", d));
            }
            return result;
        }
    }

    // Fallback if not JSON or unexpected structure
    if !body.is_empty() && body.len() < 200 {
        format!(
            "HTTP {} {}: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown"),
            body
        )
    } else {
        format!(
            "HTTP {} {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown")
        )
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
    // Try to downcast to reqwest::Error (transport-level errors)
    if let Some(reqwest_error) = error.downcast_ref::<reqwest::Error>() {
        return classify_reqwest_error(reqwest_error);
    }

    // Try to downcast to NetworkError (HTTP status errors from provider modules)
    if let Some(network_error) = error.downcast_ref::<NetworkError>() {
        return network_error.clone();
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
