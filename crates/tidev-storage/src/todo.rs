use super::*;

// ---------------------------------------------------------------------------
// Todo persistence
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Save/overwrite all todo items for a session.
    pub fn save_todos(
        &self,
        session_id: Uuid,
        todos: &[tidev_tools::types::TodoItem],
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        let json = serde_json::to_string(todos)?;
        conn.execute(
            "UPDATE sessions SET todos = ?1 WHERE id = ?2",
            params![json, session_id.to_string()],
        )?;
        Ok(())
    }

    /// Load todo items for a session.
    pub fn load_todos(&self, session_id: Uuid) -> Result<Vec<tidev_tools::types::TodoItem>> {
        self.read(|conn| {
            let mut stmt = conn.prepare("SELECT todos FROM sessions WHERE id = ?1")?;
            let mut rows = stmt.query_map(params![session_id.to_string()], |row| {
                row.get::<_, String>(0)
            })?;
            if let Some(Ok(json)) = rows.next() {
                let items: Vec<tidev_tools::types::TodoItem> =
                    serde_json::from_str(&json).unwrap_or_default();
                Ok(items)
            } else {
                Ok(Vec::new())
            }
        })
    }
}

// ===========================================================================
// End of SessionStore implementation.
// ===========================================================================
