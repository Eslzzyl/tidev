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

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "web_search_exa",
                "arguments": {
                    "query": query,
                    "type": st,
                    "numResults": num_results.unwrap_or(8),
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

        Ok(text)
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
