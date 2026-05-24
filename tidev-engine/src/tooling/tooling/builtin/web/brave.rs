//! Brave Search provider.
//!
//! Uses the Brave Search API via REST.
//! Requires an API key stored in `auth.json` under `web.search_api_keys.brave`.
//! Free tier: https://api.search.brave.com/

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use url::Url;

use crate::tooling::builtin::web::{SearchParams, SearchProvider};

const BRAVE_URL: &str = "https://api.search.brave.com/res/v1/web/search";

pub struct BraveProvider;

#[async_trait]
impl SearchProvider for BraveProvider {
    fn name(&self) -> &'static str {
        "brave"
    }

    async fn search(&self, params: SearchParams<'_>) -> Result<String> {
        let api_key = params.auth.search_api_key("brave").ok_or_else(|| {
            anyhow::anyhow!(
                "Brave Search requires an API key. \
                     Set it in ~/.local/share/tidev/auth.json under \
                     `web.search_api_keys.brave`."
            )
        })?;

        let offset = params.offset.unwrap_or(0).max(0);
        let base_num = params.num_results.unwrap_or(8);
        // Fetch extra results to account for offset
        let count = (base_num + offset).clamp(1, 20);

        let mut query_params = vec![
            ("q", params.query.to_string()),
            ("count", count.to_string()),
        ];

        if let Some("fast") = params.search_type {
            query_params.push(("freshness", "pw".to_string())); // past week
        }

        let url = Url::parse_with_params(BRAVE_URL, &query_params)
            .context("failed to build Brave Search URL")?;

        let response = params
            .http
            .get(url.as_str())
            .header("X-Subscription-Token", api_key)
            .send()
            .await
            .context("failed to send Brave Search request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Brave Search request failed with status {status}: {body}");
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("failed to parse Brave Search response")?;

        format_brave_results(&body, offset as usize)
    }
}

fn format_brave_results(body: &serde_json::Value, offset: usize) -> Result<String> {
    let results = body
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array())
        .map(|arr| arr.as_slice())
        .unwrap_or_default();

    if results.is_empty() || offset >= results.len() {
        return Ok("No search results found. Please try a different query.".to_string());
    }

    let mut output = String::new();
    for (i, item) in results.iter().enumerate().skip(offset) {
        let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let desc = item
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        output.push_str(&format!(
            "{}. [{}]({})\n   {}\n\n",
            i + 1 - offset,
            title,
            url,
            desc
        ));
    }

    if output.is_empty() {
        Ok("No search results found.".to_string())
    } else {
        Ok(output.trim_end().to_string())
    }
}
