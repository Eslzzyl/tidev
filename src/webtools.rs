use anyhow::{Context, Result, bail};
use base64::Engine as _;
use pulldown_cmark::{Event, Options as MarkdownOptions, Parser as MarkdownParser, Tag, TagEnd};
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT};
use reqwest::{Client, StatusCode};
use rmcp::ErrorData as McpError;
use rmcp::ServiceExt;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, JsonObject, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use url::Url;

const EXA_URL: &str = "https://mcp.exa.ai/mcp";
const SEARCH_TOOL_NAME: &str = "websearch";
const FETCH_TOOL_NAME: &str = "webfetch";
const SEARCH_TIMEOUT: Duration = Duration::from_secs(25);
const FETCH_DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_MAX_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESPONSE_BYTES: usize = 5 * 1024 * 1024;

pub async fn run() -> Result<()> {
    let service = WebToolsServer::new()?;
    let running = service.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}

pub async fn websearch(
    query: &str,
    num_results: Option<i64>,
    search_type: Option<&str>,
) -> Result<String> {
    let service = WebToolsServer::new()?;
    let args = SearchArgs {
        query: query.to_string(),
        num_results: num_results.map(|v| v as u64),
        livecrawl: None,
        search_type: search_type.map(|s| match s {
            "fast" => SearchType::Fast,
            "deep" => SearchType::Deep,
            _ => SearchType::Auto,
        }),
        context_max_characters: None,
    };
    let result = service.search(args).await?;
    result
        .content
        .first()
        .and_then(|c| c.raw.as_text())
        .map(|t| t.text.clone())
        .ok_or_else(|| anyhow::anyhow!("websearch returned no content"))
}

pub async fn webfetch(url: &str, format: Option<&str>, timeout: Option<i64>) -> Result<String> {
    let service = WebToolsServer::new()?;
    let args = FetchArgs {
        url: url.to_string(),
        format: format.map(|f| match f {
            "text" => WebFetchFormat::Text,
            "html" => WebFetchFormat::Html,
            _ => WebFetchFormat::Markdown,
        }),
        timeout: timeout.map(|v| v as u64),
    };
    let result = service.fetch(args).await?;
    result
        .content
        .first()
        .and_then(|c| c.raw.as_text())
        .map(|t| t.text.clone())
        .ok_or_else(|| anyhow::anyhow!("webfetch returned no content"))
}

fn stdio() -> (tokio::io::Stdin, tokio::io::Stdout) {
    (tokio::io::stdin(), tokio::io::stdout())
}

#[derive(Clone)]
struct WebToolsServer {
    http: Client,
    exa_url: String,
    tools: Arc<Vec<Tool>>,
}

impl WebToolsServer {
    fn new() -> Result<Self> {
        let http = Client::builder()
            .user_agent("tidev-webtools/0.1")
            .build()
            .context("failed to construct webtools HTTP client")?;
        let exa_url = std::env::var("WEBTOOLS_EXA_URL").unwrap_or_else(|_| EXA_URL.to_string());

        Ok(Self {
            http,
            exa_url,
            tools: Arc::new(vec![Self::websearch_tool(), Self::webfetch_tool()]),
        })
    }

