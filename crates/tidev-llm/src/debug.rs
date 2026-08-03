use chrono::Utc;
use uuid::Uuid;

/// Save the serialized request body to the system temp directory for
/// debugging prefix cache issues. Only saves when `enabled` is true.
/// Rotates old files when the count exceeds `max_files`.
pub fn save_request_for_debugging(request_body: &str, enabled: bool, max_files: usize) {
    if !enabled || max_files == 0 {
        return;
    }

    let dir = std::env::temp_dir().join("tidev-requests");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::debug!("debug_request: failed to create dir: {}", e);
        return;
    }

    rotate_files(&dir, max_files);

    // Pretty-print the request JSON with trailing newline
    let formatted = match serde_json::from_str::<serde_json::Value>(request_body) {
        Ok(val) => {
            let mut s =
                serde_json::to_string_pretty(&val).unwrap_or_else(|_| request_body.to_string());
            s.push('\n');
            s
        }
        Err(_) => {
            let mut s = request_body.to_string();
            s.push('\n');
            s
        }
    };

    let cst_offset = match chrono::FixedOffset::east_opt(8 * 3600) {
        Some(offset) => offset,
        None => return,
    };
    let now_cst = Utc::now().with_timezone(&cst_offset);
    let suffix = Uuid::new_v4().simple();
    let filename = format!("{}_{}.json", now_cst.format("%Y%m%d_%H%M%S_%3f"), suffix);
    let filepath = dir.join(&filename);
    if let Err(e) = std::fs::write(&filepath, formatted) {
        log::debug!("debug_request: failed to write {}: {}", filename, e);
    }
}

/// Save a non-streaming (complete) response as a pretty-printed JSON file.
/// Saves to the system temp directory when `enabled` is true and `max_files > 0`.
/// Rotates old files when the count exceeds `max_files`.
pub fn save_complete_response_for_debugging(response_body: &str, enabled: bool, max_files: usize) {
    if !enabled || max_files == 0 || response_body.is_empty() {
        return;
    }

    let dir = std::env::temp_dir().join("tidev-responses");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::debug!("save_complete_response: failed to create dir: {}", e);
        return;
    }

    rotate_files(&dir, max_files);

    // Pretty-print the response JSON
    let formatted = match serde_json::from_str::<serde_json::Value>(response_body) {
        Ok(val) => serde_json::to_string_pretty(&val).unwrap_or_else(|_| response_body.to_string()),
        Err(_) => response_body.to_string(),
    };

    let cst_offset = match chrono::FixedOffset::east_opt(8 * 3600) {
        Some(offset) => offset,
        None => return,
    };
    let now_cst = Utc::now().with_timezone(&cst_offset);
    let suffix = Uuid::new_v4().simple();
    let filename = format!(
        "response_{}_{}.json",
        now_cst.format("%Y%m%d_%H%M%S_%3f"),
        suffix
    );
    let filepath = dir.join(&filename);
    if let Err(e) = std::fs::write(&filepath, &formatted) {
        log::debug!(
            "save_complete_response: failed to write {}: {}",
            filename,
            e
        );
    }
}

/// Save raw SSE payloads from a streaming response to a JSONL file for debugging.
/// Each line is one `data:` payload (the JSON part after stripping the prefix).
/// Saves to the system temp directory when `enabled` is true and `max_files > 0`.
/// Rotates old files when the count exceeds `max_files`.
pub fn save_raw_response_for_debugging(
    payloads: &[String],
    enabled: bool,
    max_files: usize,
) {
    if !enabled || max_files == 0 || payloads.is_empty() {
        return;
    }

    let dir = std::env::temp_dir().join("tidev-responses");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::debug!("save_raw_response: failed to create dir: {}", e);
        return;
    }

    // Rotation: delete oldest files if over limit
    rotate_files(&dir, max_files);

    let cst_offset = match chrono::FixedOffset::east_opt(8 * 3600) {
        Some(offset) => offset,
        None => return,
    };
    let now_cst = Utc::now().with_timezone(&cst_offset);
    let suffix = Uuid::new_v4().simple();
    let filename = format!(
        "response_{}_{}.jsonl",
        now_cst.format("%Y%m%d_%H%M%S_%3f"),
        suffix,
    );
    let filepath = dir.join(&filename);

    let content = payloads.join("\n");
    if let Err(e) = std::fs::write(&filepath, &content) {
        log::debug!("save_raw_response: failed to write {}: {}", filename, e);
    }
}

fn rotate_files(dir: &std::path::Path, max_files: usize) {
    if let Ok(mut entries) =
        std::fs::read_dir(dir).map(|iter| iter.filter_map(|e| e.ok()).collect::<Vec<_>>())
    {
        entries.sort_by_key(|e| {
            std::fs::metadata(e.path())
                .ok()
                .and_then(|m| m.modified().ok())
        });
        while entries.len() >= max_files {
            if let Some(oldest) = entries.first() {
                let _ = std::fs::remove_file(oldest.path());
                entries.remove(0);
            }
        }
    }
}
