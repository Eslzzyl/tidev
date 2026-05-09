use anyhow::{Context, Result, bail};
use pulldown_cmark::{Event, Options as MarkdownOptions, Parser as MarkdownParser, Tag, TagEnd};
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::timeout;
use url::Url;

use crate::tooling::tools::{WebFetchArgs as WebFetchToolArgs, WebSearchArgs as WebSearchToolArgs};
use crate::tooling::{ToolDefinition, ToolPermission};

const EXA_URL: &str = "https://mcp.exa.ai/mcp";
const SEARCH_TIMEOUT: Duration = Duration::from_secs(25);
const FETCH_DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_MAX_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESPONSE_BYTES: usize = 5 * 1024 * 1024;

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::new::<WebSearchToolArgs>(
            "websearch",
            "Search the web using Exa and return a concise text summary.",
            ToolPermission::Search,
        ),
        ToolDefinition::new::<WebFetchToolArgs>(
            "webfetch",
            "Fetch a web page as text, markdown, or HTML.",
            ToolPermission::Read,
        ),
    ]
}

pub fn execute_tool_call(
    _workspace_root: &std::path::Path,
    tool_name: &str,
    arguments: Value,
    _max_output_bytes: usize,
) -> Result<String> {
    match crate::tooling::canonical_tool_name(tool_name) {
        Some("websearch") => {
            let args = serde_json::from_value::<SearchArgs>(arguments).map_err(|e| {
                anyhow::anyhow!("failed to decode arguments for tool '{}': {}", tool_name, e)
            })?;
            run_webtools(async { WebToolsClient::new()?.search(args).await })
        }
        Some("webfetch") => {
            let args = serde_json::from_value::<FetchArgs>(arguments).map_err(|e| {
                anyhow::anyhow!("failed to decode arguments for tool '{}': {}", tool_name, e)
            })?;
            run_webtools(async { WebToolsClient::new()?.fetch(args).await })
        }
        Some(other) => bail!("unsupported web tool '{}'", other),
        None => bail!("unknown tool '{}'", tool_name),
    }
}

struct WebToolsClient {
    http: Client,
    exa_url: String,
}

impl WebToolsClient {
    fn new() -> Result<Self> {
        let http = Client::builder()
            .user_agent("tidev-webtools/0.1")
            .build()
            .context("failed to construct web tools HTTP client")?;

        let exa_url = std::env::var("WEBTOOLS_EXA_URL").unwrap_or_else(|_| EXA_URL.to_string());

        Ok(Self { http, exa_url })
    }

    async fn search(&self, args: SearchArgs) -> Result<String> {
        let query = args.query.trim();
        if query.is_empty() {
            bail!("query cannot be empty");
        }

        let search_type = match args.search_type.as_deref() {
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
                    "type": search_type,
                    "numResults": args.num_results.unwrap_or(8),
                    "livecrawl": "fallback",
                    "contextMaxCharacters": null,
                }
            }
        });

        let body = timeout(SEARCH_TIMEOUT, async {
            let response = self
                .http
                .post(&self.exa_url)
                .header(ACCEPT, "application/json, text/event-stream")
                .json(&payload)
                .send()
                .await
                .context("failed to send web search request")?;

            if !response.status().is_success() {
                bail!(
                    "web search request failed with status {}",
                    response.status()
                );
            }

            response
                .text()
                .await
                .context("failed to read web search response")
        })
        .await
        .context("web search request timed out")??;

        let text = parse_exa_sse(&body)?.unwrap_or_else(|| {
            "No search results found. Please try a different query.".to_string()
        });

        Ok(text)
    }

    async fn fetch(&self, args: FetchArgs) -> Result<String> {
        let url = validate_url(&args.url)?;
        let format = match args.format.as_deref() {
            Some("text") => WebFetchFormat::Text,
            Some("html") => WebFetchFormat::Html,
            _ => WebFetchFormat::Markdown,
        };

        let timeout_secs = args
            .timeout
            .unwrap_or(FETCH_DEFAULT_TIMEOUT.as_secs())
            .min(FETCH_MAX_TIMEOUT.as_secs());
        let duration = Duration::from_secs(timeout_secs);
        let headers = fetch_headers(format);

        let response = timeout(duration, self.fetch_response(&url, headers.clone())).await??;

        let mime = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("text/plain")
            .to_ascii_lowercase();

        if let Some(length) = response.content_length()
            && length > MAX_RESPONSE_BYTES as u64
        {
            bail!("response too large (exceeds 5MB limit)");
        }

        let bytes = response
            .bytes()
            .await
            .context("failed to read response body")?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            bail!("response too large (exceeds 5MB limit)");
        }

        if is_image_mime(&mime) {
            return Ok(format!("Image fetched successfully ({})", mime));
        }

        let body = String::from_utf8_lossy(&bytes).into_owned();
        let output = match format {
            WebFetchFormat::Html => body,
            WebFetchFormat::Markdown => {
                if mime.contains("html") {
                    html2md::rewrite_html(&body, false)
                } else {
                    body
                }
            }
            WebFetchFormat::Text => {
                if mime.contains("html") {
                    markdown_to_text(&html2md::rewrite_html(&body, false))
                } else {
                    body
                }
            }
        };

        Ok(output)
    }

    async fn fetch_response(&self, url: &Url, headers: HeaderMap) -> Result<reqwest::Response> {
        let response = self
            .http
            .get(url.clone())
            .headers(headers.clone())
            .send()
            .await
            .context("failed to send fetch request")?;

        if response.status() == StatusCode::FORBIDDEN
            && response
                .headers()
                .get("cf-mitigated")
                .and_then(|value| value.to_str().ok())
                == Some("challenge")
        {
            let mut retry = headers;
            retry.insert(USER_AGENT, HeaderValue::from_static("opencode"));
            return self
                .http
                .get(url.clone())
                .headers(retry)
                .send()
                .await
                .context("failed to retry fetch request");
        }

        if !response.status().is_success() {
            bail!("fetch request failed with status {}", response.status());
        }

        Ok(response)
    }
}

