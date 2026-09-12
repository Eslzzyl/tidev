use super::*;

// ---------------------------------------------------------------------------
// Snapshot storage
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Save snapshot data for a message.
    pub fn save_snapshot(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        snapshot_hash: &str,
        patch_files: &str,
        file_diffs: &str,
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        let app_data_blob: Option<Vec<u8>> = conn
            .query_row(
                "SELECT app_data FROM messages WHERE id = ?1 AND session_id = ?2",
                params![message_id.to_string(), session_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let mut app_data: MessageAppData = app_data_blob
            .filter(|b| !b.is_empty())
            .map(|b| serde_json::from_str(&decompress_text(&b)).unwrap_or_default())
            .unwrap_or_default();
        app_data.snapshot_hash = Some(snapshot_hash.to_string());
        app_data.patch_files = if patch_files.is_empty() {
            None
        } else {
            Some(patch_files.to_string())
        };
        app_data.file_diffs = if file_diffs.is_empty() {
            None
        } else {
            Some(file_diffs.to_string())
        };
        let blob = if app_data == MessageAppData::default() {
            Vec::new()
        } else {
            let json = serde_json::to_string(&app_data).unwrap_or_else(|_| "{}".to_string());
            compress_text(&json)
        };
        conn.execute(
            "UPDATE messages SET app_data = ?1 WHERE id = ?2 AND session_id = ?3",
            params![blob, message_id.to_string(), session_id.to_string(),],
        )?;
        Ok(())
    }

    /// Load snapshot patch data for a message.
    pub fn load_snapshot(&self, message_id: Uuid) -> Result<Option<(String, String, String)>> {
        self.read(|conn| {
            let mut stmt = conn.prepare("SELECT app_data FROM messages WHERE id = ?1")?;
            let mut rows = stmt.query_map(params![message_id.to_string()], |row| {
                let blob: Vec<u8> = row.get(0).unwrap_or_default();
                let app_data: MessageAppData = if blob.is_empty() {
                    MessageAppData::default()
                } else {
                    serde_json::from_str(&decompress_text(&blob)).unwrap_or_default()
                };
                let hash = app_data.snapshot_hash.unwrap_or_default();
                let patches = app_data.patch_files.unwrap_or_default();
                let diffs = app_data.file_diffs.unwrap_or_default();
                Ok((hash, patches, diffs))
            })?;
            match rows.next() {
                Some(Ok(result)) => {
                    if result.0.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(result))
                    }
                }
                _ => Ok(None),
            }
        })
    }
}
