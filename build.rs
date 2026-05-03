use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

fn main() {
    // 告诉 cargo 在以下文件变化时重新运行构建脚本
    println!("cargo:rerun-if-changed=web/src");
    println!("cargo:rerun-if-changed=web/package.json");
    println!("cargo:rerun-if-changed=web/pnpm-lock.yaml");
    println!("cargo:rerun-if-changed=web/vite.config.ts");
    println!("cargo:rerun-if-changed=web/index.html");

    let dist_path = Path::new("web/dist");
    let src_path = Path::new("web/src");
    let package_json = Path::new("web/package.json");
    let lockfile = Path::new("web/pnpm-lock.yaml");

    // 检查是否需要重新构建
    let needs_build = should_rebuild(dist_path, src_path, package_json, lockfile);

    if needs_build {
        println!("cargo:warning=Building web frontend...");

        // 检查 pnpm 是否可用
        if Command::new("pnpm").arg("--version").output().is_err() {
            eprintln!("Warning: pnpm not found. Skipping frontend build.");
            eprintln!("Install pnpm from https://pnpm.io");
            eprintln!("Or manually build: cd web && pnpm install && pnpm build");
            return;
        }

        // 检查 node_modules 是否存在
        let node_modules = Path::new("web/node_modules");
        if !node_modules.exists() {
            println!("cargo:warning=Running pnpm install...");
            let status = Command::new("pnpm")
                .args(["--dir", "web", "install", "--frozen-lockfile"])
                .status();

            match status {
                Ok(s) if s.success() => {}
                Ok(s) => {
                    eprintln!(
                        "Warning: pnpm install failed with exit code: {:?}",
                        s.code()
                    );
                    return;
                }
                Err(e) => {
                    eprintln!("Warning: Failed to run pnpm install: {}", e);
                    return;
                }
            }
        }

        // 构建前端
        println!("cargo:warning=Running pnpm build...");
        let status = Command::new("pnpm")
            .args(["--dir", "web", "build"])
            .status();

        match status {
            Ok(s) if s.success() => {
                println!("cargo:warning=Web frontend built successfully");
            }
            Ok(s) => {
                eprintln!("Warning: pnpm build failed with exit code: {:?}", s.code());
            }
            Err(e) => {
                eprintln!("Warning: Failed to run pnpm build: {}", e);
            }
        }
    }
}

fn should_rebuild(dist_path: &Path, src_path: &Path, package_json: &Path, lockfile: &Path) -> bool {
    // 如果 dist 不存在，需要构建
    if !dist_path.exists() {
        return true;
    }

    let dist_mtime = get_mtime(dist_path);

    // 检查源文件是否比 dist 新
    if let Some(dist_time) = dist_mtime {
        // 检查 src 目录
        if is_path_newer_than(src_path, dist_time) {
            return true;
        }
        // 检查 package.json
        if let Some(t) = get_mtime(package_json)
            && t > dist_time
        {
            return true;
        }
        // 检查 lockfile
        if let Some(t) = get_mtime(lockfile)
            && t > dist_time
        {
            return true;
        }
    }

    false
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

    // 如果是目录，递归检查子项
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
