//! Exa search provider.
//!
//! Uses the Exa MCP endpoint (`https://mcp.exa.ai/mcp`) via JSON-RPC over SSE.
//! No API key is required for the public endpoint.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::Client;
use reqwest::header::ACCEPT;
use serde_json::json;

use crate::config::AuthStore;
use crate::config::WebSearchProviderConfig;
use crate::tooling::builtin::web::SearchProvider;

const EXA_URL: &str = "https://mcp.exa.ai/mcp";
const SEARCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

pub struct ExaProvider;

#[async_trait]
impl SearchProvider for ExaProvider {
    fn name(&self) -> &'static str {
        "exa"
    }

    async fn search(
        &self,
        http: &Client,
        _auth: &AuthStore,
        provider_config: Option<&WebSearchProviderConfig>,
        query: &str,
        num_results: Option<i64>,
        search_type: Option<&str>,
        offset: Option<i64>,
    ) -> Result<String> {
        // Resolve the endpoint URL: provider_config -> env var -> default
        let exa_url = provider_config
            .and_then(|c| c.endpoint.as_deref())
            .map(|s| s.to_string())
            .or_else(|| std::env::var("WEBTOOLS_EXA_URL").ok())
            .unwrap_or_else(|| EXA_URL.to_string());

        let st = match search_type {
            Some("fast") => "fast",
            Some("deep") => "deep",
            _ => "auto",
        };

        let offset = offset.unwrap_or(0).max(0);
        let base_num = num_results.unwrap_or(8);
        // Fetch extra results to account for offset
        let num_results_val = base_num + offset;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "web_search_exa",
                "arguments": {
                    "query": query,
                    "type": st,
                    "numResults": num_results_val,
                    "livecrawl": "fallback",
                    "contextMaxCharacters": null,
                }
            }
        });

        let body = tokio::time::timeout(SEARCH_TIMEOUT, async {
            let response = http
                .post(exa_url)
                .header(ACCEPT, "application/json, text/event-stream")
                .json(&payload)
                .send()
                .await
                .context("failed to send Exa search request")?;

            if !response.status().is_success() {
                bail!(
                    "Exa search request failed with status {}",
                    response.status()
                );
            }

            response
                .text()
                .await
                .context("failed to read Exa search response")
        })
        .await
        .context("Exa search request timed out")??;

        let text = parse_exa_sse(&body)?.unwrap_or_else(|| {
            "No search results found. Please try a different query.".to_string()
        });

        // Apply result-level offset by skipping the first N result items
        if offset > 0 {
            Ok(skip_formatted_results(&text, offset as usize))
        } else {
            Ok(text)
        }
    }
}

/// Parse an Exa SSE stream and extract the text content from the first `result.content[0].text`.
fn parse_exa_sse(body: &str) -> Result<Option<String>> {
    for line in body.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };

        let data = data.trim();
        if data.is_empty() {
            continue;
        }

        let value: serde_json::Value =
            serde_json::from_str(data).with_context(|| "failed to parse Exa SSE payload")?;

        if let Some(text) = value
            .get("result")
            .and_then(|v| v.get("content"))
            .and_then(serde_json::Value::as_array)
            .and_then(|content| content.first())
            .and_then(|item| item.get("text"))
            .and_then(serde_json::Value::as_str)
        {
            return Ok(Some(text.to_string()));
        }
    }

    Ok(None)
}

/// Skip the first `offset` search result items in formatted text.
///
/// Results are detected by lines matching the pattern `N. [Title](url)`.
/// The function finds the (offset+1)-th result and returns everything from
/// that line onward, renumbering results starting from 1.
fn skip_formatted_results(text: &str, offset: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut result_count = 0;
    let mut skip_to: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        // Detect numbered result lines: "N. [Title](...)"
        let is_result_start = {
            let trimmed = line.trim();
            let bytes = trimmed.as_bytes();
            if bytes.is_empty() {
                false
            } else {
                let mut pos = 0;
                // Skip leading digits
                while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                    pos += 1;
                }
                // Must be followed by ". [" — the start of a markdown link
                pos < bytes.len() && bytes[pos..].starts_with(b". [")
            }
        };

        if is_result_start {
            if result_count == offset {
                skip_to = Some(i);
                break;
            }
            result_count += 1;
        }
    }

    match skip_to {
        Some(start) => {
            // Renumber results starting from 1
            let mut new_num = 1;
            let mut output = String::new();
            for line in lines[start..].iter() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    let bytes = trimmed.as_bytes();
                    let mut pos = 0;
                    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                        pos += 1;
                    }
                    if pos > 0 && pos < bytes.len() && bytes[pos..].starts_with(b". [") {
                        // This is a result line — renumber it
                        output.push_str(&format!("{}. {}\n", new_num, &trimmed[pos + 2..]));
                        new_num += 1;
                    } else {
                        output.push_str(line);
                        output.push('\n');
                    }
                } else {
                    output.push('\n');
                }
            }
            output.trim().to_string()
        }
        None => format!(
            "No more results (offset {} exceeds available results)",
            offset
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skip_formatted_results_skips_first_result() {
        let text = "1. [First](https://example.com)\n   Description one\n\n2. [Second](https://example2.com)\n   Description two\n\n3. [Third](https://example3.com)\n   Description three";
        let result = skip_formatted_results(text, 1);
        assert!(result.contains("[Second]"));
        assert!(!result.contains("[First]"));
        assert!(result.starts_with("1."));
    }

    #[test]
    fn test_skip_formatted_results_skips_all_when_offset_too_large() {
        let text = "1. [Only](https://example.com)\n   Description";
        let result = skip_formatted_results(text, 5);
        assert!(result.contains("No more results"));
    }

    #[test]
    fn test_skip_formatted_results_zero_offset_returns_full() {
        let text = "1. [First](https://example.com)\n   Description\n\n2. [Second](https://example2.com)\n   Description";
        let result = skip_formatted_results(text, 0);
        assert_eq!(result, text.trim());
    }
}
