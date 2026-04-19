use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use diffy::DiffOptions;
use serde_json::Value;
use std::{fs, io::BufRead, path::Path};

use super::utils::{display_workspace_relative, read_existing_text, resolve_workspace_path};
use crate::instructions::resolve_nearby_instructions;
use crate::session::{MessageAttachment, ToolExecutionResult};
use crate::tooling::tools::{ApplyPatchArgs, EditArgs, ListArgs, ReadArgs, WriteArgs};
use crate::tooling::{ToolDefinition, ToolPermission};

const MAX_LINE_LENGTH: usize = 2000;
const MAX_LINE_SUFFIX: &str = "... (line truncated to 2000 chars)";

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::new::<ReadArgs>(
            "read",
            "Read a file or directory inside the workspace. Supports text files (with pagination), images (as attachments), and directory listing.",
            ToolPermission::Read,
        ),
        ToolDefinition::new::<WriteArgs>(
            "write",
            "Write a text file inside the workspace. If this is an existing file, you MUST use the Read tool first to read the file's contents. This tool will fail if you did not read the file first.",
            ToolPermission::Write,
        ),
        ToolDefinition::new::<EditArgs>(
            "edit",
            "Edit a file by replacing text inside it. You MUST use the Read tool at least once in the conversation before editing. This tool will error if you attempt an edit without reading the file. If the file has been modified since you last read it, you must read it again before editing.",
            ToolPermission::Edit,
        ),
        ToolDefinition::new::<ApplyPatchArgs>(
            "apply_patch",
            "Apply a unified diff patch to a file inside the workspace. You MUST use the Read tool at least once before applying a patch to an existing file. This tool will fail if you did not read the file first.",
            ToolPermission::Edit,
        ),
        ToolDefinition::new::<ListArgs>(
            "list",
            "List entries in a directory inside the workspace",
            ToolPermission::Read,
        ),
    ]
}

pub fn execute_tool_call(
    workspace_root: &Path,
    config_dir: &Path,
    call: &crate::session::ToolCall,
    _max_output_bytes: usize,
) -> Result<crate::session::ToolExecutionResult> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;

    match crate::tooling::canonical_tool_name(&call.name) {
        Some("read") => {
            let args = serde_json::from_value::<ReadArgs>(arguments)
                .with_context(|| format!("failed to decode arguments for tool '{}'", call.name))?;
            read_path(
                workspace_root,
                config_dir,
                args.path,
                args.offset,
                args.limit,
            )
        }
        Some("write") => {
            let args = serde_json::from_value::<WriteArgs>(arguments)
                .with_context(|| format!("failed to decode arguments for tool '{}'", call.name))?;
            let absolute_path = resolve_workspace_path(workspace_root, Path::new(&args.path))?;
            let original_exists = absolute_path.exists();
            write_file(workspace_root, &args.path, &args.content)?;
            Ok(crate::session::ToolExecutionResult::new(
                file_change_output(
                    workspace_root,
                    &absolute_path,
                    "",
                    &args.content,
                    "Wrote",
                    original_exists,
                ),
            ))
        }
        Some("edit") => {
            let args = serde_json::from_value::<EditArgs>(arguments)
                .with_context(|| format!("failed to decode arguments for tool '{}'", call.name))?;
            let output = edit_file(
                workspace_root,
                &args.path,
                &args.old_text,
                &args.new_text,
                args.replace_all.unwrap_or(false),
            )?;
            Ok(crate::session::ToolExecutionResult::new(output))
        }
        Some("apply_patch") => {
            let args = serde_json::from_value::<ApplyPatchArgs>(arguments)
                .with_context(|| format!("failed to decode arguments for tool '{}'", call.name))?;
            let patch = diffy::Patch::from_str(&args.patch_text)
                .with_context(|| format!("failed to parse patch for tool '{}'", call.name))?;
            let file_path = extract_patch_file_path(&patch)
                .with_context(|| "failed to determine file path from patch".to_string())?;
            let absolute_path = resolve_workspace_path(workspace_root, Path::new(&file_path))?;
            let original_exists = absolute_path.exists();
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

            Ok(crate::session::ToolExecutionResult::new(
                file_change_output(
                    workspace_root,
                    &absolute_path,
                    &old_content,
                    &updated,
                    "Patched",
                    original_exists,
                ),
            ))
        }
        Some("list") => {
            let args = serde_json::from_value::<ListArgs>(arguments)
                .with_context(|| format!("failed to decode arguments for tool '{}'", call.name))?;
            let path = args.path.unwrap_or_else(|| ".".to_string());
            let output = list_dir(workspace_root, path)?;
            Ok(crate::session::ToolExecutionResult::new(output))
        }
        Some(other) => bail!("unsupported file tool '{}'", other),
        None => bail!("unknown tool '{}'", call.name),
    }
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

