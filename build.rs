use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

fn main() {
    // Instruct Cargo to re-run the build script only when these paths change
    println!("cargo:rerun-if-changed=web/src");
    println!("cargo:rerun-if-changed=web/package.json");
    println!("cargo:rerun-if-changed=web/pnpm-lock.yaml");
    println!("cargo:rerun-if-changed=web/vite.config.ts");
    println!("cargo:rerun-if-changed=web/index.html");

    if needs_build() {
        // Attempt to build web frontend (pnpm must be available)
        if let Err(msg) = build_web() {
            println!("cargo:warning={}", msg);
        }
    }

    // ── Populate OUT_DIR with assets (real or placeholder) ──────────

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR must be set");
    let out_dist = Path::new(&out_dir).join("web-dist");

    let source_dist = Path::new("web/dist");
    if source_dist.join("index.html").exists() {
        // Real frontend build exists — copy to OUT_DIR
        if let Err(e) = copy_dir_all(source_dist, &out_dist) {
            println!("cargo:warning=Failed to copy web/dist to OUT_DIR: {}", e);
            create_placeholder(&out_dist);
        }
    } else {
        // No real assets — create placeholder in OUT_DIR only
        create_placeholder(&out_dist);
    }

    // Generate the Rust module that embeds everything via include_bytes!
    generate_asset_module(&out_dist, &out_dir);
}

// ── Build helpers ─────────────────────────────────────────────────

/// Returns true if any web source file is newer than the last build output.
fn needs_build() -> bool {
    let dist_index = Path::new("web/dist/index.html");
    if !dist_index.exists() {
        return true;
    }

    let Some(baseline) = get_mtime(dist_index) else {
        return true;
    };

    if is_path_newer_than(Path::new("web/src"), baseline) {
        return true;
    }
    for path in &[
        "web/package.json",
        "web/pnpm-lock.yaml",
        "web/vite.config.ts",
        "web/index.html",
    ] {
        if let Some(t) = get_mtime(Path::new(path))
            && t > baseline
        {
            return true;
        }
    }

    false
}

fn build_web() -> Result<(), String> {
    if Command::new("pnpm").arg("--version").output().is_err() {
        return Err("pnpm not found, skipping frontend build.\n  \
                     Install from https://pnpm.io\n  \
                     Or manually run: cd web && pnpm install && pnpm build"
            .into());
    }

    if !Path::new("web/node_modules").exists() {
        let status = Command::new("pnpm")
            .args(["--dir", "web", "install", "--frozen-lockfile"])
            .status()
            .map_err(|e| format!("Failed to run pnpm install: {}", e))?;

        if !status.success() {
            return Err(format!(
                "pnpm install failed with exit code: {:?}",
                status.code()
            ));
        }
    }

    let output = Command::new("pnpm")
        .args(["--dir", "web", "build"])
        .output()
        .map_err(|e| format!("Failed to run pnpm build: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lines: Vec<&str> = stderr.lines().filter(|l| !l.is_empty()).collect();
        let tail: Vec<&&str> = lines.iter().rev().take(10).collect();
        let mut msg = format!("pnpm build failed (exit code: {:?})", output.status.code());
        for line in tail.iter().rev() {
            msg.push_str("\n  | ");
            msg.push_str(line);
        }
        return Err(msg);
    }

    Ok(())
}

fn get_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

fn is_path_newer_than(path: &Path, threshold: SystemTime) -> bool {
    if let Some(mtime) = get_mtime(path)
        && mtime > threshold
    {
        return true;
    }

    if path.is_dir()
        && let Ok(entries) = std::fs::read_dir(path)
    {
        for entry in entries.flatten() {
            if is_path_newer_than(&entry.path(), threshold) {
                return true;
            }
        }
    }

    false
}

// ── OUT_DIR helpers ───────────────────────────────────────────────

/// Copy a directory tree from src to dst (std-only, no extra deps).
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Create a minimal placeholder index.html in OUT_DIR so the binary
/// compiles and shows a helpful message when the frontend isn't built.
fn create_placeholder(out_dist: &Path) {
    std::fs::create_dir_all(out_dist.join("assets")).ok();
    let html = r#"<!DOCTYPE html>
<html>
<head><title>TiDev</title></head>
<body>
  <h1>Frontend not available</h1>
  <p>The web frontend was not built. Run <code>cd web && pnpm install && pnpm build</code> to build it.</p>
</body>
</html>"#;
    std::fs::write(out_dist.join("index.html"), html).ok();
}

/// Recursively collect all file paths relative to `base`.
fn collect_asset_paths(base: &Path, dir: &Path, paths: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_asset_paths(base, &path, paths);
            } else if path.is_file()
                && let Ok(rel) = path.strip_prefix(base)
            {
                paths.push(rel.to_str().unwrap_or("").replace('\\', "/"));
            }
        }
    }
}

/// Escape special characters for use in a Rust string literal.
fn escape_rust_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Generate `web_assets_generated.rs` in `OUT_DIR`.
///
/// The generated module provides:
/// - `get_asset_internal(path) -> Option<&'static [u8]>`
/// - `ASSET_PATHS: &[&str]`
/// - `has_assets_internal() -> bool`
fn generate_asset_module(out_dist: &Path, out_dir: &str) {
    let mut paths: Vec<String> = Vec::new();
    collect_asset_paths(out_dist, out_dist, &mut paths);
    paths.sort();

    let gen_path = Path::new(out_dir).join("web_assets_generated.rs");
    let mut code = String::new();

    code.push_str("// Auto-generated by build.rs — DO NOT EDIT\n\n");

    // ── get_asset_internal ──
    code.push_str("/// Look up an embedded asset by its web-root-relative path.\n");
    code.push_str("pub fn get_asset_internal(path: &str) -> Option<&'static [u8]> {\n");
    code.push_str("    match path {\n");
    for p in &paths {
        let escaped = escape_rust_string(p);
        code.push_str(&format!(
            "        \"{}\" => Some(include_bytes!(\"web-dist/{}\")),\n",
            escaped, escaped
        ));
    }
    code.push_str("        _ => None,\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    // ── ASSET_PATHS ──
    code.push_str("/// All embedded asset paths (sorted).\n");
    code.push_str("pub static ASSET_PATHS: &[&str] = &[\n");
    for p in &paths {
        let escaped = escape_rust_string(p);
        code.push_str(&format!("    \"{}\",\n", escaped));
    }
    code.push_str("];\n\n");

    // ── has_assets_internal ──
    code.push_str("/// Returns true if any assets are embedded.\n");
    code.push_str("pub fn has_assets_internal() -> bool {\n");
    code.push_str("    !ASSET_PATHS.is_empty()\n");
    code.push_str("}\n");

    std::fs::write(&gen_path, code).expect("failed to write generated asset module");
}
