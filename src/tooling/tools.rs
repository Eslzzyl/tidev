use anyhow::{Context, Result, bail};
use grep::{
    regex::RegexMatcherBuilder,
    searcher::{SearcherBuilder, sinks},
};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use diffy::DiffOptions;

use crate::{
    session::{Message, MessageRole, ToolCall, ToolExecutionResult},
    skills::SkillCatalog,
    storage::SessionStore,
};
use uuid::Uuid;

use super::canonical_tool_name;
use super::schema::ToolArgs;
use super::{ToolDefinition, ToolPermission};

macro_rules! tool_field_type {
    (string($desc:literal)) => {
        String
    };
    (optional_string($desc:literal)) => {
        Option<String>
    };
    (boolean($desc:literal)) => {
        bool
    };
    (optional_boolean($desc:literal)) => {
        Option<bool>
    };
    (integer($desc:literal)) => {
        i64
    };
    (optional_integer($desc:literal)) => {
        Option<i64>
    };
    (array($item_ty:ty, $desc:literal)) => {
        Vec<$item_ty>
    };
    (optional_array($item_ty:ty, $desc:literal)) => {
        Option<Vec<$item_ty>>
    };
    (object($item_ty:ty, $desc:literal)) => {
        $item_ty
    };
    (optional_object($item_ty:ty, $desc:literal)) => {
        Option<$item_ty>
    };
}

