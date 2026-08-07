use super::types::ResponseStreamResponse;
use crate::error::NetworkError;

pub(super) fn response_error_details(
    response: &ResponseStreamResponse,
) -> (String, Option<String>) {
    response
        .error
        .as_ref()
        .map(|error| {
            let message = if error.message.is_empty() {
                "Responses API returned a failed response".to_string()
            } else {
                error.message.clone()
            };
            let code = if error.code.is_empty() {
                (!error.r#type.is_empty()).then(|| error.r#type.clone())
            } else {
                Some(error.code.clone())
            };
            (message, code)
        })
        .unwrap_or_else(|| ("Responses API returned a failed response".to_string(), None))
}

pub(super) fn classify_responses_stream_error(
    message: String,
    code: Option<String>,
) -> NetworkError {
    let searchable = format!(
        "{} {}",
        code.as_deref().unwrap_or_default().to_ascii_lowercase(),
        message.to_ascii_lowercase()
    );
    let retryable = [
        "rate_limit",
        "rate limit",
        "server_error",
        "server error",
        "overloaded",
        "temporarily unavailable",
        "timeout",
        "timed out",
        "try again",
    ]
    .iter()
    .any(|marker| searchable.contains(marker));

    if retryable {
        NetworkError::Retryable { message }
    } else {
        NetworkError::NonRetryable { message }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_error_classification_retries_transient_errors() {
        assert!(
            classify_responses_stream_error(
                "server overloaded".to_string(),
                Some("server_error".to_string())
            )
            .is_retryable()
        );
        assert!(
            !classify_responses_stream_error(
                "invalid prompt".to_string(),
                Some("invalid_request_error".to_string())
            )
            .is_retryable()
        );
    }
}
