use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repository_root = manifest_dir.join("../..");
    let git_dir = repository_root.join(".git");

    for path in [
        git_dir.join("HEAD"),
        git_dir.join("index"),
        git_dir.join("packed-refs"),
        git_dir.join("refs/heads"),
        git_dir.join("refs/tags"),
    ] {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let commit = git_output(&repository_root, &["rev-parse", "HEAD"])
        .unwrap_or_else(|| "unknown".to_owned());
    let dirty = commit != "unknown" && git_worktree_is_dirty(&repository_root);
    let display_version = format_display_version(
        env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION must be set"),
        &commit,
        dirty,
    );

    let output_path = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("build_info.rs");
    let contents = format!(
        "pub const PACKAGE_VERSION: &str = {package_version:?};\n\
         pub const COMMIT: &str = {commit:?};\n\
         pub const DISPLAY_VERSION: &str = {display_version:?};\n\
         pub const DIRTY: bool = {dirty};\n",
        package_version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION must be set"),
    );
    fs::write(output_path, contents).expect("failed to write generated build metadata");
}

fn git_output(repository_root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(arguments)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!value.is_empty() && !value.contains(['\r', '\n'])).then_some(value)
}

fn git_worktree_is_dirty(repository_root: &Path) -> bool {
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
    else {
        return false;
    };

    output.status.success() && !output.stdout.is_empty()
}

fn format_display_version(package_version: String, commit: &str, dirty: bool) -> String {
    let suffix = if commit == "unknown" {
        "dev".to_owned()
    } else {
        let short_commit: String = commit.chars().take(7).collect();
        format!("g{short_commit}{}", if dirty { ".dirty" } else { "" })
    };

    format!("{package_version}+{suffix}")
}
