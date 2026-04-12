use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Debug)]
pub struct ToolRegistry {
    workspace_root: PathBuf,
    max_output_bytes: usize,
    definitions: Vec<ToolDefinition>,
}

impl ToolRegistry {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            max_output_bytes: 12_000,
            definitions: vec![
                ToolDefinition {
                    name: "read_file",
                    description: "Read a text file inside the workspace",
                },
                ToolDefinition {
                    name: "write_file",
                    description: "Write a text file inside the workspace",
                },
                ToolDefinition {
                    name: "list_dir",
                    description: "List entries in a directory inside the workspace",
                },
                ToolDefinition {
                    name: "shell",
                    description: "Run a shell command in the workspace root",
                },
            ],
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    pub fn max_output_bytes(&self) -> usize {
        self.max_output_bytes
    }
}

pub fn read_file(workspace_root: &Path, relative_path: impl AsRef<Path>) -> Result<String> {
    let path = resolve_workspace_path(workspace_root, relative_path.as_ref())?;
    let mut contents =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;

    if contents.len() > 12_000 {
        contents.truncate(12_000);
        contents.push_str("\n[truncated]");
    }

    Ok(contents)
}

pub fn write_file(
    workspace_root: &Path,
    relative_path: impl AsRef<Path>,
    content: &str,
) -> Result<()> {
    let path = resolve_workspace_path(workspace_root, relative_path.as_ref())?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub fn list_dir(workspace_root: &Path, relative_path: impl AsRef<Path>) -> Result<String> {
    let path = resolve_workspace_path(workspace_root, relative_path.as_ref())?;

    let mut entries = Vec::new();
    for entry in fs::read_dir(&path).with_context(|| format!("failed to read {}", path.display()))? {
        let entry = entry.with_context(|| format!("failed to read entry in {}", path.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        let mut name = entry.file_name().to_string_lossy().to_string();
        if file_type.is_dir() {
            name.push('/');
        }
        entries.push(name);
    }

    entries.sort();

    if entries.is_empty() {
        Ok("(empty)".to_string())
    } else {
        Ok(entries.join("\n"))
    }
}

pub fn run_shell(workspace_root: &Path, command: &str, max_output_bytes: usize) -> Result<String> {
    let output = if cfg!(target_os = "windows") {
        Command::new("cmd")
            .arg("/C")
            .arg(command)
            .current_dir(workspace_root)
            .output()
            .with_context(|| format!("failed to run command '{command}'"))?
    } else {
        Command::new("sh")
            .arg("-lc")
            .arg(command)
            .current_dir(workspace_root)
            .output()
            .with_context(|| format!("failed to run command '{command}'"))?
    };

    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));

    if !output.stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
    }

    if combined.len() > max_output_bytes {
        combined.truncate(max_output_bytes);
        combined.push_str("\n[truncated]");
    }

    let status = output.status.code().unwrap_or_default();
    Ok(format!("[exit {status}]\n{combined}"))
}

fn resolve_workspace_path(workspace_root: &Path, candidate: &Path) -> Result<PathBuf> {
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    };

    if !absolute.starts_with(workspace_root) {
        bail!("path {} escapes the workspace root", candidate.display());
    }

    Ok(absolute)
}