macro_rules! tool_field_schema {
    (string($desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("string".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), true)
    }};
    (optional_string($desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("string".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), false)
    }};
    (boolean($desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("boolean".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), true)
    }};
    (optional_boolean($desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("boolean".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), false)
    }};
    (integer($desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("integer".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), true)
    }};
    (optional_integer($desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("integer".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), false)
    }};
    (array($item_ty:ty, $desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("array".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        schema.insert("items".to_string(), <$item_ty as ToolArgs>::schema());
        (Value::Object(schema), true)
    }};
    (optional_array($item_ty:ty, $desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("array".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        schema.insert("items".to_string(), <$item_ty as ToolArgs>::schema());
        (Value::Object(schema), false)
    }};
    (object($item_ty:ty, $desc:literal)) => {{
        let mut schema = <$item_ty as ToolArgs>::schema();
        if let Value::Object(ref mut object_schema) = schema {
            object_schema.insert("description".to_string(), Value::String($desc.to_string()));
        }
        (schema, true)
    }};
    (optional_object($item_ty:ty, $desc:literal)) => {{
        let mut schema = <$item_ty as ToolArgs>::schema();
        if let Value::Object(ref mut object_schema) = schema {
            object_schema.insert("description".to_string(), Value::String($desc.to_string()));
        }
        (schema, false)
    }};
}

macro_rules! tool_args {
    (
        $(#[$struct_meta:meta])*
        $vis:vis struct $name:ident {
            $(
                $field:ident : $kind:ident ( $($kind_args:tt)+ )
            ),* $(,)?
        }
    ) => {
        $(#[$struct_meta])*
        #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
        #[serde(deny_unknown_fields)]
        $vis struct $name {
            $(
                pub $field: tool_field_type!($kind($($kind_args)+)),
            )*
        }

        impl ToolArgs for $name {
            fn schema() -> Value {
                let mut properties = serde_json::Map::new();
                let mut required = Vec::new();

                $(
                    let (field_schema, is_required) = tool_field_schema!($kind($($kind_args)+));
                    properties.insert(stringify!($field).to_string(), field_schema);
                    if is_required {
                        required.push(Value::String(stringify!($field).to_string()));
                    }
                )*

                let mut schema = serde_json::Map::new();
                schema.insert("type".to_string(), Value::String("object".to_string()));
                schema.insert("properties".to_string(), Value::Object(properties));
                schema.insert("additionalProperties".to_string(), Value::Bool(false));
                if !required.is_empty() {
                    schema.insert("required".to_string(), Value::Array(required));
                }

                Value::Object(schema)
            }
        }
    };
}

tool_args! {
    pub struct ReadArgs {
        path: string("Path to read relative to the workspace root"),
    }
}

tool_args! {
    pub struct WriteArgs {
        path: string("Path to write relative to the workspace root"),
        content: string("File contents to write"),
    }
}

tool_args! {
    pub struct EditArgs {
        path: string("Path to edit relative to the workspace root"),
        old_text: string("Text to replace; must match exactly"),
        new_text: string("Replacement text"),
        replace_all: optional_boolean("Replace all matches instead of only the first"),
    }
}

tool_args! {
    pub struct ListArgs {
        path: optional_string("Directory path relative to the workspace root"),
    }
}

tool_args! {
    pub struct GlobArgs {
        pattern: string("Glob pattern to match against workspace-relative paths"),
        path: optional_string("Directory path to search relative to the workspace root"),
    }
}

tool_args! {
    pub struct GrepArgs {
        pattern: string("Regular expression to search for in file contents"),
        path: optional_string("Directory path to search relative to the workspace root"),
        include: optional_string("File glob to include in the search"),
    }
}

tool_args! {
    pub struct BashArgs {
        command: string("Shell command to execute from the workspace root"),
    }
}

tool_args! {
    pub struct TaskArgs {
        description: string("Short title for the task"),
        prompt: string("Task prompt to give the subagent"),
        subagent_type: optional_string("Optional subagent type, such as general or review"),
    }
}

tool_args! {
    pub struct TodoItem {
        content: string("Brief description of the task"),
        status: string("Current status of the task: pending, in_progress, completed, cancelled"),
        priority: string("Priority level of the task: high, medium, low"),
    }
}

tool_args! {
    pub struct TodoWriteArgs {
        todos: array(TodoItem, "The updated todo list"),
    }
}

tool_args! {
    pub struct SkillArgs {
        name: string("Skill name to load"),
    }
}

tool_args! {
    pub struct QuestionOption {
        label: string("Display text for the option"),
        description: optional_string("Optional explanation of the option"),
    }
}

tool_args! {
    pub struct QuestionInfo {
        question: string("Complete question"),
        header: string("Short label for the question"),
        options: array(QuestionOption, "Available choices"),
        multiple: optional_boolean("Allow selecting multiple choices"),
        custom: optional_boolean("Allow typing a custom answer"),
    }
}

tool_args! {
    pub struct QuestionArgs {
        questions: array(QuestionInfo, "Questions to ask"),
    }
}

tool_args! {
    pub struct WebSearchArgs {
        query: string("Web search query"),
        num_results: optional_integer("Number of search results to return"),
        search_type: optional_string("Search type: auto, fast, or deep"),
    }
}

tool_args! {
    pub struct WebFetchArgs {
        url: string("The URL to fetch"),
        format: optional_string("Output format: text, markdown, or html"),
        timeout: optional_integer("Timeout in seconds (max 120)"),
    }
}

fn parse_arguments<Args>(tool_name: &str, arguments: Value) -> Result<Args>
where
    Args: ToolArgs,
{
    serde_json::from_value(arguments)
        .with_context(|| format!("failed to decode arguments for tool '{}'", tool_name))
}

pub(super) fn tool_definitions(skill_description: String) -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::new::<ReadArgs>(
            "read",
            "Read a text file inside the workspace",
            ToolPermission::Read,
        ),
        ToolDefinition::new::<WriteArgs>(
            "write",
            "Write a text file inside the workspace",
            ToolPermission::Write,
        ),
        ToolDefinition::new::<EditArgs>(
            "edit",
            "Edit a file by replacing text inside it",
            ToolPermission::Edit,
        ),
        ToolDefinition::new::<ListArgs>(
            "list",
            "List entries in a directory inside the workspace",
            ToolPermission::Read,
        ),
        ToolDefinition::new::<GlobArgs>(
            "glob",
            "Find files matching a glob pattern inside the workspace",
            ToolPermission::Search,
        ),
        ToolDefinition::new::<GrepArgs>(
            "grep",
            "Search workspace files with a regular expression",
            ToolPermission::Search,
        ),
        ToolDefinition::new::<BashArgs>(
            "bash",
            "Run a shell command in the workspace root",
            ToolPermission::Execute,
        ),
        ToolDefinition::new::<TaskArgs>("task", "Run a subagent task", ToolPermission::Session),
        ToolDefinition::new::<QuestionArgs>(
            "question",
            "Ask the user questions during execution",
            ToolPermission::Session,
        ),
        ToolDefinition::new::<TodoWriteArgs>(
            "todowrite",
            "Update the session todo list",
            ToolPermission::Session,
        ),
        ToolDefinition::new::<SkillArgs>("skill", skill_description, ToolPermission::Session),
        ToolDefinition::new::<WebSearchArgs>(
            "websearch",
            "Search the web using Exa",
            ToolPermission::Search,
        ),
        ToolDefinition::new::<WebFetchArgs>(
            "webfetch",
            "Fetch a web page as text, markdown, or HTML",
            ToolPermission::Read,
        ),
    ]
}

pub fn read_file(workspace_root: &Path, relative_path: impl AsRef<Path>) -> Result<String> {
    let path = resolve_workspace_path(workspace_root, relative_path.as_ref())?;
    let mut contents =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;

    truncate_in_place(&mut contents, 12_000);
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

    if !path.is_dir() {
        bail!("{} is not a directory", path.display());
    }

    let mut entries = Vec::new();
    for entry in
        fs::read_dir(&path).with_context(|| format!("failed to read {}", path.display()))?
    {
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
        let label = display_workspace_relative(workspace_root, &path);
        Ok(format!("{label}/\n{}", entries.join("\n")))
    }
}

#[allow(dead_code)]
pub(super) fn edit_file(
    workspace_root: &Path,
    relative_path: impl AsRef<Path>,
    old_text: &str,
    new_text: &str,
    replace_all: bool,
) -> Result<String> {
    let path = resolve_workspace_path(workspace_root, relative_path.as_ref())?;
    let contents =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;

    let updated = apply_edit_contents(&contents, old_text, new_text, replace_all)?;

    fs::write(&path, updated).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(format!(
        "Edited {}",
        display_workspace_relative(workspace_root, &path)
    ))
}

fn apply_edit_contents(
    contents: &str,
    old_text: &str,
    new_text: &str,
    replace_all: bool,
) -> Result<String> {
    if old_text.is_empty() {
        bail!("old_text cannot be empty");
    }

    let matches = contents.match_indices(old_text).count();
    if matches == 0 {
        bail!("text not found in file");
    }
    if !replace_all && matches > 1 {
        bail!("text occurs multiple times; set replace_all to true");
    }

    Ok(if replace_all {
        contents.replace(old_text, new_text)
    } else {
        contents.replacen(old_text, new_text, 1)
    })
}

fn read_existing_text(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn file_change_output(
    workspace_root: &Path,
    absolute_path: &Path,
    old_content: &str,
    new_content: &str,
    action: &str,
) -> String {
    let relative = display_workspace_relative(workspace_root, absolute_path);
    let mut options = DiffOptions::new();
    options.set_original_filename(format!("a/{relative}"));
    options.set_modified_filename(format!("b/{relative}"));
    let patch = options.create_patch(old_content, new_content);

    if patch.hunks().is_empty() {
        format!("{action} {relative} (no content changes)")
    } else {
        patch.to_string()
    }
}

pub(super) fn glob_paths(
    workspace_root: &Path,
    relative_path: impl AsRef<Path>,
    pattern: &str,
) -> Result<String> {
    let search_root = resolve_workspace_path(workspace_root, relative_path.as_ref())?;
    if !search_root.exists() {
        bail!("{} does not exist", search_root.display());
    }

    let matcher = globset::GlobBuilder::new(pattern)
        .literal_separator(false)
        .build()
        .with_context(|| format!("invalid glob pattern '{pattern}'"))?
        .compile_matcher();

    let mut matches = Vec::new();
    let mut skipped = 0usize;

    if search_root.is_file() {
        let candidate = search_root.as_path();
        if glob_matches_path(candidate, &search_root, &matcher, pattern) {
            matches.push(SearchHit::from_path(&search_root)?);
        }
    } else {
        for result in WalkBuilder::new(&search_root)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .follow_links(false)
            .build()
        {
            let entry = match result {
                Ok(entry) => entry,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };

            if !entry
                .file_type()
                .map(|file_type| file_type.is_file())
                .unwrap_or(false)
            {
                continue;
            }

            let path = entry.into_path();
            if !glob_matches_path(&path, &search_root, &matcher, pattern) {
                continue;
            }

            matches.push(SearchHit::from_path(&path)?);
        }
    }

    matches.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| left.path.cmp(&right.path))
    });

    if matches.is_empty() {
        let mut output = String::from("No files found");
        if skipped > 0 {
            output.push_str("\n\n(Some paths were inaccessible and skipped)");
        }
        return Ok(output);
    }

    let limit = 100usize;
    let truncated = matches.len() > limit;
    let display_matches = if truncated {
        &matches[..limit]
    } else {
        &matches
    };

    let mut output = vec![format!(
        "Found {} files{}",
        matches.len(),
        if truncated {
            format!(" (showing first {limit})")
        } else {
            String::new()
        }
    )];

    for hit in display_matches {
        output.push(display_workspace_relative(workspace_root, &hit.path));
    }

    if truncated {
        output.push(String::new());
        output.push(format!(
            "(Results truncated: showing {limit} of {} matches.)",
            matches.len()
        ));
    }
    if skipped > 0 {
        output.push(String::new());
        output.push("(Some paths were inaccessible and skipped)".to_string());
    }

    Ok(output.join("\n"))
}

