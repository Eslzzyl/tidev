//! Tavily Search provider.
//!
//! Uses the Tavily Search API (designed for AI agents).
//! Requires an API key stored in `auth.json` under `web.search_api_keys.tavily`.
//! Free tier: 1,000 requests/month.
//! https://docs.tavily.com/

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

use crate::config::AuthStore;
use crate::config::WebSearchProviderConfig;
use crate::tooling::builtin::web::SearchProvider;

const TAVILY_URL: &str = "https://api.tavily.com/search";

pub struct TavilyProvider;

#[async_trait]
impl SearchProvider for TavilyProvider {
    fn name(&self) -> &'static str {
        "tavily"
    }

    async fn search(
        &self,
        http: &Client,
        auth: &AuthStore,
        _provider_config: Option<&WebSearchProviderConfig>,
        query: &str,
        num_results: Option<i64>,
        search_type: Option<&str>,
    ) -> Result<String> {
        let api_key = auth.search_api_key("tavily").ok_or_else(|| {
            anyhow::anyhow!(
                "Tavily Search requires an API key. \
                     Set it in ~/.local/share/tidev/auth.json under \
                     `web.search_api_keys.tavily`."
            )
        })?;

        // "fast" → basic depth, otherwise advanced
        let depth = match search_type {
            Some("fast") => "basic",
            _ => "advanced",
        };

        let max_results = num_results.unwrap_or(8).min(20).max(1);

        let payload = json!({
            "api_key": api_key,
            "query": query,
            "search_depth": depth,
            "max_results": max_results,
            "include_answer": true,
        });

        let response = http
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

        format_tavily_results(&body)
    }
}

fn format_tavily_results(body: &serde_json::Value) -> Result<String> {
    let mut output = String::new();

    // Tavily includes a human-readable answer
    if let Some(answer) = body.get("answer").and_then(|v| v.as_str())
        && !answer.is_empty()
    {
        output.push_str(&format!("Summary: {}\n\n", answer));
    }

    let results = body
        .get("results")
        .and_then(|v| v.as_array())
        .map(|arr| arr.as_slice())
        .unwrap_or_default();

    if results.is_empty() {
        return Ok("No search results found. Please try a different query.".to_string());
    }

    for (i, item) in results.iter().enumerate() {
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
            i + 1,
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
