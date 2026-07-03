//! Tavily Search provider.
//!
//! Uses the Tavily Search API (designed for AI agents).
//! Requires an API key stored in `auth.json` under `web.search_api_keys.tavily`.
//! Free tier: 1,000 requests/month.
//! https://docs.tavily.com/

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::json;

use super::{SearchParams, SearchProvider};

const TAVILY_URL: &str = "https://api.tavily.com/search";

pub struct TavilyProvider;

#[async_trait]
impl SearchProvider for TavilyProvider {
    fn name(&self) -> &'static str {
        "tavily"
    }

    async fn search(&self, params: SearchParams<'_>) -> Result<String> {
        let api_key = params.auth.api_key("tavily").ok_or_else(|| {
            anyhow::anyhow!(
                "Tavily Search requires an API key. \
                     Set it in ~/.local/share/tidev/auth.json under \
                     `web.search_api_keys.tavily`."
            )
        })?;

        // "fast" → basic depth, otherwise advanced
        let depth = match params.search_type {
            Some("fast") => "basic",
            _ => "advanced",
        };

        let offset = params.offset.unwrap_or(0).max(0);
        let base_num = params.num_results.unwrap_or(8);
        // Fetch extra results to account for offset
        let max_results = (base_num + offset).clamp(1, 20);

        let payload = json!({
            "api_key": api_key,
            "query": params.query,
            "search_depth": depth,
            "max_results": max_results,
            "include_answer": true,
        });

        let response = params
            .http
            .post(TAVILY_URL)
            .json(&payload)
            .send()
            .await
            .context("failed to send Tavily Search request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Tavily Search request failed with status {status}: {body}");
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("failed to parse Tavily Search response")?;

        format_tavily_results(&body, offset as usize)
    }
}

fn format_tavily_results(body: &serde_json::Value, offset: usize) -> Result<String> {
    let mut output = String::new();

    // Tavily includes a human-readable answer (only show on first page)
    if offset == 0
        && let Some(answer) = body.get("answer").and_then(|v| v.as_str())
        && !answer.is_empty()
    {
        output.push_str(&format!("Summary: {}\n\n", answer));
    }

    let results = body
        .get("results")
        .and_then(|v| v.as_array())
        .map(|arr| arr.as_slice())
        .unwrap_or_default();

    if results.is_empty() || offset >= results.len() {
        return Ok("No search results found. Please try a different query.".to_string());
    }

    for (i, item) in results.iter().enumerate().skip(offset) {
        let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let content = item.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let score = item
            .get("score")
            .and_then(|v| v.as_f64())
            .map(|s| format!(" (relevance: {:.2})", s))
            .unwrap_or_default();

        output.push_str(&format!(
            "{}. [{}]({}){}\n   {}\n\n",
            i + 1 - offset,
            title,
            url,
            score,
            content
        ));
    }

    if output.is_empty() {
        Ok("No search results found.".to_string())
    } else {
        Ok(output.trim_end().to_string())
    }
}