pub(super) fn grep_paths(
    workspace_root: &Path,
    relative_path: impl AsRef<Path>,
    pattern: &str,
    include: Option<&str>,
) -> Result<String> {
    if pattern.trim().is_empty() {
        bail!("pattern cannot be empty");
    }

    let search_root = resolve_workspace_path(workspace_root, relative_path.as_ref())?;
    if !search_root.exists() {
        bail!("{} does not exist", search_root.display());
    }

    let matcher = RegexMatcherBuilder::new()
        .build(pattern)
        .with_context(|| format!("invalid regular expression '{pattern}'"))?;
    let include_matcher = match include {
        Some(include) => Some(
            globset::GlobBuilder::new(include)
                .literal_separator(false)
                .build()
                .with_context(|| format!("invalid include glob '{include}'"))?
                .compile_matcher(),
        ),
        None => None,
    };
    let include_has_separator = include
        .map(|value| value.contains('/') || value.contains('\\'))
        .unwrap_or(false);

    let mut searcher = SearcherBuilder::new().line_number(true).build();
    let mut matches = Vec::new();
    let mut skipped = 0usize;

    let files: Box<dyn Iterator<Item = PathBuf>> = if search_root.is_file() {
        Box::new(std::iter::once(search_root.clone()))
    } else {
        Box::new(
            WalkBuilder::new(&search_root)
                .hidden(false)
                .git_ignore(true)
                .git_global(true)
                .git_exclude(true)
                .follow_links(false)
                .build()
                .filter_map(|result| match result {
                    Ok(entry) => entry
                        .file_type()
                        .map(|file_type| (entry, file_type.is_file())),
                    Err(_) => None,
                })
                .filter_map(|(entry, is_file)| is_file.then(|| entry.into_path())),
        )
    };

    for path in files {
        if let Some(include_matcher) = &include_matcher {
            let relative_candidate = path.strip_prefix(&search_root).unwrap_or(path.as_path());
            if !include_matcher.is_match(relative_candidate)
                && (!include_has_separator
                    && !path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| include_matcher.is_match(name))
                        .unwrap_or(false))
            {
                continue;
            }
        }

        let modified_at = path
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        let path_for_sink = path.clone();
        let mut file_hits = Vec::new();
        let sink = sinks::Lossy(|line_number, line| {
            file_hits.push(SearchHit {
                path: path_for_sink.clone(),
                line_number,
                line_text: line.to_string(),
                modified_at,
            });
            Ok(true)
        });

        if searcher.search_path(matcher.clone(), &path, sink).is_err() {
            skipped += 1;
            continue;
        }

        matches.extend(file_hits);
    }

    matches.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.line_number.cmp(&right.line_number))
    });

    if matches.is_empty() {
        let mut output = String::from("No files found");
        if skipped > 0 {
            output.push_str("\n\n(Some paths were inaccessible and skipped)");
        }
        return Ok(output);
    }

    let limit = 100usize;
    let truncated = matches.len() > limit;
    let display_matches = if truncated {
        &matches[..limit]
    } else {
        &matches
    };

    let mut output = vec![format!(
        "Found {} matches{}",
        matches.len(),
        if truncated {
            format!(" (showing first {limit})")
        } else {
            String::new()
        }
    )];

    let mut current_file: Option<PathBuf> = None;
    for hit in display_matches {
        let display_path = display_workspace_relative(workspace_root, &hit.path);
        if current_file.as_ref() != Some(&hit.path) {
            if current_file.is_some() {
                output.push(String::new());
            }
            current_file = Some(hit.path.clone());
            output.push(format!("{display_path}:"));
        }

        let mut line_text = hit.line_text.clone();
        if line_text.len() > 2_000 {
            truncate_in_place(&mut line_text, 2_000);
        }
        output.push(format!("  Line {}: {}", hit.line_number, line_text));
    }

    if truncated {
        output.push(String::new());
        output.push(format!(
            "(Results truncated: showing {limit} of {} matches.)",
            matches.len()
        ));
    }
    if skipped > 0 {
        output.push(String::new());
        output.push("(Some paths were inaccessible and skipped)".to_string());
    }

    Ok(output.join("\n"))
}

