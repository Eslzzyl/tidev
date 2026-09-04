//! Web page fetch tool.
//!
//! Fetches a URL and returns its content as text, markdown, or HTML.

use anyhow::{Context, Result, bail};
use pulldown_cmark::{Event, Options as MarkdownOptions, Parser as MarkdownParser, Tag, TagEnd};
use reqwest::Client;
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT};
use std::time::Duration;
use tokio::time::timeout;
use url::Url;

use tidev_utils::encoding::{
    DecodeOptions, decode_command_output, decode_text_lossy_with_options, encoding_from_label,
};

use crate::types::WebFetchArgs;

const FETCH_DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_MAX_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESPONSE_BYTES: usize = 5 * 1024 * 1024;
const FETCH_DEFAULT_LINE_LIMIT: i64 = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WebFetchFormat {
    Text,
    Markdown,
    Html,
}

/// Execute a webfetch request.
pub async fn fetch(args: WebFetchArgs) -> Result<String> {
    let url = validate_url(&args.url)?;
    let format = match args.format.as_deref() {
        Some("text") => WebFetchFormat::Text,
        Some("html") => WebFetchFormat::Html,
        _ => WebFetchFormat::Markdown,
    };

    let timeout_secs = args
        .timeout
        .unwrap_or(FETCH_DEFAULT_TIMEOUT.as_secs() as i64)
        .min(FETCH_MAX_TIMEOUT.as_secs() as i64)
        .max(1);
    let duration = Duration::from_secs(timeout_secs as u64);

    crate::ensure_rustls_crypto_provider();
    let http = Client::builder()
        .user_agent("tidev-webtools/0.1")
        .build()
        .context("failed to construct fetch HTTP client")?;

    let headers = fetch_headers(format);
    let response = timeout(duration, fetch_response(&http, &url, headers.clone()))
        .await
        .context("fetch request timed out")??;

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mime = content_type
        .as_deref()
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

    let body = decode_response_body(&bytes, content_type.as_deref());
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
                let md = html2md::rewrite_html(&body, false);
                markdown_to_text(&md)
            } else {
                body
            }
        }
    };

    let output = output.trim().to_string();
    if output.is_empty() {
        bail!("fetched page is empty");
    }

    // Apply line-based offset/limit (matching read tool behavior)
    let offset = args.offset.unwrap_or(1);
    let limit = args.limit.unwrap_or(FETCH_DEFAULT_LINE_LIMIT);
    if offset < 1 {
        bail!("offset must be greater than or equal to 1");
    }
    if limit < 1 {
        bail!("limit must be greater than or equal to 1");
    }

    let lines: Vec<&str> = output.lines().collect();
    let total_lines = lines.len();

    if total_lines < offset as usize && !(total_lines == 0 && offset == 1) {
        bail!(
            "Offset {} is out of range for this page ({} lines)",
            offset,
            total_lines,
        );
    }

    let start = (offset as usize).saturating_sub(1);
    let selected: Vec<&str> = lines
        .iter()
        .skip(start)
        .take(limit as usize)
        .copied()
        .collect();
    let end = start + selected.len();

    let mut content = selected
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{}: {}", offset as usize + i, line))
        .collect::<Vec<_>>()
        .join("\n");

    let has_more = end < total_lines;
    let next_offset = offset + selected.len() as i64;
    if has_more {
        content.push_str(&format!(
            "\n\n(Showing lines {}-{} of {}. Use offset={} to continue.)",
            offset,
            offset + selected.len() as i64 - 1,
            total_lines,
            next_offset,
        ));
    } else {
        content.push_str(&format!("\n\n(End of page - total {} lines)", total_lines));
    }

    Ok(content)
}

fn decode_response_body(bytes: &[u8], content_type: Option<&str>) -> String {
    let explicit_encoding = content_type
        .and_then(response_charset)
        .and_then(encoding_from_label);
    match explicit_encoding {
        Some(encoding) => decode_text_lossy_with_options(
            bytes,
            DecodeOptions {
                fallback_encoding: Some(encoding),
                allow_heuristic: false,
            },
        )
        .into_text(),
        None => decode_command_output(bytes),
    }
}

fn response_charset(content_type: &str) -> Option<&str> {
    content_type.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        if name.trim().eq_ignore_ascii_case("charset") {
            Some(value.trim().trim_matches('"').trim_matches('\''))
        } else {
            None
        }
    })
}

async fn fetch_response(http: &Client, url: &Url, headers: HeaderMap) -> Result<reqwest::Response> {
    let response = http
        .get(url.as_str())
        .headers(headers)
        .send()
        .await
        .with_context(|| format!("failed to fetch {}", url))?;

    if !response.status().is_success() {
        bail!("fetch request failed with status {}", response.status());
    }

    Ok(response)
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

    let mut _in_code_block = false;
    for event in MarkdownParser::new_ext(markdown, options) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                _in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                if !output.ends_with('\n') {
                    output.push('\n');
                }
                _in_code_block = false;
            }
            Event::Text(text) | Event::Code(text) => {
                output.push_str(&text);
            }
            Event::SoftBreak | Event::HardBreak => {
                output.push('\n');
            }
            Event::Start(Tag::Paragraph) if !output.is_empty() && !output.ends_with('\n') => {
                output.push('\n');
            }
            Event::End(TagEnd::Paragraph) => {
                output.push('\n');
            }
            Event::End(TagEnd::Item) => {
                output.push('\n');
            }
            Event::End(TagEnd::List(_)) if !output.ends_with('\n') => {
                output.push('\n');
            }
            _ => {}
        }
    }

    output.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_charset_parameter() {
        assert_eq!(
            response_charset("text/html; charset=\"gb2312\""),
            Some("gb2312")
        );
    }

    #[test]
    fn decodes_gbk_response_body_from_charset() {
        assert_eq!(
            decode_response_body(&[0xC4, 0xE3, 0xBA, 0xC3], Some("text/plain; charset=gbk")),
            "你好"
        );
    }
}
