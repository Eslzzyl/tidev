use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let is_release = env::var("PROFILE").as_deref() == Ok("release");
    let builds_web = is_release && env::var_os("TIDEV_WEB_SKIP_BUILD").is_none();

    // The Windows Vite process has reproducibly crashed with 0xc0000374 when its
    // production build runs alongside Cargo's Rust compilation. Reserving Cargo's remaining
    // jobserver tokens lets active Rust jobs finish before pnpm starts and prevents
    // new ones from starting until the frontend build exits.
    #[cfg(windows)]
    let _cargo_job_tokens = builds_web.then(reserve_cargo_job_tokens);

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let web_dir = manifest_dir.join("../../web");
    let embedded_dir = manifest_dir.join("web-dist");

    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=TIDEV_WEB_SKIP_BUILD");
    for path in [
        web_dir.join("package.json"),
        web_dir.join("pnpm-lock.yaml"),
        web_dir.join("vite.config.ts"),
        web_dir.join("tsconfig.json"),
        web_dir.join("tsconfig.app.json"),
        web_dir.join("tsconfig.node.json"),
        web_dir.join("index.html"),
        web_dir.join("src"),
        web_dir.join("public"),
    ] {
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    if !is_release {
        return;
    }

    let dist_dir = web_dir.join("dist");
    reset_directory(&embedded_dir).expect("failed to prepare release web asset directory");

    if builds_web {
        if let Err(error) = run_pnpm(&web_dir, &["install", "--frozen-lockfile"]) {
            println!("cargo:warning={error}");
        } else if let Err(error) = run_pnpm(&web_dir, &["run", "build"]) {
            println!("cargo:warning={error}");
        }
    }

    if !dist_dir.join("index.html").is_file() {
        println!(
            "cargo:warning=web frontend assets are unavailable; the release binary will serve the compatibility fallback page. Build them with `pnpm --dir {} install --frozen-lockfile && pnpm --dir {} run build`.",
            web_dir.display(),
            web_dir.display(),
        );
        return;
    }

    copy_directory(&dist_dir, &embedded_dir)
        .expect("failed to copy web assets into embedded web asset directory");
}

#[cfg(windows)]
/// Reserves every Cargo jobserver token except the one held by this build script.
///
/// Keeping the returned guards alive for the pnpm invocation serializes only the
/// Windows frontend build with the rest of Cargo's parallel work.
fn reserve_cargo_job_tokens() -> Vec<jobserver::Acquired> {
    let jobs = env::var("NUM_JOBS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    // SAFETY: this runs before the build script opens files or starts child processes.
    let Some(client) = (unsafe { jobserver::Client::from_env() }) else {
        return Vec::new();
    };

    (1..jobs)
        .map(|_| {
            client
                .acquire()
                .expect("failed to reserve a Cargo jobserver token")
        })
        .collect()
}

fn reset_directory(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    fs::create_dir_all(path)
}

fn run_pnpm(web_dir: &Path, args: &[&str]) -> Result<(), String> {
    #[cfg(windows)]
    let status = run_pnpm_command("pnpm.cmd", web_dir, args).or_else(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            run_pnpm_command("pnpm", web_dir, args)
        } else {
            Err(error)
        }
    });

    #[cfg(not(windows))]
    let status = run_pnpm_command("pnpm", web_dir, args);

    let status = status
        .map_err(|error| {
            format!(
                "failed to execute pnpm for the web frontend: {error}; the release binary will serve the compatibility fallback page"
            )
        })?;

    if !status.success() {
        return Err(format!(
            "pnpm {} failed with status {status}; the release binary will serve the compatibility fallback page",
            args.join(" ")
        ));
    }

    Ok(())
}

fn run_pnpm_command(
    command: &str,
    web_dir: &Path,
    args: &[&str],
) -> std::io::Result<std::process::ExitStatus> {
    Command::new(command)
        .arg("--dir")
        .arg(web_dir)
        .args(args)
        .status()
}

fn copy_directory(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}