pub fn run_shell(workspace_root: &Path, command: &str, max_output_bytes: usize) -> Result<String> {
    run_shell_inner(workspace_root, command, max_output_bytes, None)
}

pub fn run_shell_with_cancel(
    workspace_root: &Path,
    command: &str,
    max_output_bytes: usize,
    cancelled: Arc<AtomicBool>,
) -> Result<String> {
    run_shell_inner(workspace_root, command, max_output_bytes, Some(cancelled))
}

pub fn execute_shell_tool_call(
    workspace_root: &Path,
    call: &ToolCall,
    max_output_bytes: usize,
    cancelled: Arc<AtomicBool>,
) -> Result<String> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;
    let args = parse_arguments::<BashArgs>(&call.name, arguments)?;
    run_shell_with_cancel(workspace_root, &args.command, max_output_bytes, cancelled)
}

fn run_shell_inner(
    workspace_root: &Path,
    command: &str,
    max_output_bytes: usize,
    cancelled: Option<Arc<AtomicBool>>,
) -> Result<String> {
    let mut process = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .arg("/C")
            .arg(command)
            .current_dir(workspace_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to run command '{command}'"))?
    } else {
        std::process::Command::new("sh")
            .arg("-lc")
            .arg(command)
            .current_dir(workspace_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to run command '{command}'"))?
    };

    let mut stdout = process.stdout.take();
    let mut stderr = process.stderr.take();

    loop {
        if cancelled
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            let _ = process.kill();
            let _ = process.wait();
            return Err(anyhow::anyhow!("shell command cancelled"));
        }

        if let Some(status) = process
            .try_wait()
            .with_context(|| format!("failed while waiting for command '{command}' to finish"))?
        {
            let mut combined = String::new();

            if let Some(mut handle) = stdout.take() {
                let _ = handle.read_to_string(&mut combined);
            }

            if let Some(mut handle) = stderr.take() {
                let mut error_output = String::new();
                let _ = handle.read_to_string(&mut error_output);
                if !error_output.is_empty() {
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str(&error_output);
                }
            }

            truncate_in_place(&mut combined, max_output_bytes);

            let status = status.code().unwrap_or_default();
            return Ok(format!("[exit {status}]\n{combined}"));
        }

        thread::sleep(std::time::Duration::from_millis(50));
    }
}

