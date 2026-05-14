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
        if let Err(msg) = build_web() {
            println!("cargo:warning={}", msg);
            // Ensure web/dist exists so rust-embed doesn't fail at compile time.
            // If pnpm wasn't available, create a placeholder directory.
            ensure_dist_placeholder();
        }
    }
}

/// Create a minimal web/dist placeholder so that rust-embed compilation succeeds
/// even when the full frontend build is skipped (e.g., pnpm not available).
fn ensure_dist_placeholder() {
    let dist = std::path::Path::new("web/dist");
    if dist.exists() {
        return;
    }
    std::fs::create_dir_all(dist.join("assets")).ok();
    let placeholder = r#"<!DOCTYPE html>
<html>
<head><title>TiDev</title></head>
<body>
  <h1>Frontend not available</h1>
  <p>The web frontend was not built. Run <code>cd web && pnpm install && pnpm build</code> to build it.</p>
</body>
</html>"#;
    std::fs::write(dist.join("index.html"), placeholder).ok();
}

/// Returns true if any source file is newer than the last build output,
/// meaning a rebuild is needed. Uses web/dist/index.html (the actual build
/// artifact) as the reference instead of the dist directory, which has
/// unreliable mtime semantics on some filesystems.
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
