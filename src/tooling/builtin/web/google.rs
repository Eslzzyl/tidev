//! Google Custom Search provider.
//!
//! Uses the Google Custom Search JSON API.
//! Requires an API key and a Search Engine ID (cx).
//! Free tier: 100 queries/day.
//! https://developers.google.com/custom-search/v1/overview

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::Client;
use url::Url;

use crate::config::AuthStore;
use crate::config::WebSearchProviderConfig;
use crate::tooling::builtin::web::SearchProvider;

const GOOGLE_URL: &str = "https://www.googleapis.com/customsearch/v1";

pub struct GoogleProvider;

#[async_trait]
impl SearchProvider for GoogleProvider {
    fn name(&self) -> &'static str {
        "google"
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
        let api_key = auth.search_api_key("google").ok_or_else(|| {
            anyhow::anyhow!(
                "Google Custom Search requires an API key. \
                     Set it in ~/.local/share/tidev/auth.json under \
                     `web.search_api_keys.google`."
            )
        })?;

        let cx = auth.google_cx().ok_or_else(|| {
            anyhow::anyhow!(
                "Google Custom Search requires a Search Engine ID (cx). \
                     Set it in ~/.local/share/tidev/auth.json under \
                     `web.google_cx`."
            )
        })?;

        // Google allows max 10 results per request.
        let num = num_results.unwrap_or(8).min(10).max(1);

        let mut params: Vec<(&str, String)> = vec![
            ("key", api_key.to_string()),
            ("cx", cx.to_string()),
            ("q", query.to_string()),
            ("num", num.to_string()),
        ];

        // "fast" → sort by date
        if search_type == Some("fast") {
            params.push(("sort", "date".to_string()));
        }

        let url = Url::parse_with_params(GOOGLE_URL, &params)
            .context("failed to build Google Custom Search URL")?;

        let response = http
            .get(url.as_str())
            .send()
            .await
            .context("failed to send Google Custom Search request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Google Custom Search request failed with status {status}: {body}");
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("failed to parse Google Custom Search response")?;

        format_google_results(&body)
    }
}

fn format_google_results(body: &serde_json::Value) -> Result<String> {
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .map(|arr| arr.as_slice())
        .unwrap_or_default();

    if items.is_empty() {
        return Ok("No search results found. Please try a different query.".to_string());
    }

    // Include search metadata if present
    let mut output = String::new();
    if let Some(info) = body.get("searchInformation")
        && let Some(total) = info.get("totalResults").and_then(|v| v.as_str())
    {
        output.push_str(&format!("Total results: {}\n\n", total));
    }

    for (i, item) in items.iter().enumerate() {
        let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let link = item.get("link").and_then(|v| v.as_str()).unwrap_or("");
        let snippet = item.get("snippet").and_then(|v| v.as_str()).unwrap_or("");

        output.push_str(&format!(
            "{}. [{}]({})\n   {}\n\n",
            i + 1,
            title,
            link,
            snippet
        ));
    }

    if output.is_empty() {
        Ok("No search results found.".to_string())
    } else {
        Ok(output.trim_end().to_string())
    }
}
