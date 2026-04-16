use anyhow::{Context, Result, anyhow, bail};
use grep::{
    regex::RegexMatcherBuilder,
    searcher::{SearcherBuilder, sinks},
};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read},
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

const DEFAULT_READ_LIMIT: usize = 2000;
const MAX_READ_BYTES: usize = 50 * 1024;
const MAX_LINE_LENGTH: usize = 2000;
const MAX_LINE_SUFFIX: &str = "... (line truncated to 2000 chars)";
const MAX_BYTES_LABEL: &str = "50 KB";

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
        offset: optional_integer("1-indexed line number to start reading from"),
        limit: optional_integer("Maximum number of lines to read"),
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
    pub struct ApplyPatchArgs {
        patch_text: string("The full patch text that describes all changes to be made"),
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
            "Read a text file inside the workspace, optionally using offset/limit to page large files",
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
        ToolDefinition::new::<ApplyPatchArgs>(
            "apply_patch",
            "Apply a unified diff patch to a file inside the workspace",
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
    read_file_with_options(workspace_root, relative_path, None, None)
}

pub fn read_file_with_options(
    workspace_root: &Path,
    relative_path: impl AsRef<Path>,
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<String> {
    let path = resolve_workspace_path(workspace_root, relative_path.as_ref())?;
    let file =
        fs::File::open(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = BufReader::new(file);

    let offset = offset.unwrap_or(1);
    let limit = limit.unwrap_or(DEFAULT_READ_LIMIT as i64);
    if offset < 1 {
        bail!("offset must be greater than or equal to 1");
    }
    if limit < 1 {
        bail!("limit must be greater than or equal to 1");
    }

    let mut lines = Vec::new();
    let mut total_lines = 0;
    let mut bytes = 0;
    let mut cut = false;
    let mut more = false;
    let mut raw_line = String::new();

    while reader.read_line(&mut raw_line)? > 0 {
        total_lines += 1;
        if total_lines < offset as usize {
            raw_line.clear();
            continue;
        }

        if lines.len() >= limit as usize {
            more = true;
            raw_line.clear();
            continue;
        }

        let trimmed = raw_line.trim_end_matches(&['\r', '\n'][..]);
        let text = truncate_line_to_limit(trimmed);
        let size = text.as_bytes().len() + if lines.is_empty() { 0 } else { 1 };
        if bytes + size > MAX_READ_BYTES {
            cut = true;
            more = true;
            break;
        }

        bytes += size;
        lines.push(text);
        raw_line.clear();
    }

    if total_lines < offset as usize && !(total_lines == 0 && offset == 1) {
        bail!(
            "Offset {} is out of range for this file ({} lines)",
            offset,
            total_lines
        );
    }

    let start = offset as usize;
    let last = start + lines.len().saturating_sub(1);
    let next_offset = start as i64 + lines.len() as i64;
    let mut output = lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| format!("{}: {}", start + i, line))
        .collect::<Vec<_>>()
        .join("\n");

    if cut {
        output.push_str(&format!(
            "\n\n(Output capped at {}. Showing lines {}-{}. Use offset={} to continue.)",
            MAX_BYTES_LABEL, start, last, next_offset
        ));
    } else if more {
        output.push_str(&format!(
            "\n\n(Showing lines {}-{} of {}. Use offset={} to continue.)",
            start, last, total_lines, next_offset
        ));
    } else {
        output.push_str(&format!("\n\n(End of file - total {} lines)", total_lines));
    }

    Ok(output)
}

fn truncate_line_to_limit(line: &str) -> String {
    if line.chars().count() <= MAX_LINE_LENGTH {
        line.to_string()
    } else {
        line.chars().take(MAX_LINE_LENGTH).collect::<String>() + MAX_LINE_SUFFIX
    }
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
        return Ok(new_text.to_string());
    }

    let ending = detect_line_ending(contents);
    let old = convert_to_line_ending(&normalize_line_endings(old_text), ending);
    let new_text = convert_to_line_ending(&normalize_line_endings(new_text), ending);

    let candidates = find_edit_candidates(contents, &old);
    for candidate in candidates {
        if !contents.contains(&candidate) {
            continue;
        }

        if replace_all {
            return Ok(contents.replace(&candidate, &new_text));
        }

        let occurrences = contents.match_indices(&candidate).count();
        if occurrences == 1 {
            return Ok(replace_first_occurrence(contents, &candidate, &new_text));
        }
    }

    let direct_matches = contents.match_indices(&old).count();
    if direct_matches == 1 {
        return Ok(replace_first_occurrence(contents, &old, &new_text));
    }
    if direct_matches > 1 {
        bail!("text occurs multiple times; set replace_all to true");
    }

    bail!("text not found in file");
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn detect_line_ending(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

fn convert_to_line_ending(text: &str, ending: &str) -> String {
    if ending == "\n" {
        text.to_string()
    } else {
        text.replace("\n", "\r\n")
    }
}

fn replace_first_occurrence(content: &str, old: &str, new_text: &str) -> String {
    if let Some(index) = content.find(old) {
        let mut result = String::with_capacity(content.len() - old.len() + new_text.len());
        result.push_str(&content[..index]);
        result.push_str(new_text);
        result.push_str(&content[index + old.len()..]);
        result
    } else {
        content.to_string()
    }
}

fn find_edit_candidates(content: &str, old_text: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    if content.contains(old_text) {
        candidates.push(old_text.to_string());
    }
    candidates.extend(line_trimmed_replacer(content, old_text));
    candidates.extend(block_anchor_replacer(content, old_text));
    candidates.extend(whitespace_normalized_replacer(content, old_text));
    candidates.extend(indentation_flexible_replacer(content, old_text));
    candidates.extend(escape_normalized_replacer(content, old_text));
    candidates.extend(trimmed_boundary_replacer(content, old_text));
    candidates.extend(context_aware_replacer(content, old_text));
    candidates.extend(multi_occurrence_replacer(content, old_text));

    candidates.dedup();
    candidates
}

fn split_lines_inclusive(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n').collect()
}

fn trim_newline(line: &str) -> &str {
    line.strip_suffix('\n').unwrap_or(line)
}

fn trim_line(line: &str) -> String {
    trim_newline(line).trim().to_string()
}

fn line_slice<'a>(content: &'a str, lines: &[&'a str], start: usize, end: usize) -> &'a str {
    let start_byte: usize = lines[..start].iter().map(|l| l.len()).sum();
    let end_byte: usize = lines[..end + 1].iter().map(|l| l.len()).sum();
    &content[start_byte..end_byte]
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn remove_indentation(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let min_indent = lines
        .iter()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                None
            } else {
                Some(line.len() - trimmed.len())
            }
        })
        .min()
        .unwrap_or(0);

    lines
        .into_iter()
        .map(|line| {
            if line.trim().is_empty() {
                line.trim_end().to_string()
            } else {
                line.chars().skip(min_indent).collect()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn unescape_string(text: &str) -> String {
    let mut output = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                match next {
                    'n' => output.push('\n'),
                    't' => output.push('\t'),
                    'r' => output.push('\r'),
                    '\\' => output.push('\\'),
                    '"' => output.push('"'),
                    '\'' => output.push('\''),
                    '`' => output.push('`'),
                    '$' => output.push('$'),
                    _ => {
                        output.push('\\');
                        output.push(next);
                    }
                }
            } else {
                output.push('\\');
            }
        } else {
            output.push(c);
        }
    }
    output
}

fn levenshtein(a: &str, b: &str) -> usize {
    if a.is_empty() || b.is_empty() {
        return a.len().max(b.len());
    }

    let mut matrix: Vec<Vec<usize>> = vec![vec![0; b.len() + 1]; a.len() + 1];
    for i in 0..=a.len() {
        matrix[i][0] = i;
    }
    for j in 0..=b.len() {
        matrix[0][j] = j;
    }

    for (i, a_ch) in a.chars().enumerate() {
        for (j, b_ch) in b.chars().enumerate() {
            let cost = if a_ch == b_ch { 0 } else { 1 };
            matrix[i + 1][j + 1] = *[
                matrix[i][j + 1] + 1,
                matrix[i + 1][j] + 1,
                matrix[i][j] + cost,
            ]
            .iter()
            .min()
            .unwrap();
        }
    }

    matrix[a.len()][b.len()]
}

fn line_trimmed_replacer(content: &str, find: &str) -> Vec<String> {
    let original_lines = split_lines_inclusive(content);
    let mut search_lines = split_lines_inclusive(find);
    if search_lines
        .last()
        .map(|line| trim_newline(line).is_empty())
        == Some(true)
    {
        search_lines.pop();
    }

    if search_lines.is_empty() {
        return Vec::new();
    }

    let search_trimmed: Vec<String> = search_lines.iter().map(|line| trim_line(line)).collect();
    let needed = search_trimmed.len();
    let original_trimmed: Vec<String> = original_lines.iter().map(|line| trim_line(line)).collect();

    let mut results = Vec::new();
    if original_trimmed.len() < needed {
        return results;
    }

    for start in 0..=original_trimmed.len() - needed {
        if original_trimmed[start..start + needed] == search_trimmed[..] {
            results
                .push(line_slice(content, &original_lines, start, start + needed - 1).to_string());
        }
    }

    results
}

const SINGLE_CANDIDATE_SIMILARITY_THRESHOLD: f64 = 0.0;
const MULTIPLE_CANDIDATES_SIMILARITY_THRESHOLD: f64 = 0.3;

fn block_anchor_replacer(content: &str, find: &str) -> Vec<String> {
    let original_lines = split_lines_inclusive(content);
    let mut search_lines = split_lines_inclusive(find);
    if search_lines
        .last()
        .map(|line| trim_newline(line).is_empty())
        == Some(true)
    {
        search_lines.pop();
    }
    if search_lines.len() < 3 {
        return Vec::new();
    }

    let original_trimmed: Vec<String> = original_lines.iter().map(|line| trim_line(line)).collect();
    let search_trimmed: Vec<String> = search_lines.iter().map(|line| trim_line(line)).collect();
    let first = &search_trimmed[0];
    let last = &search_trimmed[search_trimmed.len() - 1];
    let mut candidates = Vec::new();

    for i in 0..original_trimmed.len() {
        if &original_trimmed[i] != first {
            continue;
        }
        for j in i + 2..original_trimmed.len() {
            if &original_trimmed[j] == last {
                candidates.push((i, j));
                break;
            }
        }
    }

    if candidates.is_empty() {
        return Vec::new();
    }

    if candidates.len() == 1 {
        let (start, end) = candidates[0];
        if anchor_similarity(&original_trimmed, &search_trimmed, start, end)
            >= SINGLE_CANDIDATE_SIMILARITY_THRESHOLD
        {
            return vec![line_slice(content, &original_lines, start, end).to_string()];
        }
        return Vec::new();
    }

    let mut best_match = None;
    let mut max_similarity = -1.0;
    for (start, end) in candidates {
        let similarity = anchor_similarity(&original_trimmed, &search_trimmed, start, end);
        if similarity > max_similarity {
            max_similarity = similarity;
            best_match = Some((start, end));
        }
    }

    if max_similarity >= MULTIPLE_CANDIDATES_SIMILARITY_THRESHOLD {
        if let Some((start, end)) = best_match {
            return vec![line_slice(content, &original_lines, start, end).to_string()];
        }
    }
    Vec::new()
}

fn anchor_similarity(
    original_trimmed: &[String],
    search_trimmed: &[String],
    start: usize,
    end: usize,
) -> f64 {
    let actual_block_size = end - start + 1;
    let search_mid = if search_trimmed.len() > 2 {
        &search_trimmed[1..search_trimmed.len() - 1]
    } else {
        &[]
    };
    let actual_mid = if actual_block_size > 2 {
        &original_trimmed[start + 1..end]
    } else {
        &[]
    };

    if search_mid.is_empty() || actual_mid.is_empty() {
        return 1.0;
    }

    let lines_to_check = std::cmp::min(search_mid.len(), actual_mid.len());
    let mut similarity = 0.0;
    for i in 0..lines_to_check {
        let original_line = &actual_mid[i];
        let search_line = &search_mid[i];
        let max_len = original_line.len().max(search_line.len());
        if max_len == 0 {
            continue;
        }
        let distance = levenshtein(original_line, search_line);
        similarity += 1.0 - (distance as f64 / max_len as f64);
    }
    similarity / lines_to_check as f64
}

fn whitespace_normalized_replacer(content: &str, find: &str) -> Vec<String> {
    let normalized_find = normalize_whitespace(find);
    let lines = split_lines_inclusive(content);
    let mut results = Vec::new();

    for line in &lines {
        let line_norm = normalize_whitespace(trim_newline(line));
        if line_norm == normalized_find || line_norm.contains(&normalized_find) {
            results.push(line.to_string());
        }
    }

    let find_lines: Vec<&str> = find.lines().collect();
    if find_lines.len() > 1 {
        for start in 0..=lines.len().saturating_sub(find_lines.len()) {
            let block = lines[start..start + find_lines.len()].concat();
            if normalize_whitespace(trim_newline(&block)) == normalized_find {
                results.push(block);
            }
        }
    }

    results
}

fn indentation_flexible_replacer(content: &str, find: &str) -> Vec<String> {
    let normalized_find = remove_indentation(find);
    let lines = split_lines_inclusive(content);
    let mut results = Vec::new();
    let find_count = split_lines_inclusive(find).len();

    if find_count == 0 {
        return results;
    }

    for start in 0..=lines.len().saturating_sub(find_count) {
        let block = lines[start..start + find_count].concat();
        if remove_indentation(&block) == normalized_find {
            results.push(block);
        }
    }

    results
}

fn escape_normalized_replacer(content: &str, find: &str) -> Vec<String> {
    let unescaped_find = unescape_string(find);
    let mut results = Vec::new();

    if content.contains(&unescaped_find) {
        results.push(unescaped_find.clone());
    }

    let lines = split_lines_inclusive(content);
    let find_lines = split_lines_inclusive(&unescaped_find);
    if find_lines.len() > 1 {
        for start in 0..=lines.len().saturating_sub(find_lines.len()) {
            let block = lines[start..start + find_lines.len()].concat();
            if unescape_string(&block) == unescaped_find {
                results.push(block);
            }
        }
    }

    results
}

fn trimmed_boundary_replacer(content: &str, find: &str) -> Vec<String> {
    let trimmed_find = find.trim();
    if trimmed_find == find {
        return Vec::new();
    }

    let mut results = Vec::new();
    if content.contains(trimmed_find) {
        results.push(trimmed_find.to_string());
    }

    let lines = split_lines_inclusive(content);
    let find_count = split_lines_inclusive(find).len();
    if find_count == 0 {
        return results;
    }

    for start in 0..=lines.len().saturating_sub(find_count) {
        let block = lines[start..start + find_count].concat();
        if block.trim() == trimmed_find {
            results.push(block);
        }
    }

    results
}

fn context_aware_replacer(content: &str, find: &str) -> Vec<String> {
    let mut search_lines = split_lines_inclusive(find);
    if search_lines.len() < 3 {
        return Vec::new();
    }
    if search_lines
        .last()
        .map(|line| trim_newline(line).is_empty())
        == Some(true)
    {
        search_lines.pop();
    }
    if search_lines.len() < 3 {
        return Vec::new();
    }

    let content_lines = split_lines_inclusive(content);
    let find_trimmed: Vec<String> = search_lines.iter().map(|line| trim_line(line)).collect();
    let first = &find_trimmed[0];
    let last = &find_trimmed[find_trimmed.len() - 1];
    let mut results = Vec::new();

    for start in 0..content_lines.len() {
        if trim_line(content_lines[start]) != *first {
            continue;
        }
        for end in start + 2..content_lines.len() {
            if trim_line(content_lines[end]) != *last {
                continue;
            }
            let block_lines = &content_lines[start..=end];
            if block_lines.len() != search_lines.len() {
                continue;
            }

            let mut matching = 0;
            let mut total = 0;
            for i in 1..block_lines.len() - 1 {
                let block_line = trim_line(block_lines[i]);
                let find_line = find_trimmed[i].clone();
                if !block_line.is_empty() || !find_line.is_empty() {
                    total += 1;
                    if block_line == find_line {
                        matching += 1;
                    }
                }
            }

            if total == 0 || matching * 2 >= total {
                results.push(block_lines.concat());
                break;
            }
        }
    }

    results
}

fn multi_occurrence_replacer(content: &str, find: &str) -> Vec<String> {
    let mut results = Vec::new();
    let mut offset = 0;
    while let Some(index) = content[offset..].find(find) {
        results.push(find.to_string());
        offset += index + find.len();
    }
    results
}

fn extract_patch_file_path(patch: &diffy::Patch<'_, str>) -> Result<String> {
    let file_path = patch
        .modified()
        .or_else(|| patch.original())
        .ok_or_else(|| anyhow!("patch is missing file path header"))?
        .trim();

    let file_path = file_path
        .strip_prefix("a/")
        .or_else(|| file_path.strip_prefix("b/"))
        .unwrap_or(file_path);

    if file_path.is_empty() {
        bail!("patch file path is empty");
    }

    Ok(file_path.to_string())
}

fn apply_patch_contents(contents: &str, patch: &diffy::Patch<'_, str>) -> Result<String> {
    let line_fragments = split_lines_inclusive(contents);
    let mut result = String::new();
    let mut cursor = 0usize;

    for hunk in patch.hunks() {
        let old_start = hunk.old_range().start();
        let old_len = hunk.old_range().len();
        let old_index = old_start.saturating_sub(1);

        if old_index > line_fragments.len() {
            bail!("patch hunk refers to a line outside the file");
        }

        result.push_str(&line_fragments[cursor..old_index].concat());

        let mut source_index = old_index;
        for line in hunk.lines() {
            match line {
                diffy::Line::Context(text) => {
                    if source_index >= line_fragments.len() || line_fragments[source_index] != *text
                    {
                        bail!("patch context does not match file contents");
                    }
                    result.push_str(text);
                    source_index += 1;
                }
                diffy::Line::Delete(text) => {
                    if source_index >= line_fragments.len() || line_fragments[source_index] != *text
                    {
                        bail!("patch delete hunk does not match file contents");
                    }
                    source_index += 1;
                }
                diffy::Line::Insert(text) => {
                    result.push_str(text);
                }
            }
        }

        cursor = old_index + old_len;
    }

    result.push_str(&line_fragments[cursor..].concat());
    Ok(result)
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
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", command])
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
            read_file_with_options(workspace_root, args.path, args.offset, args.limit)
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
        Some("apply_patch") => {
            let args = parse_arguments::<ApplyPatchArgs>(&call.name, arguments)?;
            let patch = diffy::Patch::from_str(&args.patch_text)
                .with_context(|| format!("failed to parse patch for tool '{}'", call.name))?;
            let file_path = extract_patch_file_path(&patch)
                .with_context(|| format!("failed to determine file path from patch"))?;
            let absolute_path = resolve_workspace_path(workspace_root, Path::new(&file_path))?;
            let old_content = read_existing_text(&absolute_path)?;
            let updated = apply_patch_contents(&old_content, &patch)?;

            if updated.is_empty() {
                if absolute_path.exists() {
                    fs::remove_file(&absolute_path)
                        .with_context(|| format!("failed to remove {}", absolute_path.display()))?;
                }
            } else {
                if let Some(parent) = absolute_path.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("failed to create directory {}", parent.display())
                    })?;
                }
                fs::write(&absolute_path, &updated)
                    .with_context(|| format!("failed to write {}", absolute_path.display()))?;
            }

            Ok(file_change_output(
                workspace_root,
                &absolute_path,
                &old_content,
                &updated,
                "Patched",
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

            store.copy_tool_permissions(parent_session.session_id, child_session_id)?;

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

    #[test]
    fn read_file_with_options_pages_large_files() {
        let root =
            PathBuf::from(std::env::temp_dir()).join(format!("tidev-read-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("failed to create temp workspace");

        let path = root.join("sample.txt");
        let content = (1..=100)
            .map(|i| format!("line {}\n", i))
            .collect::<String>();
        fs::write(&path, &content).expect("failed to write sample file");

        let relative = path.strip_prefix(&root).expect("path should be relative");
        let output =
            read_file_with_options(&root, relative, Some(10), Some(5)).expect("read failed");

        assert!(output.contains("10: line 10"));
        assert!(output.contains("14: line 14"));
        assert!(output.contains("Use offset=15 to continue"));

        fs::remove_file(&path).expect("failed to remove temp file");
        fs::remove_dir(&root).expect("failed to remove temp dir");
    }

    #[test]
    fn apply_patch_contents_creates_new_file_content() {
        let patch = diffy::Patch::from_str(
            "--- /dev/null\n+++ b/foo.txt\n@@ -0,0 +1,2 @@\n+hello\n+world\n",
        )
        .expect("failed to parse patch");
        let updated = apply_patch_contents("", &patch).expect("patch apply failed");
        assert_eq!(updated, "hello\nworld\n");
    }

    #[test]
    fn apply_patch_contents_updates_existing_file() {
        let patch = diffy::Patch::from_str(
            "--- a/foo.txt\n+++ b/foo.txt\n@@ -1,2 +1,2 @@\n-hello\n+hi\n world\n",
        )
        .expect("failed to parse patch");
        let original = "hello\nworld\n";
        let updated = apply_patch_contents(original, &patch).expect("patch apply failed");
        assert_eq!(updated, "hi\nworld\n");
    }

    #[test]
    fn apply_patch_contents_deletes_only_removed_text() {
        let patch = diffy::Patch::from_str(
            "--- a/foo.txt\n+++ b/foo.txt\n@@ -1,2 +1,1 @@\n-hello\n world\n",
        )
        .expect("failed to parse patch");
        let original = "hello\nworld\n";
        let updated = apply_patch_contents(original, &patch).expect("patch apply failed");
        assert_eq!(updated, "world\n");
    }

    #[test]
    fn apply_edit_contents_line_trimmed_match() {
        let original = "  hello \n  world \n";
        let updated = apply_edit_contents(original, "hello\nworld\n", "hi\n", false)
            .expect("edit apply failed");
        assert_eq!(updated, "hi\n");
    }

    #[test]
    fn apply_edit_contents_indentation_flexible_match() {
        let original = "    fn main() {\n        println!(\"hi\");\n    }\n";
        let updated = apply_edit_contents(
            original,
            "fn main() {\n    println!(\"hi\");\n}\n",
            "fn main() {\n    println!(\"hello\");\n}\n",
            false,
        )
        .expect("edit apply failed");
        assert_eq!(updated, "fn main() {\n    println!(\"hello\");\n}\n");
    }

    #[test]
    fn extract_patch_file_path_strips_a_or_b_prefix() {
        let patch =
            diffy::Patch::from_str("--- a/foo.txt\n+++ b/foo.txt\n@@ -1,1 +1,1 @@\n-hello\n+hi\n")
                .expect("failed to parse patch");
        assert_eq!(extract_patch_file_path(&patch).unwrap(), "foo.txt");
    }
}