    fn websearch_tool() -> Tool {
        let schema = json_schema(json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Web search query"
                },
                "numResults": {
                    "type": "integer",
                    "description": "Number of search results to return (default: 8)"
                },
                "livecrawl": {
                    "type": "string",
                    "enum": ["fallback", "preferred"],
                    "description": "Live crawl mode"
                },
                "type": {
                    "type": "string",
                    "enum": ["auto", "fast", "deep"],
                    "description": "Search type"
                },
                "contextMaxCharacters": {
                    "type": "integer",
                    "description": "Maximum characters for the context string"
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }));

        let mut tool = Tool::new(
            Cow::Borrowed(SEARCH_TOOL_NAME),
            Cow::Borrowed("Search the web using Exa and return a concise text summary."),
            Arc::new(schema),
        );
        tool.annotations = Some(ToolAnnotations::new().read_only(true));
        tool
    }

    fn webfetch_tool() -> Tool {
        let schema = json_schema(json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "markdown", "html"],
                    "default": "markdown",
                    "description": "The output format"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (max 120)"
                }
            },
            "required": ["url"],
            "additionalProperties": false
        }));

        let mut tool = Tool::new(
            Cow::Borrowed(FETCH_TOOL_NAME),
            Cow::Borrowed("Fetch a web page as text, markdown, or HTML."),
            Arc::new(schema),
        );
        tool.annotations = Some(ToolAnnotations::new().read_only(true));
        tool
    }

    async fn search(&self, args: SearchArgs) -> Result<CallToolResult> {
        let query = args.query.trim();
        if query.is_empty() {
            bail!("query cannot be empty");
        }

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "web_search_exa",
                "arguments": {
                    "query": query,
                    "type": args.search_type.unwrap_or_default(),
                    "numResults": args.num_results.unwrap_or(8),
                    "livecrawl": args.livecrawl.unwrap_or_default(),
                    "contextMaxCharacters": args.context_max_characters,
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

        Ok(CallToolResult::success(vec![rmcp::model::Content::text(
            text,
        )]))
    }

    async fn fetch(&self, args: FetchArgs) -> Result<CallToolResult> {
        let url = validate_url(&args.url)?;
        let format = args.format.unwrap_or(WebFetchFormat::Markdown);
        let timeout_secs = args
            .timeout
            .unwrap_or(FETCH_DEFAULT_TIMEOUT.as_secs())
            .min(FETCH_MAX_TIMEOUT.as_secs());
        let duration = Duration::from_secs(timeout_secs);
        let headers = fetch_headers(format);

        let response = timeout(duration, self.fetch_response(&url, headers)).await??;
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
            let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return Ok(CallToolResult::success(vec![
                rmcp::model::Content::text("Image fetched successfully"),
                rmcp::model::Content::image(encoded, mime),
            ]));
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

        Ok(CallToolResult::success(vec![rmcp::model::Content::text(
            output,
        )]))
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

impl ServerHandler for WebToolsServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_tool_list_changed()
            .build();
        info
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let tools = self.tools.clone();
        async move {
            Ok(ListToolsResult {
                tools: (*tools).clone(),
                next_cursor: None,
                meta: None,
            })
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        match request.name.as_ref() {
            SEARCH_TOOL_NAME => {
                let args = parse_call_args::<SearchArgs>(&request, SEARCH_TOOL_NAME)?;
                self.search(args).await.map_err(internal_error)
            }
            FETCH_TOOL_NAME => {
                let args = parse_call_args::<FetchArgs>(&request, FETCH_TOOL_NAME)?;
                self.fetch(args).await.map_err(internal_error)
            }
            other => Err(McpError::invalid_params(
                format!("unknown tool: {other}"),
                None,
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(rename = "numResults")]
    num_results: Option<u64>,
    livecrawl: Option<LivecrawlMode>,
    #[serde(rename = "type")]
    search_type: Option<SearchType>,
    #[serde(rename = "contextMaxCharacters")]
    context_max_characters: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
struct FetchArgs {
    url: String,
    format: Option<WebFetchFormat>,
    timeout: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum LivecrawlMode {
    #[default]
    Fallback,
    Preferred,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SearchType {
    #[default]
    Auto,
    Fast,
    Deep,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WebFetchFormat {
    Text,
    Markdown,
    Html,
}

fn parse_call_args<T: for<'de> Deserialize<'de>>(
    request: &CallToolRequestParams,
    tool: &'static str,
) -> Result<T, McpError> {
    let arguments = request.arguments.clone().unwrap_or_default();
    serde_json::from_value(serde_json::Value::Object(arguments.into_iter().collect())).map_err(
        |err| {
            McpError::invalid_params(
                format!("failed to decode arguments for {tool}: {err}"),
                None,
            )
        },
    )
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
            Event::Start(tag) if is_block_tag(&tag) => {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
            }
            Event::End(tag_end) if is_block_tag_end(&tag_end) => {
                if !output.ends_with('\n') {
                    output.push('\n');
                }
            }
            Event::Text(text)
            | Event::Code(text)
            | Event::Html(text)
            | Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text) => {
                append_text_segment(&mut output, &text, in_code_block);
            }
            Event::SoftBreak | Event::HardBreak => {
                if !output.ends_with('\n') {
                    output.push('\n');
                }
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

fn json_schema(value: serde_json::Value) -> JsonObject {
    serde_json::from_value(value).expect("valid JSON schema")
}

fn internal_error(err: anyhow::Error) -> McpError {
    McpError::internal_error(err.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn test_server(exa_url: impl Into<String>) -> WebToolsServer {
        WebToolsServer {
            http: Client::builder()
                .user_agent("tidev-webtools/0.1")
                .build()
                .expect("test client"),
            exa_url: exa_url.into(),
            tools: Arc::new(vec![
                WebToolsServer::websearch_tool(),
                WebToolsServer::webfetch_tool(),
            ]),
        }
    }

    async fn spawn_http_server(response: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");

        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buffer = [0u8; 4096];
                let _ = socket.read(&mut buffer).await;
                let _ = socket.write_all(&response).await;
            }
        });

        format!("http://{}", addr)
    }

    fn http_response(status: &str, headers: &[(&str, &str)], body: impl AsRef<[u8]>) -> Vec<u8> {
        let body = body.as_ref();
        let mut response = format!("HTTP/1.1 {status}\r\n");
        for (name, value) in headers {
            response.push_str(&format!("{name}: {value}\r\n"));
        }
        response.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        ));
        let mut bytes = response.into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }

    #[tokio::test]
    async fn discovers_web_tools() {
        let server = test_server("http://127.0.0.1");
        let info = server.get_info();

        assert!(info.capabilities.tools.is_some());
        assert_eq!(server.tools.len(), 2);
        assert!(
            server
                .tools
                .iter()
                .any(|tool| tool.name.as_ref() == SEARCH_TOOL_NAME)
        );
        assert!(
            server
                .tools
                .iter()
                .any(|tool| tool.name.as_ref() == FETCH_TOOL_NAME)
        );
    }

    #[tokio::test]
    async fn search_returns_result_text() {
        let body = "event: message\ndata: {\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"Rust search result\"}]}}\n";
        let url = spawn_http_server(http_response(
            "200 OK",
            &[("Content-Type", "text/event-stream")],
            body,
        ))
        .await;

        let server = test_server(url);
        let result = server
            .search(SearchArgs {
                query: "rust".to_string(),
                num_results: Some(3),
                livecrawl: None,
                search_type: None,
                context_max_characters: None,
            })
            .await
            .expect("search should succeed");

        let text = result
            .content
            .iter()
            .find_map(|content| content.as_text().map(|text| text.text.clone()))
            .expect("text content");
        assert!(text.contains("Rust search result"));
    }

    #[tokio::test]
    async fn fetch_returns_markdown_for_html() {
        let html = b"<h1>Hello</h1><p>World</p>".to_vec();
        let url = spawn_http_server(http_response(
            "200 OK",
            &[("Content-Type", "text/html; charset=utf-8")],
            html,
        ))
        .await;

        let server = test_server("http://127.0.0.1");
        let result = server
            .fetch(FetchArgs {
                url,
                format: Some(WebFetchFormat::Markdown),
                timeout: Some(5),
            })
            .await
            .expect("fetch should succeed");

        let text = result
            .content
            .iter()
            .find_map(|content| content.as_text().map(|text| text.text.clone()))
            .expect("text content");
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
    }

    #[tokio::test]
    async fn fetch_returns_image_content() {
        let url = spawn_http_server(http_response(
            "200 OK",
            &[("Content-Type", "image/png")],
            b"fake-png-bytes",
        ))
        .await;

        let server = test_server("http://127.0.0.1");
        let result = server
            .fetch(FetchArgs {
                url,
                format: Some(WebFetchFormat::Markdown),
                timeout: Some(5),
            })
            .await
            .expect("fetch should succeed");

        let image = result
            .content
            .iter()
            .find_map(|content| content.as_image())
            .expect("image content");
        assert_eq!(image.mime_type, "image/png");
        assert!(!image.data.is_empty());
    }

    #[tokio::test]
    async fn fetch_rejects_http_errors() {
        let url = spawn_http_server(http_response(
            "500 Internal Server Error",
            &[("Content-Type", "text/plain")],
            b"boom",
        ))
        .await;

        let server = test_server("http://127.0.0.1");
        let err = server
            .fetch(FetchArgs {
                url,
                format: Some(WebFetchFormat::Text),
                timeout: Some(5),
            })
            .await
            .expect_err("fetch should fail");
        assert!(err.to_string().contains("status 500"));
    }

    #[tokio::test]
    async fn search_rejects_empty_queries() {
        let server = test_server("http://127.0.0.1");
        let err = server
            .search(SearchArgs {
                query: "   ".to_string(),
                num_results: None,
                livecrawl: None,
                search_type: None,
                context_max_characters: None,
            })
            .await
            .expect_err("search should reject empty queries");
        assert!(err.to_string().contains("query cannot be empty"));
    }

    #[test]
    fn parses_exa_sse_payload() {
        let body = r#"
event: message
data: {"result":{"content":[{"type":"text","text":"hello"}]}}
"#;

        let text = parse_exa_sse(body)
            .expect("should parse")
            .expect("should contain text");
        assert_eq!(text, "hello");
    }

    #[test]
    fn rejects_non_http_urls() {
        let err = validate_url("file:///tmp/data").expect_err("should reject file URLs");
        assert!(err.to_string().contains("http:// or https://"));
    }

    #[test]
    fn converts_html_to_markdown() {
        let markdown = html2md::rewrite_html("<h1>Hello</h1><p>World</p>", false);
        assert!(markdown.contains("Hello"));
        assert!(markdown.contains("World"));
    }

    #[test]
    fn converts_markdown_to_plain_text() {
        let text = markdown_to_text("# Title\n\nHello **world**\n");
        assert!(text.contains("Title"));
        assert!(text.contains("Hello world"));
    }
}