pub(super) fn validate_todos(todos: &[TodoItem]) -> Result<()> {
    for (index, todo) in todos.iter().enumerate() {
        if todo.content.trim().is_empty() {
            bail!("todo item {} has empty content", index + 1);
        }

        if !matches!(
            todo.status.as_str(),
            "pending" | "in_progress" | "completed" | "cancelled"
        ) {
            bail!(
                "todo item {} has invalid status '{}'",
                index + 1,
                todo.status
            );
        }

        if !matches!(todo.priority.as_str(), "high" | "medium" | "low") {
            bail!(
                "todo item {} has invalid priority '{}'",
                index + 1,
                todo.priority
            );
        }
    }

    Ok(())
}

pub(super) fn resolve_workspace_path(workspace_root: &Path, candidate: &Path) -> Result<PathBuf> {
    let mut resolved = if candidate.is_absolute() {
        PathBuf::new()
    } else {
        workspace_root.to_path_buf()
    };

    for component in candidate.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(part) => resolved.push(part),
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::Prefix(prefix) => resolved.push(prefix.as_os_str()),
        }
    }

    if !resolved.starts_with(workspace_root) {
        bail!("path {} escapes the workspace root", candidate.display());
    }

    Ok(resolved)
}

pub(super) fn display_workspace_relative(workspace_root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(workspace_root).unwrap_or(path);
    if relative.as_os_str().is_empty() {
        ".".to_string()
    } else {
        relative.display().to_string()
    }
}