fn run_webtools<T>(future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to construct webtools runtime")?;

    runtime.block_on(future)
}

#[derive(Clone, Debug, Deserialize)]
struct SearchArgs {
    query: String,
    num_results: Option<i64>,
    search_type: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct FetchArgs {
    url: String,
    format: Option<String>,
    timeout: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WebFetchFormat {
    Text,
    Markdown,
    Html,
}

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
            .and_then(|value| value.get("content"))
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

fn fetch_headers(format: WebFetchFormat) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36",
        ),
    );
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));

    let accept = match format {
        WebFetchFormat::Markdown => {
            "text/markdown;q=1.0, text/x-markdown;q=0.9, text/plain;q=0.8, text/html;q=0.7, */*;q=0.1"
        }
        WebFetchFormat::Text => "text/plain;q=1.0, text/markdown;q=0.9, text/html;q=0.8, */*;q=0.1",
        WebFetchFormat::Html => {
            "text/html;q=1.0, application/xhtml+xml;q=0.9, text/plain;q=0.8, text/markdown;q=0.7, */*;q=0.1"
        }
    };
    headers.insert(ACCEPT, HeaderValue::from_static(accept));
    headers
}

fn validate_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).with_context(|| format!("invalid URL '{value}'"))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        _ => bail!("URL must start with http:// or https://"),
    }
}

fn is_image_mime(mime: &str) -> bool {
    mime.starts_with("image/") && mime != "image/svg+xml"
}

fn markdown_to_text(markdown: &str) -> String {
    let mut output = String::new();
    let mut options = MarkdownOptions::empty();
    options.insert(MarkdownOptions::ENABLE_STRIKETHROUGH);
    options.insert(MarkdownOptions::ENABLE_TABLES);

    let mut in_code_block = false;
    for event in MarkdownParser::new_ext(markdown, options) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                if !output.ends_with('\n') {
                    output.push('\n');
                }
                in_code_block = false;
            }
            Event::Start(tag)
                if is_block_tag(&tag) && !output.is_empty() && !output.ends_with('\n') =>
            {
                output.push('\n');
            }
            Event::End(tag_end) if is_block_tag_end(&tag_end) && !output.ends_with('\n') => {
                output.push('\n');
            }
            Event::Text(text)
            | Event::Code(text)
            | Event::Html(text)
            | Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text) => {
                append_text_segment(&mut output, &text, in_code_block);
            }
            Event::SoftBreak | Event::HardBreak if !output.ends_with('\n') => {
                output.push('\n');
            }
            _ => {}
        }
    }

    normalize_plain_text(output)
}

fn append_text_segment(output: &mut String, text: &str, in_code_block: bool) {
    if in_code_block {
        output.push_str(text);
        return;
    }

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }

    if matches!(output.chars().last(), Some(last) if !last.is_whitespace()) {
        output.push(' ');
    }

    output.push_str(trimmed);
}

fn normalize_plain_text(text: String) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut previous_blank_line = false;

    for line in text.lines().map(str::trim_end) {
        let is_blank = line.trim().is_empty();
        if is_blank {
            if !previous_blank_line && !normalized.is_empty() {
                normalized.push('\n');
            }
            previous_blank_line = true;
            continue;
        }

        if !normalized.is_empty() && !normalized.ends_with('\n') {
            normalized.push('\n');
        }
        normalized.push_str(line.trim());
        previous_blank_line = false;
    }

    normalized.trim().to_string()
}

fn is_block_tag(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(_)
            | Tag::HtmlBlock
            | Tag::List(_)
            | Tag::Item
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell
            | Tag::MetadataBlock(_)
    )
}

fn is_block_tag_end(tag: &TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::HtmlBlock
            | TagEnd::List(_)
            | TagEnd::Item
            | TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Table
            | TagEnd::TableHead
            | TagEnd::TableRow
            | TagEnd::TableCell
            | TagEnd::MetadataBlock(_)
    )
}
