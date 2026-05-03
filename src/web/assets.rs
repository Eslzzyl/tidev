use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

/// Embedded web assets from web/dist
#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct EmbeddedAssets;

/// Serve an embedded file by path
pub fn serve_file(path: &str) -> Option<Response> {
    let asset = EmbeddedAssets::get(path)?;

    let mime = mime_guess::from_path(path).first_or_octet_stream();

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .body(Body::from(asset.data))
        .ok()?;

    Some(response)
}

/// Handle a request for an embedded asset
pub async fn handle_embedded_request(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    // Try to serve the exact path
    if let Some(response) = serve_file(path) {
        return response;
    }

    // If path is empty or directory-like, try index.html
    if (path.is_empty() || path.ends_with('/'))
        && let Some(response) = serve_file(&format!("{}index.html", path)) {
            return response;
        }

    // Try appending .html for SPA routes
    if !path.contains('.')
        && let Some(response) = serve_file(&format!("{}.html", path)) {
            return response;
        }

    // Fallback to index.html for SPA routing (client-side routing)
    if let Some(response) = serve_file("index.html") {
        return response;
    }

    // Last resort: 404
    (StatusCode::NOT_FOUND, "Not found").into_response()
}

/// Check if embedded assets are available
pub fn has_embedded_assets() -> bool {
    // Try to get index.html to verify assets are embedded
    EmbeddedAssets::get("index.html").is_some()
}

/// List available embedded assets (for debugging)
#[allow(dead_code)]
pub fn list_assets() -> Vec<String> {
    EmbeddedAssets::iter().map(|f| f.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_assets_available() {
        // This test verifies that the embedding worked
        if has_embedded_assets() {
            let assets = list_assets();
            assert!(!assets.is_empty(), "Embedded assets should not be empty");
            eprintln!("Embedded assets: {:?}", assets);
        }
    }
}
