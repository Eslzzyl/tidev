use std::borrow::Cow;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::Response;
use axum_reverse_proxy::ReverseProxy;
use rust_embed::RustEmbed;
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

#[derive(RustEmbed)]
#[folder = "web-dist"]
#[allow_missing = true]
struct EmbeddedAssets;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrontendMode {
    Dev,
    Embedded,
    Fallback,
}

#[derive(Clone, Debug)]
pub struct FrontendConfig {
    pub root: PathBuf,
    pub port: u16,
}

impl Default for FrontendConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web"),
            port: 5173,
        }
    }
}

pub struct Frontend {
    mode: FrontendMode,
    dev_child: Option<Child>,
    dev_port: u16,
}

impl Frontend {
    pub async fn start(config: FrontendConfig, cancel: CancellationToken) -> Self {
        if cfg!(debug_assertions) {
            match start_vite(&config, cancel).await {
                Ok(child) => {
                    log::info!("frontend mode: Vite development server");
                    return Self {
                        mode: FrontendMode::Dev,
                        dev_child: Some(child),
                        dev_port: config.port,
                    };
                }
                Err(error) => {
                    log::warn!("Vite development server unavailable: {error}");
                    return Self {
                        mode: FrontendMode::Fallback,
                        dev_child: None,
                        dev_port: config.port,
                    };
                }
            }
        }

        if EmbeddedAssets::iter().next().is_some() {
            log::info!("frontend mode: embedded release assets");
            Self {
                mode: FrontendMode::Embedded,
                dev_child: None,
                dev_port: config.port,
            }
        } else {
            log::error!("embedded frontend assets are unavailable");
            Self {
                mode: FrontendMode::Fallback,
                dev_child: None,
                dev_port: config.port,
            }
        }
    }

    pub fn mode(&self) -> FrontendMode {
        self.mode
    }

    pub fn router(&self) -> Router<Arc<crate::api::AppState>> {
        match self.mode {
            FrontendMode::Dev => {
                let target = format!("http://127.0.0.1:{}", self.dev_port);
                ReverseProxy::new("/", target.as_str()).into()
            }
            FrontendMode::Embedded => Router::new().fallback(serve_embedded),
            FrontendMode::Fallback => Router::new().fallback(serve_fallback),
        }
    }

    pub async fn shutdown(mut self) {
        if let Some(mut child) = self.dev_child.take() {
            if let Err(error) = child.kill().await {
                log::debug!("failed to stop Vite process: {error}");
            }
            let _ = child.wait().await;
        }
    }
}

async fn start_vite(config: &FrontendConfig, cancel: CancellationToken) -> Result<Child> {
    if !config.root.join("package.json").is_file() {
        anyhow::bail!(
            "frontend directory does not contain package.json: {}",
            config.root.display()
        );
    }

    // Run the frozen install on every debug start so an interrupted or partial
    // node_modules directory cannot make Vite start with broken dependencies.
    run_pnpm_install(&config.root, &cancel).await?;

    let port = config.port.to_string();
    let mut child = Command::new("pnpm")
        .arg("--dir")
        .arg(&config.root)
        .args(["run", "dev", "--host", "127.0.0.1", "--port"])
        .arg(&port)
        .arg("--strictPort")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;

    if let Err(error) = wait_for_port(config.port, &cancel).await {
        if let Err(kill_error) = child.kill().await {
            log::debug!("failed to stop Vite after startup error: {kill_error}");
        }
        let _ = child.wait().await;
        return Err(error);
    }
    Ok(child)
}

async fn run_pnpm_install(root: &std::path::Path, cancel: &CancellationToken) -> Result<()> {
    let mut child = Command::new("pnpm")
        .arg("--dir")
        .arg(root)
        .args(["install", "--frozen-lockfile"])
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;

    tokio::select! {
        status = child.wait() => {
            let status = status?;
            if !status.success() {
                anyhow::bail!("pnpm install failed with status {status}");
            }
        }
        _ = cancel.cancelled() => {
            if let Err(error) = child.kill().await {
                log::debug!("failed to stop pnpm install: {error}");
            }
            let _ = child.wait().await;
            anyhow::bail!("frontend startup cancelled");
        }
    }
    Ok(())
}

async fn wait_for_port(port: u16, cancel: &CancellationToken) -> Result<()> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    timeout(Duration::from_secs(10), async {
        loop {
            if tokio::net::TcpStream::connect(address).await.is_ok() {
                return Ok(());
            }
            tokio::select! {
                _ = cancel.cancelled() => anyhow::bail!("frontend startup cancelled"),
                _ = sleep(Duration::from_millis(100)) => {},
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("Vite did not listen on {address} within 10 seconds"))??;
    Ok(())
}

async fn serve_embedded(uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let mut candidates = vec![requested.to_owned()];
    if requested.is_empty() || requested.ends_with('/') {
        candidates.insert(0, format!("{requested}index.html"));
    } else if !requested.contains('.') {
        candidates.push(format!("{requested}.html"));
    }
    candidates.push("index.html".to_owned());

    for candidate in candidates {
        if let Some(file) = EmbeddedAssets::get(candidate.trim_start_matches('/')) {
            return asset_response(&candidate, file.data);
        }
    }
    fallback_response("embedded frontend asset not found")
}

async fn serve_fallback() -> Response {
    fallback_response("the Vite development server or embedded assets are unavailable")
}

fn asset_response(path: &str, data: Cow<'static, [u8]>) -> Response {
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else if path == "index.html" {
        "public, no-cache"
    } else {
        "public, max-age=86400"
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            mime_guess::from_path(path).first_or_octet_stream().as_ref(),
        )
        .header(header::CACHE_CONTROL, HeaderValue::from_static(cache))
        .body(Body::from(data.into_owned()))
        .expect("asset response builder should be valid")
}

fn fallback_response(reason: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>tidev Web</title><style>body{{font-family:system-ui,sans-serif;max-width:52rem;margin:4rem auto;padding:0 1rem;color:#222}}code,pre{{background:#f1f3f5;padding:.2rem .4rem;border-radius:.25rem}}pre{{padding:1rem;overflow:auto}}</style></head><body><h1>tidev Web</h1><p>Backend is running, but the frontend is unavailable.</p><p>Reason: <code>{reason}</code></p><p>For development, install pnpm and run:</p><pre>pnpm --dir web install --frozen-lockfile\npnpm --dir web dev</pre><p>The <code>/api</code> endpoints remain available.</p></body></html>"
    );
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(html))
        .expect("fallback response builder should be valid")
}

#[cfg(test)]
mod tests {
    use super::EmbeddedAssets;

    #[test]
    fn release_build_embeds_the_frontend_entrypoint() {
        if !cfg!(debug_assertions) {
            assert!(EmbeddedAssets::get("index.html").is_some());
        }
    }
}