fn truncate_to_limit(mut value: String, max_bytes: usize) -> String {
    truncate_in_place(&mut value, max_bytes);
    value
}

pub(super) fn truncate_in_place(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }

    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }

    value.truncate(end);
    value.push_str("\n[truncated]");
}

fn glob_matches_path(
    path: &Path,
    search_root: &Path,
    matcher: &globset::GlobMatcher,
    pattern: &str,
) -> bool {
    let relative_candidate = path.strip_prefix(search_root).unwrap_or(path);
    if matcher.is_match(relative_candidate) {
        return true;
    }

    if pattern.contains('/') || pattern.contains('\\') {
        return false;
    }

    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| matcher.is_match(name))
        .unwrap_or(false)
}

#[derive(Clone, Debug)]
struct SearchHit {
    path: PathBuf,
    line_number: u64,
    line_text: String,
    modified_at: SystemTime,
}

impl SearchHit {
    fn from_path(path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            line_number: 0,
            line_text: String::new(),
            modified_at: path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH),
        })
    }
}

pub(super) fn execute_tool_call(
    workspace_root: &Path,
    skills: &SkillCatalog,
    store: &SessionStore,
    session_id: uuid::Uuid,
    call: &ToolCall,
    max_output_bytes: usize,
) -> Result<ToolExecutionResult> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;

    let output = match canonical_tool_name(&call.name) {
        Some("read") => {
            let args = parse_arguments::<ReadArgs>(&call.name, arguments)?;
            read_file(workspace_root, args.path)
        }
        Some("write") => {
            let args = parse_arguments::<WriteArgs>(&call.name, arguments)?;
            let path = args.path;
            let absolute_path = resolve_workspace_path(workspace_root, Path::new(&path))?;
            let old_content = read_existing_text(&absolute_path)?;
            write_file(workspace_root, &path, &args.content)?;
            Ok(file_change_output(
                workspace_root,
                &absolute_path,
                &old_content,
                &args.content,
                "Wrote",
            ))
        }
        Some("edit") => {
            let args = parse_arguments::<EditArgs>(&call.name, arguments)?;
            let replace_all = args.replace_all.unwrap_or(false);
            let path = args.path;
            let absolute_path = resolve_workspace_path(workspace_root, Path::new(&path))?;
            let old_content = read_existing_text(&absolute_path)?;
            let updated =
                apply_edit_contents(&old_content, &args.old_text, &args.new_text, replace_all)?;

            fs::write(&absolute_path, &updated)
                .with_context(|| format!("failed to write {}", absolute_path.display()))?;

            Ok(file_change_output(
                workspace_root,
                &absolute_path,
                &old_content,
                &updated,
                "Edited",
            ))
        }
        Some("list") => {
            let args = parse_arguments::<ListArgs>(&call.name, arguments)?;
            let path = args.path.unwrap_or_else(|| ".".to_string());
            list_dir(workspace_root, path)
        }
        Some("glob") => {
            let args = parse_arguments::<GlobArgs>(&call.name, arguments)?;
            let path = args.path.unwrap_or_else(|| ".".to_string());
            glob_paths(workspace_root, path, &args.pattern)
        }
        Some("grep") => {
            let args = parse_arguments::<GrepArgs>(&call.name, arguments)?;
            let path = args.path.unwrap_or_else(|| ".".to_string());
            grep_paths(workspace_root, path, &args.pattern, args.include.as_deref())
        }
        Some("bash") => {
            let args = parse_arguments::<BashArgs>(&call.name, arguments)?;
            run_shell(workspace_root, &args.command, max_output_bytes)
        }
        Some("task") => {
            let args = parse_arguments::<TaskArgs>(&call.name, arguments)?;
            let parent_session = store
                .load_session_record(session_id)?
                .context("parent session not found")?;
            let description = args.description.trim();
            let prompt = args.prompt.trim();
            let subagent_type = args
                .subagent_type
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("general");

            if description.is_empty() {
                bail!("task description cannot be empty");
            }
            if prompt.is_empty() {
                bail!("task prompt cannot be empty");
            }

            let child_session_id = Uuid::new_v4();
            let child_title = format!("Task: {description}");
            store.create_session_with_parent(
                child_session_id,
                parent_session.session_id,
                workspace_root,
                &parent_session.provider_id,
                &parent_session.provider_display_name,
                &parent_session.model_id,
                &parent_session.model_display_name,
                &child_title,
            )?;

            let bootstrap_message = Message::new(
                MessageRole::System,
                format!(
                    "You are a {subagent_type} assistant. Work on the task and keep the response concise."
                ),
            );
            store.append_message(child_session_id, &bootstrap_message)?;

            let user_message = Message::new(MessageRole::User, prompt.to_string());
            store.append_message(child_session_id, &user_message)?;

            Ok(format!(
                "Started {subagent_type} subagent task '{description}'"
            ))
        }
        Some("todowrite") => {
            let args = parse_arguments::<TodoWriteArgs>(&call.name, arguments)?;
            validate_todos(&args.todos)?;
            store.replace_todos(session_id, &args.todos)?;
            let todos = store.load_todos(session_id)?;
            serde_json::to_string_pretty(&todos).context("failed to serialize todo list")
        }
        Some("skill") => {
            let args = parse_arguments::<SkillArgs>(&call.name, arguments)?;
            skills.render_skill(&args.name)
        }
        Some("websearch") => {
            let args = parse_arguments::<WebSearchArgs>(&call.name, arguments)?;
            let runtime = tokio::runtime::Handle::current();
            runtime.block_on(crate::webtools::websearch(
                &args.query,
                args.num_results,
                args.search_type.as_deref(),
            ))
        }
        Some("webfetch") => {
            let args = parse_arguments::<WebFetchArgs>(&call.name, arguments)?;
            let runtime = tokio::runtime::Handle::current();
            runtime.block_on(crate::webtools::webfetch(
                &args.url,
                args.format.as_deref(),
                args.timeout,
            ))
        }
        None => bail!("unknown tool '{}'", call.name),
        Some(other) => bail!("unsupported tool '{}'", other),
    }?;

    Ok(ToolExecutionResult::new(truncate_to_limit(
        output,
        max_output_bytes,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_schema_marks_optional_fields_as_optional() {
        let schema = EditArgs::schema();
        let object = schema.as_object().expect("schema should be an object");
        let required = object
            .get("required")
            .and_then(Value::as_array)
            .expect("required field should exist");
        let required_names = required
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();

        assert!(required_names.contains(&"path"));
        assert!(required_names.contains(&"old_text"));
        assert!(required_names.contains(&"new_text"));
        assert!(!required_names.contains(&"replace_all"));
    }

    #[test]
    fn resolve_workspace_path_rejects_escape_attempts() {
        let root = PathBuf::from("/tmp/tidev-workspace");
        let escaped = resolve_workspace_path(&root, Path::new("../outside"));
        assert!(escaped.is_err());
    }
}
