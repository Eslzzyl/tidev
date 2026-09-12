use super::*;

// ---------------------------------------------------------------------------
// Session instruction sources
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Save instruction sources for a session (replaces all existing).
    pub fn save_instruction_sources(&self, session_id: Uuid, sources: &[String]) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        let json = serde_json::to_string(sources)?;
        conn.execute(
            "UPDATE sessions SET instruction_sources = ?1 WHERE id = ?2",
            params![json, session_id.to_string()],
        )?;
        Ok(())
    }

    /// Append instruction sources for a session (deduplicates against existing items
    /// so the same source is never stored twice).
    pub fn append_instruction_sources(&self, session_id: Uuid, sources: &[String]) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        let current_json: Option<String> = conn
            .query_row(
                "SELECT instruction_sources FROM sessions WHERE id = ?1",
                params![session_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let mut existing: Vec<String> = current_json
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        for source in sources {
            if !existing.contains(source) {
                existing.push(source.clone());
            }
        }
        let json = serde_json::to_string(&existing)?;
        conn.execute(
            "UPDATE sessions SET instruction_sources = ?1 WHERE id = ?2",
            params![json, session_id.to_string()],
        )?;
        Ok(())
    }

    /// Load instruction sources for a session.
    pub fn load_instruction_sources(&self, session_id: Uuid) -> Result<Vec<String>> {
        self.read(|conn| {
            let mut stmt =
                conn.prepare("SELECT instruction_sources FROM sessions WHERE id = ?1")?;
            let mut rows = stmt.query_map(params![session_id.to_string()], |row| {
                row.get::<_, String>(0)
            })?;
            if let Some(Ok(json)) = rows.next() {
                let mut sources: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
                sources.sort();
                Ok(sources)
            } else {
                Ok(Vec::new())
            }
        })
    }
}
