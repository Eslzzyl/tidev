use chrono::Utc;
use uuid::Uuid;

/// Save the serialized request body to /tmp/tidev-requests/ for debugging
/// prefix cache issues. Only saves when `enabled` is true.
/// Rotates old files when the count exceeds `max_files`.
pub fn save_request_for_debugging(request_body: &str, enabled: bool, max_files: usize) {
    if !enabled || max_files == 0 {
        return;
    }

    let dir = std::path::Path::new("/tmp/tidev-requests");
    if let Err(e) = std::fs::create_dir_all(dir) {
        crate::log_debug!("debug_request: failed to create dir: {}", e);
        return;
    }

    // Rotation: delete oldest files if over limit
    if let Ok(mut entries) = std::fs::read_dir(dir)
        .map(|iter| iter.filter_map(|e| e.ok()).collect::<Vec<_>>())
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

    let cst_offset = match chrono::FixedOffset::east_opt(8 * 3600) {
        Some(offset) => offset,
        None => return,
    };
    let now_cst = Utc::now().with_timezone(&cst_offset);
    let suffix = Uuid::new_v4().simple();
    let filename = format!("{}_{}.json", now_cst.format("%Y%m%d_%H%M%S_%3f"), suffix);
    let filepath = dir.join(&filename);
    if let Err(e) = std::fs::write(&filepath, request_body) {
        crate::log_debug!("debug_request: failed to write {}: {}", filename, e);
    }
}
