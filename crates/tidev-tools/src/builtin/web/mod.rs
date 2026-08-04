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
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::types::{ToolDefinition, ToolPermission};
use crate::types::{WebFetchArgs as WebFetchToolArgs, WebSearchArgs as WebSearchToolArgs};
use tidev_config::AuthStore;
use tidev_config::{WebSearchConfig, WebSearchProviderConfig};

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

/// Parameters for a web search operation.
#[derive(Debug, Clone)]
pub struct SearchParams<'a> {
    pub http: &'a Client,
    pub auth: &'a AuthStore,
    pub provider_config: Option<&'a WebSearchProviderConfig>,
    pub query: &'a str,
    pub num_results: Option<i64>,
    pub search_type: Option<&'a str>,
    pub offset: Option<i64>,
}

/// A single web search provider.
#[async_trait]
pub trait SearchProvider: Send + Sync {
    /// Human-readable provider name (e.g. "exa", "brave").
    fn name(&self) -> &'static str;

    /// Execute a search and return a formatted text result.
    async fn search(&self, params: SearchParams<'_>) -> Result<String>;
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
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown search provider '{}'; available: {}",
                    {
                        let mut names: Vec<&str> = self.providers.keys().copied().collect();
                        names.sort();
                        names.join(", ")
                    },
                    self.default
                )
            })
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

/// Shared tokio runtime used by the sync wrapper.
///
/// Avoids constructing a new runtime on every call (the old per-call pattern).
static WEB_RT: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build web tools runtime")
});

/// Execute a `websearch` or `webfetch` tool call asynchronously.
///
/// Unlike the sync wrapper, this does not create or rely on a nested runtime.
/// Call from an async context (e.g. `execute_tool_call_streaming`).
pub async fn execute_tool_call_async(
    tool_name: &str,
    arguments: Value,
    web_search_config: &WebSearchConfig,
    auth_store: &AuthStore,
) -> Result<String> {
    match tidev_utils::tool_name::canonical_tool_name(tool_name) {
        Some("websearch") => {
            let args = serde_json::from_value::<WebSearchToolArgs>(arguments)?;
            execute_search(args, web_search_config, auth_store).await
        }
        Some("webfetch") => {
            let args = serde_json::from_value::<WebFetchToolArgs>(arguments)?;
            fetch::fetch(args).await
        }
        Some(other) => bail!("unsupported web tool '{}'", other),
        None => bail!("unknown tool '{}'", tool_name),
    }
}

/// Execute a `websearch` or `webfetch` tool call synchronously.
///
/// Uses a shared [`LazyLock`] runtime so no per-call runtime is constructed.
/// Intended for the sync dispatch path (`ToolRegistry::execute` via
/// `spawn_blocking`).
pub fn execute_tool_call(
    _workspace_root: &std::path::Path,
    tool_name: &str,
    arguments: Value,
    _max_output_bytes: usize,
    web_search_config: &WebSearchConfig,
    auth_store: &AuthStore,
) -> Result<String> {
    WEB_RT.block_on(execute_tool_call_async(
        tool_name,
        arguments,
        web_search_config,
        auth_store,
    ))
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
        .search(SearchParams {
            http: &http,
            auth,
            provider_config,
            query,
            num_results: args.num_results,
            search_type: args.search_type.as_deref(),
            offset: args.offset,
        })
        .await
}
