use super::*;

// ---------------------------------------------------------------------------
// Session revert support
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Save revert state for a session.
    pub fn save_revert_state(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        redo_snapshot: Option<&[u8]>,
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        let msg_id_opt = if message_id.is_nil() {
            None
        } else {
            Some(message_id.to_string())
        };
        let snap_opt = redo_snapshot.map(|s| String::from_utf8_lossy(s).to_string());
        conn.execute(
            "UPDATE sessions SET revert_message_id = ?1, revert_redo_snapshot = ?2 WHERE id = ?3",
            params![msg_id_opt, snap_opt, session_id.to_string()],
        )?;
        Ok(())
    }

    /// Load revert state for a session.
    pub fn load_revert_state(&self, session_id: Uuid) -> Result<Option<(Uuid, Option<Vec<u8>>)>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT revert_message_id, revert_redo_snapshot FROM sessions WHERE id = ?1",
            )?;
            let mut rows = stmt.query_map(params![session_id.to_string()], |row| {
                let msg_id_opt: Option<String> = row.get(0)?;
                let snapshot_opt: Option<String> = row.get(1)?;
                Ok((msg_id_opt, snapshot_opt))
            })?;
            match rows.next() {
                Some(Ok((Some(msg_id_str), snap))) => {
                    let id = Uuid::parse_str(&msg_id_str).unwrap_or_default();
                    if id.is_nil() {
                        Ok(None)
                    } else {
                        Ok(Some((id, snap.map(|s| s.into_bytes()))))
                    }
                }
                _ => Ok(None),
            }
        })
    }
}