pub(super) fn apply_patch_contents(
    contents: &str,
    patch: &diffy::Patch<'_, str>,
) -> Result<String> {
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

fn truncate_line_to_limit(line: &str) -> String {
    if line.chars().count() <= MAX_LINE_LENGTH {
        line.to_string()
    } else {
        line.chars().take(MAX_LINE_LENGTH).collect::<String>() + MAX_LINE_SUFFIX
    }
}

fn file_change_output(
    workspace_root: &Path,
    absolute_path: &Path,
    old_content: &str,
    new_content: &str,
    action: &str,
    original_exists: bool,
) -> String {
    let relative = display_workspace_relative(workspace_root, absolute_path);
    let mut options = DiffOptions::new();
    if !original_exists {
        options.set_context_len(0);
    }
    options.set_original_filename(format!("a/{relative}"));
    options.set_modified_filename(format!("b/{relative}"));
    let patch = options.create_patch(old_content, new_content);

    if patch.hunks().is_empty() {
        format!("{action} {relative} (no content changes)")
    } else if !original_exists {
        let patch_text = patch.to_string();
        format!("new file mode 100644\n{patch_text}")
    } else {
        patch.to_string()
    }
}

pub(super) fn read_path(
    workspace_root: &Path,
    config_dir: &Path,
    relative_path: impl AsRef<Path>,
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<ToolExecutionResult> {
    let path = resolve_workspace_path(workspace_root, relative_path.as_ref())?;

    if !path.exists() {
        let suggestions = find_fuzzy_suggestions(workspace_root, relative_path.as_ref())?;
        if suggestions.is_empty() {
            bail!("failed to read {}: file not found", path.display());
        } else {
            bail!(
                "failed to read {}: file not found. Did you mean one of these?\n{}",
                path.display(),
                suggestions.join("\n")
            );
        }
    }

    if path.is_dir() {
        let output = list_dir(workspace_root, &relative_path)?;
        let mut result = ToolExecutionResult::new(output);
        result
            .attachments
            .push(MessageAttachment::DirectoryReference {
                path: display_workspace_relative(workspace_root, &path),
                tree: "".to_string(),
            });
        return Ok(result);
    }

    // Treat as potential image
    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    let mime_str = mime.to_string();

    if mime_str.starts_with("image/") {
        let content =
            fs::read(&path).with_context(|| format!("failed to read image {}", path.display()))?;
        let data_url = format!(
            "data:{};base64,{}",
            mime_str,
            BASE64_STANDARD.encode(content)
        );

        let mut result = ToolExecutionResult::new("Image read successfully.");
        result.attachments.push(MessageAttachment::Image {
            filename: path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            mime: mime_str,
            data_url,
        });
        return Ok(result);
    }

    // Detect if binary but not an image
    if is_binary_file(&path)? {
        bail!("Cannot read binary file: {}", path.display());
    }

    // Treat as text file
    let file =
        fs::File::open(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = std::io::BufReader::new(file);

    let offset = offset.unwrap_or(1);
    let limit = limit.unwrap_or(2000);
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
        let size = text.len() + if lines.is_empty() { 0 } else { 1 };
        if bytes + size > 50 * 1024 {
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
            total_lines,
        );
    }

    let start = offset as usize;
    let last = start + lines.len().saturating_sub(1);
    let next_offset = start as i64 + lines.len() as i64;
    let mut content_str = lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| format!("{}: {}", start + i, line))
        .collect::<Vec<_>>()
        .join("\n");

    if cut {
        content_str.push_str(&format!(
            "\n\n(Output capped at 50 KB. Showing lines {}-{}. Use offset={} to continue.)",
            start, last, next_offset
        ));
    } else if more {
        content_str.push_str(&format!(
            "\n\n(Showing lines {}-{} of {}. Use offset={} to continue.)",
            start, last, total_lines, next_offset
        ));
    } else {
        content_str.push_str(&format!("\n\n(End of file - total {} lines)", total_lines));
    }

    // Load nearby instruction files
    let instructions = resolve_nearby_instructions(workspace_root, config_dir, &path)?;

    // Build XML-style output
    let mut output = format!(
        "<path>{}</path>\n<type>file</type>\n<content>\n{}\n</content>",
        display_workspace_relative(workspace_root, &path),
        content_str
    );

    if !instructions.is_empty() {
        let reminders: Vec<String> = instructions.into_iter().map(|(_, c)| c).collect();
        output.push_str(&format!(
            "\n\n<system-reminder>\n{}\n</system-reminder>",
            reminders.join("\n\n")
        ));
    }

    Ok(ToolExecutionResult::new(output))
}

fn is_binary_file(path: &Path) -> Result<bool> {
    use std::io::Read;
    let mut f = fs::File::open(path)?;
    let mut buf = [0u8; 1024];
    let n = f.read(&mut buf)?;
    Ok(buf[..n].iter().any(|&b| b == 0))
}

fn find_fuzzy_suggestions(workspace_root: &Path, relative_path: &Path) -> Result<Vec<String>> {
    let parent = relative_path.parent().unwrap_or(Path::new("."));
    let absolute_parent = resolve_workspace_path(workspace_root, parent)?;
    if !absolute_parent.exists() || !absolute_parent.is_dir() {
        return Ok(vec![]);
    }

    let target_name = relative_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if target_name.is_empty() {
        return Ok(vec![]);
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(absolute_parent)? {
        let entry = entry?;
        entries.push(entry.file_name().to_string_lossy().to_string());
    }

    let mut suggestions: Vec<(f64, String)> = entries
        .into_iter()
        .map(|name| (strsim::jaro_winkler(&target_name, &name), name))
        .filter(|(sim, _)| *sim > 0.8)
        .collect();

    suggestions.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    Ok(suggestions
        .into_iter()
        .take(3)
        .map(|(_, name)| {
            let mut p = parent.to_path_buf();
            p.push(name);
            p.to_string_lossy().to_string()
        })
        .collect())
}

#[cfg(test)]
pub(super) fn read_file_with_options(
    _workspace_root: &Path,
    _config_dir: &Path,
    _relative_path: impl AsRef<Path>,
    _offset: Option<i64>,
    _limit: Option<i64>,
) -> Result<String> {
    bail!("deprecated: use read_path instead")
}

pub(super) fn write_file(
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

pub(super) fn list_dir(workspace_root: &Path, relative_path: impl AsRef<Path>) -> Result<String> {
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
        Ok(format!(
            "{}/\n{}",
            display_workspace_relative(workspace_root, &path),
            entries.join("\n")
        ))
    }
}

pub(super) fn edit_file(
    workspace_root: &Path,
    relative_path: impl AsRef<Path>,
    old_text: &str,
    new_text: &str,
    replace_all: bool,
) -> Result<String> {
    let path = resolve_workspace_path(workspace_root, relative_path.as_ref())?;
    let original_exists = path.exists();
    let old_contents =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;

    let new_contents = apply_edit_contents(&old_contents, old_text, new_text, replace_all)?;
    fs::write(&path, &new_contents)
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(file_change_output(
        workspace_root,
        &path,
        &old_contents,
        &new_contents,
        "Edited",
        original_exists,
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

    bail!("text not found in file")
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
    #[allow(clippy::needless_range_loop)]
    for i in 0..=a.len() {
        matrix[i][0] = i;
    }
    #[allow(clippy::needless_range_loop)]
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
const MULTIPLE_CANDIDATE_SIMILARITY_THRESHOLD: f64 = 0.3;

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
        #[allow(clippy::needless_range_loop)]
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

    if max_similarity >= MULTIPLE_CANDIDATE_SIMILARITY_THRESHOLD
        && let Some((start, end)) = best_match
    {
        return vec![line_slice(content, &original_lines, start, end).to_string()];
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
