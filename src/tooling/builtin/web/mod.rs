//! Web search and fetch tools.
//!
//! This module provides:
//! - `websearch` — multi-provider web search (Exa, Brave, Google, Tavily)
//! - `webfetch` — fetch and render a web page as text/markdown/html

pub mod brave;
pub mod exa;
pub mod fetch;
pub mod google;
pub mod tavily;

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::config::AuthStore;
use crate::config::{WebSearchConfig, WebSearchProviderConfig};
use crate::tooling::tools::{WebFetchArgs as WebFetchToolArgs, WebSearchArgs as WebSearchToolArgs};
use crate::tooling::{ToolDefinition, ToolPermission};

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

/// A single web search provider.
#[async_trait]
pub trait SearchProvider: Send + Sync {
    /// Human-readable provider name (e.g. "exa", "brave").
    fn name(&self) -> &'static str;

    /// Execute a search and return a formatted text result.
    async fn search(
        &self,
        http: &Client,
        auth: &AuthStore,
        provider_config: Option<&WebSearchProviderConfig>,
        query: &str,
        num_results: Option<i64>,
        search_type: Option<&str>,
    ) -> Result<String>;
}

// ---------------------------------------------------------------------------
// Provider registry
// ---------------------------------------------------------------------------

struct SearchRegistry {
    providers: HashMap<&'static str, Box<dyn SearchProvider>>,
    default: String,
}

impl SearchRegistry {
    fn new(default: &str) -> Self {
        Self {
            providers: HashMap::new(),
            default: default.to_string(),
        }
    }

    fn register(&mut self, provider: Box<dyn SearchProvider>) {
        self.providers.insert(provider.name(), provider);
    }

    fn resolve(&self) -> Result<&dyn SearchProvider> {
        self.providers
            .get(&self.default[..])
            .map(|p| p.as_ref())
            .ok_or_else(|| anyhow::anyhow!("unknown search provider '{}'; available: {}", {
                let mut names: Vec<&str> = self.providers.keys().copied().collect();
                names.sort();
                names.join(", ")
            }, self.default))
    }
}

fn build_registry(config: &WebSearchConfig) -> SearchRegistry {
    let mut r = SearchRegistry::new(&config.default_provider);
    r.register(Box::new(exa::ExaProvider));
    r.register(Box::new(brave::BraveProvider));
    r.register(Box::new(google::GoogleProvider));
    r.register(Box::new(tavily::TavilyProvider));
    r
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Tool definitions for `websearch` and `webfetch`.
pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::new::<WebSearchToolArgs>(
            "websearch",
            "Search the web and return a concise text summary.",
            ToolPermission::Search,
        ),
        ToolDefinition::new::<WebFetchToolArgs>(
            "webfetch",
            "Fetch a web page as text, markdown, or HTML.",
            ToolPermission::Read,
        ),
    ]
}

/// Execute a `websearch` or `webfetch` tool call synchronously.
///
/// This function creates a short-lived tokio runtime for async I/O.
pub fn execute_tool_call(
    _workspace_root: &std::path::Path,
    tool_name: &str,
    arguments: Value,
    _max_output_bytes: usize,
    web_search_config: &WebSearchConfig,
    auth_store: &AuthStore,
) -> Result<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to construct webtools runtime")?;

    match crate::tooling::canonical_tool_name(tool_name) {
        Some("websearch") => {
            let args =
                serde_json::from_value::<WebSearchToolArgs>(arguments).map_err(|e| {
                    anyhow::anyhow!(
                        "failed to decode arguments for tool '{}': {}",
                        tool_name,
                        e
                    )
                })?;
            runtime.block_on(execute_search(args, web_search_config, auth_store))
        }
        Some("webfetch") => {
            let args =
                serde_json::from_value::<WebFetchToolArgs>(arguments).map_err(|e| {
                    anyhow::anyhow!(
                        "failed to decode arguments for tool '{}': {}",
                        tool_name,
                        e
                    )
                })?;
            runtime.block_on(fetch::fetch(args))
        }
        Some(other) => bail!("unsupported web tool '{}'", other),
        None => bail!("unknown tool '{}'", tool_name),
    }
}

async fn execute_search(
    args: WebSearchToolArgs,
    config: &WebSearchConfig,
    auth: &AuthStore,
) -> Result<String> {
    let query = args.query.trim();
    if query.is_empty() {
        bail!("query cannot be empty");
    }

    let registry = build_registry(config);
    let provider = registry.resolve()?;

    let provider_config = config.providers.get(provider.name());

    let http = Client::builder()
        .user_agent("tidev-webtools/0.1")
        .build()
        .context("failed to construct web tools HTTP client")?;

    provider
        .search(&http, auth, provider_config, query, args.num_results, args.search_type.as_deref())
        .await
}
