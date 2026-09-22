use super::*;

// ---------------------------------------------------------------------------
// Message CRUD
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Insert a single message row (no session timestamp update).
    fn insert_message(conn: &Connection, session_id: Uuid, msg: &Message) -> Result<()> {
        Self::insert_message_with_app_data(conn, session_id, msg, &MessageAppData::default())
    }

    fn insert_message_with_app_data(
        conn: &Connection,
        session_id: Uuid,
        msg: &Message,
        app_data: &MessageAppData,
    ) -> Result<()> {
        let now = msg.created_at.to_rfc3339();
        let completed = msg.completed_at.map(|t| t.to_rfc3339());
        let mut combined_app_data = app_data.clone();
        if combined_app_data.reasoning_started_at.is_none() {
            combined_app_data.reasoning_started_at = msg.reasoning_started_at;
        }
        if combined_app_data.reasoning_completed_at.is_none() {
            combined_app_data.reasoning_completed_at = msg.reasoning_completed_at;
        }
        let content_blob = compress_text(&msg.content);
        let reasoning_blob = if msg.reasoning.is_empty() {
            None
        } else {
            Some(compress_text(&msg.reasoning))
        };
        let tool_calls_blob = if msg.tool_calls.is_empty() {
            Vec::new()
        } else {
            compress_text(
                &serde_json::to_string(&msg.tool_calls).unwrap_or_else(|_| "[]".to_string()),
            )
        };
        let metadata_blob = if msg.metadata == tidev_llm::message::ToolMetadata::default() {
            Vec::new()
        } else {
            compress_text(
                &serde_json::to_string(&msg.metadata).unwrap_or_else(|_| "{}".to_string()),
            )
        };
        let app_data_blob = if combined_app_data == MessageAppData::default() {
            Vec::new()
        } else {
            let app_data_json =
                serde_json::to_string(&combined_app_data).unwrap_or_else(|_| "{}".to_string());
            compress_text(&app_data_json)
        };

        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, attachments, reasoning, \
             tool_calls, tool_call_id, tool_name, metadata, created_at, completed_at, \
             streaming, input_tokens, output_tokens, total_tokens, cache_read_tokens, \
             cache_write_tokens, model_id, tokens_per_second, mode, thinking_level, \
             app_data) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
             ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
            params![
                msg.id.to_string(),
                session_id.to_string(),
                msg.role.db_value(),
                content_blob,
                serde_json::to_string(&msg.attachments).unwrap_or_else(|_| "[]".to_string()),
                reasoning_blob,
                tool_calls_blob,
                msg.tool_call_id.as_deref(),
                msg.tool_name.as_deref(),
                metadata_blob,
                now,
                completed,
                msg.streaming as i64,
                msg.input_tokens,
                msg.output_tokens,
                msg.total_tokens,
                msg.cache_read_tokens,
                msg.cache_write_tokens,
                msg.model_id,
                msg.tokens_per_second,
                app_data.mode.as_deref(),
                msg.thinking_level
                    .as_ref()
                    .map(|t| serde_json::to_string(t).unwrap_or_default()),
                app_data_blob,
            ],
        )?;
        Ok(())
    }

    /// Append multiple messages in a single transaction.
    ///
    /// More efficient than calling [`append_message`] in a loop because it
    /// acquires the write lock once and wraps all INSERTs + session timestamp
    /// update in one SQLite transaction.
    pub fn append_messages(&self, session_id: Uuid, messages: &[Message]) -> Result<()> {
        let mut conn = self.write_conn.lock().unwrap();
        let tx = conn.transaction()?;
        for msg in messages {
            Self::insert_message(&tx, session_id, msg)?;
        }
        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), session_id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Append protocol messages with their application-owned fields.
    pub fn append_messages_with_app_data(
        &self,
        session_id: Uuid,
        messages: &[Message],
        app_data: &HashMap<Uuid, MessageAppData>,
    ) -> Result<()> {
        let mut conn = self.write_conn.lock().unwrap();
        let tx = conn.transaction()?;
        for msg in messages {
            let fallback;
            let data = match app_data.get(&msg.id) {
                Some(data) => data,
                None => {
                    fallback = MessageAppData::default();
                    &fallback
                }
            };
            Self::insert_message_with_app_data(&tx, session_id, msg, data)?;
        }
        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), session_id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Persist protocol messages and newly applied instruction sources together.
    ///
    /// The message bytes and the source ledger form one durable boundary: a
    /// source is recorded only when the message carrying its instructions is
    /// committed successfully.
    pub fn append_messages_with_app_data_and_instruction_sources(
        &self,
        session_id: Uuid,
        messages: &[Message],
        app_data: &HashMap<Uuid, MessageAppData>,
        instruction_sources: &[String],
    ) -> Result<()> {
        let mut conn = self.write_conn.lock().unwrap();
        let tx = conn.transaction()?;
        for msg in messages {
            let fallback;
            let data = match app_data.get(&msg.id) {
                Some(data) => data,
                None => {
                    fallback = MessageAppData::default();
                    &fallback
                }
            };
            Self::insert_message_with_app_data(&tx, session_id, msg, data)?;
        }

        if !instruction_sources.is_empty() {
            let current_json: String = tx.query_row(
                "SELECT instruction_sources FROM sessions WHERE id = ?1",
                params![session_id.to_string()],
                |row| row.get(0),
            )?;
            let mut sources: Vec<String> = serde_json::from_str(&current_json).unwrap_or_default();
            for source in instruction_sources {
                if !sources.contains(source) {
                    sources.push(source.clone());
                }
            }
            tx.execute(
                "UPDATE sessions SET instruction_sources = ?1 WHERE id = ?2",
                params![serde_json::to_string(&sources)?, session_id.to_string()],
            )?;
        }

        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), session_id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Persist compaction state and its provider-visible summary message in
    /// the same transaction.
    pub fn apply_compaction(
        &self,
        session_id: Uuid,
        summary: &str,
        retained_from: usize,
        marker: &Message,
    ) -> Result<()> {
        let mut conn = self.write_conn.lock().unwrap();
        let tx = conn.transaction()?;
        Self::insert_message(&tx, session_id, marker)?;
        tx.execute(
            "UPDATE sessions SET context_summary = ?1, context_retained_from = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                compress_text(summary),
                retained_from as i64,
                Utc::now().to_rfc3339(),
                session_id.to_string(),
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Append a single message to a session.
    pub fn append_message(&self, session_id: Uuid, msg: &Message) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        Self::insert_message(&conn, session_id, msg)?;
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), session_id.to_string()],
        )?;
        Ok(())
    }

    /// Atomically replace a message while retaining its position in the
    /// conversation timeline. This is used to turn a durable streaming draft
    /// into its completed protocol message without a delete/append crash gap.
    pub fn replace_message_with_app_data(
        &self,
        session_id: Uuid,
        previous_id: Uuid,
        message: &Message,
        app_data: &MessageAppData,
    ) -> Result<()> {
        let mut conn = self.write_conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM messages WHERE session_id = ?1 AND id = ?2",
            params![session_id.to_string(), previous_id.to_string()],
        )?;
        Self::insert_message_with_app_data(&tx, session_id, message, app_data)?;
        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), session_id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Finalise an interrupted stream and append its durable notice in one
    /// transaction. Empty drafts are omitted, matching the live UI contract.
    pub fn finalize_interrupted_stream(
        &self,
        session_id: Uuid,
        draft: &Message,
        draft_app_data: &MessageAppData,
        notice: &Message,
        notice_app_data: &MessageAppData,
    ) -> Result<bool> {
        let keep_draft = !draft.content.is_empty()
            || !draft.reasoning.trim().is_empty()
            || !draft.tool_calls.is_empty();
        let mut conn = self.write_conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM messages WHERE session_id = ?1 AND id = ?2",
            params![session_id.to_string(), draft.id.to_string()],
        )?;
        if keep_draft {
            Self::insert_message_with_app_data(&tx, session_id, draft, draft_app_data)?;
        }
        Self::insert_message_with_app_data(&tx, session_id, notice, notice_app_data)?;
        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), session_id.to_string()],
        )?;
        tx.commit()?;
        Ok(keep_draft)
    }

    /// Mark unfinished assistant drafts for one session as terminal
    /// interruptions and return the application data written for each draft.
    ///
    /// Recovery is intentionally scoped to the session about to receive a new
    /// user message. A Runtime may inspect any session concurrently, so a
    /// startup-wide scan would mutate streams owned by another Runtime. The
    /// drafts remain `streaming` protocol messages so future LLM requests
    /// continue to exclude their partial content.
    pub fn recover_interrupted_streams(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<(Uuid, DateTime<Utc>, MessageAppData)>> {
        let mut conn = self.write_conn.lock().unwrap();
        let tx = conn.transaction()?;
        let rows = {
            let mut stmt = tx.prepare(
                "SELECT id, session_id, app_data FROM messages \
                 WHERE session_id = ?1 AND role = 'assistant' AND streaming = 1",
            )?;
            let mapped = stmt.query_map(params![session_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut recovered = Vec::new();
        for (message_id, session_id, blob) in rows {
            let mut app_data = if blob.is_empty() {
                MessageAppData::default()
            } else {
                serde_json::from_str(&decompress_text(&blob)).unwrap_or_default()
            };
            if app_data.interruption.is_some() {
                continue;
            }
            app_data.interruption = Some(InterruptionData {
                reason: InterruptionReason::RuntimeRestarted,
                request_id: 0,
                user_message_id: None,
            });
            let app_data_blob = compress_text(&serde_json::to_string(&app_data)?);
            let now = Utc::now();
            tx.execute(
                "UPDATE messages SET completed_at = COALESCE(completed_at, ?1), app_data = ?2 \
                 WHERE id = ?3 AND session_id = ?4",
                params![now.to_rfc3339(), app_data_blob, message_id, session_id],
            )?;
            recovered.push((Uuid::parse_str(&message_id)?, now, app_data));
        }
        if !recovered.is_empty() {
            tx.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                params![Utc::now().to_rfc3339(), session_id.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(recovered)
    }

    /// Remove legacy restart notices created by the former startup-wide
    /// recovery path. The cleanup is idempotent and leaves assistant drafts
    /// plus their interruption metadata intact.
    pub fn delete_legacy_restart_notices(&self) -> Result<usize> {
        let mut conn = self.write_conn.lock().unwrap();
        let tx = conn.transaction()?;
        let rows = {
            let mut stmt =
                tx.prepare("SELECT id, session_id, app_data FROM messages WHERE role = 'error'")?;
            let mapped = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut deleted = Vec::new();
        for (message_id, session_id, blob) in rows {
            let app_data = if blob.is_empty() {
                MessageAppData::default()
            } else {
                serde_json::from_str(&decompress_text(&blob)).unwrap_or_default()
            };
            if app_data
                .interruption
                .is_some_and(|data| data.reason == InterruptionReason::RuntimeRestarted)
            {
                tx.execute(
                    "DELETE FROM messages WHERE id = ?1 AND session_id = ?2",
                    params![message_id, session_id],
                )?;
                deleted.push(session_id);
            }
        }
        tx.commit()?;
        Ok(deleted.len())
    }

    /// Delete specific messages from a session.
    pub fn delete_messages(&self, session_id: Uuid, message_ids: &[Uuid]) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }
        for id in message_ids {
            self.write_execute(
                "DELETE FROM messages WHERE session_id = ?1 AND id = ?2",
                params![session_id.to_string(), id.to_string()],
            )?;
        }
        // Update session timestamp
        let now = Utc::now().to_rfc3339();
        self.write_execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id.to_string()],
        )?;
        Ok(())
    }

    /// Load all messages for a session, ordered by creation time.
    ///
    /// **Phase 1** — collect raw column data from SQLite (fast, serial).
    /// **Phase 2** — decompress zstd blobs and parse JSON in parallel via
    /// rayon, utilising all available CPU cores for these CPU-bound steps.
    pub fn load_messages(&self, session_id: Uuid) -> Result<Vec<Message>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, role, content, attachments, reasoning, tool_calls, tool_call_id, \
                 tool_name, metadata, created_at, completed_at, streaming, input_tokens, \
                 output_tokens, total_tokens, cache_read_tokens, cache_write_tokens, model_id, \
                 tokens_per_second, thinking_level, app_data \
                 FROM messages WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
            )?;

            // ── Phase 1: collect raw rows (no decompression) ──────────
            let raw_rows: Vec<RawMessageRow> = {
                let rows = stmt.query_map(params![session_id.to_string()], |row| {
                    Ok(RawMessageRow {
                        id: row.get::<_, String>(0).unwrap_or_default(),
                        role: row.get::<_, String>(1).unwrap_or_default(),
                        content: row.get::<_, Vec<u8>>(2).unwrap_or_default(),
                        attachments: row.get::<_, String>(3).unwrap_or_default(),
                        reasoning: row.get::<_, Vec<u8>>(4).unwrap_or_default(),
                        tool_calls: row.get::<_, Vec<u8>>(5).unwrap_or_default(),
                        tool_call_id: row.get(6).ok().flatten(),
                        tool_name: row.get(7).ok().flatten(),
                        metadata: row.get::<_, Vec<u8>>(8).unwrap_or_default(),
                        created_at: row.get::<_, String>(9).unwrap_or_default(),
                        completed_at: row.get(10).ok().flatten(),
                        streaming: row.get::<_, i64>(11).unwrap_or(0) != 0,
                        input_tokens: row.get(12).ok().flatten(),
                        output_tokens: row.get(13).ok().flatten(),
                        total_tokens: row.get(14).ok().flatten(),
                        cache_read_tokens: row.get(15).ok().flatten(),
                        cache_write_tokens: row.get(16).ok().flatten(),
                        model_id: row.get(17).ok().flatten(),
                        tokens_per_second: row.get(18).ok().flatten(),
                        thinking_level: row.get(19).ok().flatten(),
                        app_data: row.get::<_, Vec<u8>>(20).unwrap_or_default(),
                    })
                })?;
                let mut raw = Vec::new();
                for row in rows {
                    raw.push(row?);
                }
                raw
            };

            // ── Phase 2: parallel decompress and parse ───────────────
            let messages: Vec<Message> = raw_rows
                .into_par_iter()
                .map(|raw| raw.decompress_and_parse())
                .collect();

            Ok(messages)
        })
    }

    /// Load application-owned fields for all messages in a session.
    pub fn load_message_app_data(&self, session_id: Uuid) -> Result<HashMap<Uuid, MessageAppData>> {
        self.read(|conn| {
            let mut stmt =
                conn.prepare("SELECT id, mode, app_data FROM messages WHERE session_id = ?1")?;
            let rows = stmt.query_map(params![session_id.to_string()], |row| {
                let id = Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default();
                let mode: Option<String> = row.get(1)?;
                let app_data_blob: Vec<u8> = row.get(2).unwrap_or_default();
                let mut app_data: MessageAppData = if app_data_blob.is_empty() {
                    MessageAppData::default()
                } else {
                    serde_json::from_str(&decompress_text(&app_data_blob)).unwrap_or_default()
                };
                if mode.is_some() {
                    app_data.mode = mode;
                }
                Ok((id, app_data))
            })?;
            let mut app_data = HashMap::new();
            for row in rows {
                let (id, data) = row?;
                app_data.insert(id, data);
            }
            Ok(app_data)
        })
    }

    /// Update a message's content (used for streaming).
    pub fn update_message_content(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        content: &str,
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET content = ?1 WHERE id = ?2 AND session_id = ?3",
            params![
                compress_text(content),
                message_id.to_string(),
                session_id.to_string()
            ],
        )?;
        Ok(())
    }

    /// Update a message's tool calls (used for streaming updates).
    pub fn update_message_tool_calls(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        tool_calls: &[tidev_llm::message::ToolCall],
    ) -> Result<()> {
        let blob = if tool_calls.is_empty() {
            Vec::new()
        } else {
            let json = serde_json::to_string(tool_calls)?;
            compress_text(&json)
        };
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET tool_calls = ?1 WHERE id = ?2 AND session_id = ?3",
            params![blob, message_id.to_string(), session_id.to_string()],
        )?;
        Ok(())
    }

    /// Update a message's metadata (used by subagent tracking and tool result enrichment).
    pub fn update_message_metadata(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        metadata: &tidev_llm::message::ToolMetadata,
    ) -> Result<()> {
        let blob = if metadata == &tidev_llm::message::ToolMetadata::default() {
            Vec::new()
        } else {
            let json = serde_json::to_string(metadata)?;
            compress_text(&json)
        };
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET metadata = ?1 WHERE id = ?2 AND session_id = ?3",
            params![blob, message_id.to_string(), session_id.to_string()],
        )?;
        Ok(())
    }

    /// Update a message's child-session association without touching protocol metadata.
    pub fn update_message_child_session_id(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        child_session_id: Uuid,
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
        app_data.child_session_id = Some(child_session_id);
        let blob = if app_data == MessageAppData::default() {
            Vec::new()
        } else {
            let json = serde_json::to_string(&app_data).unwrap_or_else(|_| "{}".to_string());
            compress_text(&json)
        };
        conn.execute(
            "UPDATE messages SET app_data = ?1 WHERE id = ?2 AND session_id = ?3",
            params![blob, message_id.to_string(), session_id.to_string()],
        )?;
        Ok(())
    }

    /// Update message completion status.
    #[allow(clippy::too_many_arguments)]
    pub fn update_message_completed(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        completed_at: DateTime<Utc>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        total_tokens: Option<u32>,
        cache_read_tokens: Option<u32>,
        cache_write_tokens: Option<u32>,
        model_id: Option<String>,
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET completed_at = ?1, input_tokens = ?2, output_tokens = ?3, \
             total_tokens = ?4, cache_read_tokens = ?5, cache_write_tokens = ?6, model_id = ?7 \
             WHERE id = ?8 AND session_id = ?9",
            params![
                completed_at.to_rfc3339(),
                input_tokens,
                output_tokens,
                total_tokens,
                cache_read_tokens,
                cache_write_tokens,
                model_id,
                message_id.to_string(),
                session_id.to_string(),
            ],
        )?;
        Ok(())
    }

    /// Mark a message as no longer streaming.
    pub fn finish_streaming(&self, session_id: Uuid, message_id: Uuid) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET streaming = 0 WHERE id = ?1 AND session_id = ?2",
            params![message_id.to_string(), session_id.to_string()],
        )?;
        Ok(())
    }

    /// Get token statistics for a session.
    pub fn get_session_token_stats(&self, session_id: Uuid) -> Result<SessionTokenStats> {
        self.read(|conn| {
            conn.query_row(
                "SELECT COALESCE(SUM(input_tokens), 0) as input_tokens, \
                 COALESCE(SUM(output_tokens), 0) as output_tokens \
                 FROM messages WHERE session_id = :session_id",
                named_params! { ":session_id": session_id.to_string() },
                |row| {
                    Ok(SessionTokenStats {
                        input_tokens: row.get::<_, i64>(0)? as u32,
                        output_tokens: row.get::<_, i64>(1)? as u32,
                    })
                },
            )
            .map_err(Into::into)
        })
    }

    /// Load token usage records without loading message content or metadata.
    pub fn load_usage_records(&self) -> Result<Vec<UsageRecord>> {
        self.load_usage_records_in_range(None, None)
    }

    /// Load token usage records in an optional half-open UTC time range.
    ///
    /// Applying the range in SQLite avoids materializing unrelated message
    /// metadata before the web layer aggregates statistics.
    pub fn load_usage_records_in_range(
        &self,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Result<Vec<UsageRecord>> {
        self.read(|conn| {
            let mut conditions = vec![String::from(
                "m.role = 'assistant' AND (m.total_tokens IS NOT NULL OR m.input_tokens IS NOT NULL OR m.output_tokens IS NOT NULL)",
            )];
            let mut values: Vec<Box<dyn ToSql>> = Vec::new();
            if let Some(start) = start {
                conditions.push("m.created_at >= ?".to_owned());
                values.push(Box::new(start.to_rfc3339()));
            }
            if let Some(end) = end {
                conditions.push("m.created_at < ?".to_owned());
                values.push(Box::new(end.to_rfc3339()));
            }
            let sql = format!(
                "SELECT m.session_id, s.title, s.provider_id, s.provider_display_name, \
                 s.model_id, s.model_display_name, s.created_at, s.updated_at, m.created_at, \
                 COALESCE(m.input_tokens, 0), COALESCE(m.output_tokens, 0), \
                 COALESCE(m.cache_read_tokens, 0), COALESCE(m.cache_write_tokens, 0), \
                 COALESCE(m.total_tokens, COALESCE(m.input_tokens, 0) + COALESCE(m.output_tokens, 0)) \
                 FROM messages m INNER JOIN sessions s ON s.id = m.session_id \
                 WHERE {} ORDER BY m.created_at ASC, m.rowid ASC",
                conditions.join(" AND ")
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params_from_iter(values.iter().map(|value| value.as_ref())),
                |row| -> rusqlite::Result<UsageRecord> {
                    Ok(UsageRecord {
                        session_id: row.get(0)?,
                        title: row.get(1)?,
                        provider_id: row.get(2)?,
                        provider_display_name: row.get(3)?,
                        model_id: row.get(4)?,
                        model_display_name: row.get(5)?,
                        session_created_at: row.get(6)?,
                        session_updated_at: row.get(7)?,
                        created_at: row.get(8)?,
                        input_tokens: row.get::<_, i64>(9)?.max(0) as u64,
                        output_tokens: row.get::<_, i64>(10)?.max(0) as u64,
                        cache_read_tokens: row.get::<_, i64>(11)?.max(0) as u64,
                        cache_write_tokens: row.get::<_, i64>(12)?.max(0) as u64,
                        total_tokens: row.get::<_, i64>(13)?.max(0) as u64,
                    })
                },
            )?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Aggregate daily activity directly from assistant messages with usage.
    ///
    /// The caller supplies a half-open UTC range. SQLite performs filtering
    /// and grouping so the response remains bounded by the requested days.
    pub fn load_usage_activity_days(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<UsageActivityDay>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT substr(m.created_at, 1, 10) AS activity_date, \
                 COUNT(*) AS request_count, \
                 COALESCE(SUM(COALESCE(m.total_tokens, COALESCE(m.input_tokens, 0) + COALESCE(m.output_tokens, 0))), 0) AS total_tokens \
                 FROM messages m \
                 WHERE m.role = 'assistant' \
                   AND (m.total_tokens IS NOT NULL OR m.input_tokens IS NOT NULL OR m.output_tokens IS NOT NULL) \
                   AND m.created_at >= ?1 \
                   AND m.created_at < ?2 \
                 GROUP BY activity_date \
                 ORDER BY activity_date ASC",
            )?;
            let rows = stmt.query_map(params![start.to_rfc3339(), end.to_rfc3339()], |row| {
                Ok(UsageActivityDay {
                    date: row.get(0)?,
                    request_count: row.get::<_, i64>(1)?.max(0) as u64,
                    total_tokens: row.get::<_, i64>(2)?.max(0) as u64,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Count distinct sessions with token-bearing assistant responses per UTC bucket.
    pub fn load_usage_active_sessions(
        &self,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        granularity: &str,
    ) -> Result<Vec<UsageActiveSessionBucket>> {
        self.read(|conn| {
            let bucket = usage_time_bucket_expression(granularity, "m.created_at");
            let (range_filter, values) = usage_range_filter(start, end);
            let sql = format!(
                "SELECT {bucket} AS time_bucket, COUNT(DISTINCT m.session_id) AS active_sessions \
                 FROM messages m \
                 WHERE {USAGE_MESSAGE_FILTER}{range_filter} \
                 GROUP BY time_bucket \
                 ORDER BY time_bucket ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params_from_iter(values.iter().map(|value| value.as_ref())),
                |row| {
                    Ok(UsageActiveSessionBucket {
                        time_bucket: row.get(0)?,
                        active_sessions: row.get::<_, i64>(1)?.max(0) as u64,
                    })
                },
            )?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Aggregate response activity by weekday and hour in UTC.
    pub fn load_usage_rhythm(
        &self,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Result<Vec<UsageRhythmCell>> {
        self.read(|conn| {
            let (range_filter, values) = usage_range_filter(start, end);
            let sql = format!(
                "SELECT \
                   ((CAST(strftime('%w', m.created_at) AS INTEGER) + 6) % 7) AS weekday, \
                   CAST(strftime('%H', m.created_at) AS INTEGER) AS hour, \
                   COUNT(*) AS request_count, \
                   COALESCE(SUM(COALESCE(m.total_tokens, COALESCE(m.input_tokens, 0) + COALESCE(m.output_tokens, 0))), 0) AS total_tokens \
                 FROM messages m \
                 WHERE {USAGE_MESSAGE_FILTER}{range_filter} \
                 GROUP BY weekday, hour \
                 ORDER BY weekday ASC, hour ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params_from_iter(values.iter().map(|value| value.as_ref())),
                |row| {
                    Ok(UsageRhythmCell {
                        weekday: row.get::<_, i64>(0)?.clamp(0, 6) as u8,
                        hour: row.get::<_, i64>(1)?.clamp(0, 23) as u8,
                        request_count: row.get::<_, i64>(2)?.max(0) as u64,
                        total_tokens: row.get::<_, i64>(3)?.max(0) as u64,
                    })
                },
            )?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Aggregate the top models and one combined remainder series per UTC bucket.
    pub fn load_usage_model_mix(
        &self,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        granularity: &str,
        model_limit: u64,
    ) -> Result<Vec<UsageModelMixBucket>> {
        self.read(|conn| {
            let bucket = usage_time_bucket_expression(granularity, "usage.created_at");
            let (range_filter, mut values) = usage_range_filter(start, end);
            values.push(Box::new(model_limit.clamp(1, 10) as i64));
            let sql = format!(
                "WITH usage AS ( \
                   SELECT \
                     m.created_at, \
                     COALESCE(NULLIF(s.provider_id, ''), 'unknown') AS provider_id, \
                     COALESCE(NULLIF(s.provider_display_name, ''), NULLIF(s.provider_id, ''), 'Unknown') AS provider_display_name, \
                     COALESCE(NULLIF(m.model_id, ''), NULLIF(s.model_id, ''), 'unknown') AS model_id, \
                     COALESCE(NULLIF(s.model_display_name, ''), NULLIF(m.model_id, ''), NULLIF(s.model_id, ''), 'Unknown') AS model_display_name, \
                     COALESCE(m.total_tokens, COALESCE(m.input_tokens, 0) + COALESCE(m.output_tokens, 0)) AS total_tokens \
                   FROM messages m \
                   INNER JOIN sessions s ON s.id = m.session_id \
                   WHERE {USAGE_MESSAGE_FILTER}{range_filter} \
                 ), \
                 top_models AS ( \
                   SELECT provider_id, model_id \
                   FROM usage \
                   GROUP BY provider_id, model_id \
                   ORDER BY SUM(total_tokens) DESC, provider_id ASC, model_id ASC \
                   LIMIT ? \
                 ), \
                 bucketed AS ( \
                   SELECT \
                     {bucket} AS time_bucket, \
                     CASE WHEN top_models.model_id IS NULL THEN 'other' ELSE usage.provider_id END AS provider_id, \
                     CASE WHEN top_models.model_id IS NULL THEN 'Other' ELSE usage.provider_display_name END AS provider_display_name, \
                     CASE WHEN top_models.model_id IS NULL THEN 'other' ELSE usage.model_id END AS model_id, \
                     CASE WHEN top_models.model_id IS NULL THEN 'Other' ELSE usage.model_display_name END AS model_display_name, \
                     CASE WHEN top_models.model_id IS NULL THEN 1 ELSE 0 END AS is_other, \
                     usage.total_tokens \
                   FROM usage \
                   LEFT JOIN top_models \
                     ON top_models.provider_id = usage.provider_id \
                     AND top_models.model_id = usage.model_id \
                 ) \
                 SELECT \
                   time_bucket, provider_id, MIN(provider_display_name), model_id, MIN(model_display_name), is_other, \
                   COALESCE(SUM(total_tokens), 0) AS total_tokens \
                 FROM bucketed \
                 GROUP BY time_bucket, provider_id, model_id, is_other \
                 ORDER BY time_bucket ASC, is_other ASC, total_tokens DESC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params_from_iter(values.iter().map(|value| value.as_ref())),
                |row| {
                    Ok(UsageModelMixBucket {
                        time_bucket: row.get(0)?,
                        provider_id: row.get(1)?,
                        provider_display_name: row.get(2)?,
                        model_id: row.get(3)?,
                        model_display_name: row.get(4)?,
                        is_other: row.get::<_, i64>(5)? != 0,
                        total_tokens: row.get::<_, i64>(6)?.max(0) as u64,
                    })
                },
            )?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Aggregate response counts into a fixed set of total-token size ranges.
    pub fn load_usage_request_size_distribution(
        &self,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Result<Vec<UsageRequestSizeBucket>> {
        self.read(|conn| {
            let (range_filter, values) = usage_range_filter(start, end);
            let total_tokens = "COALESCE(m.total_tokens, COALESCE(m.input_tokens, 0) + COALESCE(m.output_tokens, 0))";
            let sql = format!(
                "SELECT \
                   CASE \
                     WHEN {total_tokens} < 2048 THEN 0 \
                     WHEN {total_tokens} < 8192 THEN 1 \
                     WHEN {total_tokens} < 32768 THEN 2 \
                     WHEN {total_tokens} < 131072 THEN 3 \
                     ELSE 4 \
                   END AS bucket_index, \
                   COUNT(*) AS request_count, \
                   COALESCE(SUM({total_tokens}), 0) AS total_tokens \
                 FROM messages m \
                 WHERE {USAGE_MESSAGE_FILTER}{range_filter} \
                 GROUP BY bucket_index \
                 ORDER BY bucket_index ASC"
            );
            let mut buckets = USAGE_REQUEST_SIZE_RANGES
                .iter()
                .map(|(lower_bound, upper_bound)| UsageRequestSizeBucket {
                    lower_bound: *lower_bound,
                    upper_bound: *upper_bound,
                    request_count: 0,
                    total_tokens: 0,
                })
                .collect::<Vec<_>>();
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params_from_iter(values.iter().map(|value| value.as_ref())),
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?.max(0) as u64,
                        row.get::<_, i64>(2)?.max(0) as u64,
                    ))
                },
            )?;
            for row in rows {
                let (bucket_index, request_count, total_tokens) = row?;
                if let Some(bucket) = buckets.get_mut(bucket_index.max(0) as usize) {
                    bucket.request_count = request_count;
                    bucket.total_tokens = total_tokens;
                }
            }
            Ok(buckets)
        })
    }

    /// Get the most recent message for a session.
    pub fn get_last_message(&self, session_id: Uuid) -> Result<Option<Message>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, role, content, attachments, reasoning, tool_calls, tool_call_id, \
                 tool_name, metadata, created_at, completed_at, streaming, input_tokens, \
                 output_tokens, total_tokens, cache_read_tokens, cache_write_tokens, model_id, \
                 tokens_per_second, thinking_level, app_data \
                 FROM messages WHERE session_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
            )?;
            let mut rows = stmt.query_map(params![session_id.to_string()], |row| {
                Ok(Self::build_message_from_row(row))
            })?;
            match rows.next() {
                Some(Ok(msg)) => Ok(Some(msg)),
                _ => Ok(None),
            }
        })
    }

    /// Build a Message from a SQLite row (used by load_messages and get_last_message).
    fn build_message_from_row(row: &rusqlite::Row) -> Message {
        let role_str: String = row.get(1).unwrap_or_default();
        let content_blob: Vec<u8> = row.get(2).unwrap_or_default();
        let content = if content_blob.is_empty() {
            row.get::<_, String>(2).unwrap_or_default()
        } else {
            decompress_text(&content_blob)
        };
        let attachments_raw: String = row.get(3).unwrap_or_default();
        let attachments: Vec<tidev_llm::message::MessageAttachment> =
            serde_json::from_str(&attachments_raw).unwrap_or_default();
        let metadata_raw: Vec<u8> = row.get(8).unwrap_or_default();
        let metadata: tidev_llm::message::ToolMetadata = if metadata_raw.is_empty() {
            tidev_llm::message::ToolMetadata::default()
        } else {
            serde_json::from_str(&decompress_text(&metadata_raw)).unwrap_or_default()
        };
        let completed_at: Option<String> = row.get(10).ok().flatten();
        let streaming: bool = row.get::<_, i64>(11).unwrap_or(0) != 0;

        let reasoning = row
            .get::<_, Vec<u8>>(4)
            .map(|b| {
                if b.is_empty() {
                    String::new()
                } else {
                    decompress_text(&b)
                }
            })
            .unwrap_or_default();

        let tool_calls_blob: Vec<u8> = row.get(5).unwrap_or_default();
        let tool_calls: Vec<tidev_llm::message::ToolCall> = if tool_calls_blob.is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&decompress_text(&tool_calls_blob)).unwrap_or_default()
        };

        let thinking_level: Option<String> = row.get(19).ok().flatten();
        let app_data_blob: Vec<u8> = row.get(20).unwrap_or_default();
        let app_data: MessageAppData = if app_data_blob.is_empty() {
            MessageAppData::default()
        } else {
            serde_json::from_str(&decompress_text(&app_data_blob)).unwrap_or_default()
        };

        Message {
            id: Uuid::parse_str(&row.get::<_, String>(0).unwrap_or_default()).unwrap_or_default(),
            role: MessageRole::from_db_value(&role_str),
            content,
            attachments,
            reasoning,
            tool_calls,
            tool_call_id: row.get(6).ok().flatten(),
            tool_name: row.get(7).ok().flatten(),
            metadata,
            created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(9).unwrap_or_default())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_default(),
            completed_at: completed_at.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            streaming,
            input_tokens: row.get(12).ok().flatten(),
            output_tokens: row.get(13).ok().flatten(),
            total_tokens: row.get(14).ok().flatten(),
            cache_read_tokens: row.get(15).ok().flatten(),
            cache_write_tokens: row.get(16).ok().flatten(),
            model_id: row.get(17).ok().flatten(),
            tokens_per_second: row.get(18).ok().flatten(),
            thinking_level: thinking_level.and_then(|t| serde_json::from_str(&t).ok()),
            reasoning_started_at: app_data.reasoning_started_at,
            reasoning_completed_at: app_data.reasoning_completed_at,
        }
    }

    /// Export one or more sessions to an uncompressed SQLite database.
    ///
    /// The output database uses the [`EXPORT_SCHEMA_SQL`] schema where all
    /// text columns are stored as plain TEXT (no zstd compression), suitable
    /// for inspection with standard SQLite tools or re-import.
    pub fn export_to_sqlite(&self, session_ids: &[Uuid], output_path: &Path) -> Result<()> {
        // Remove existing file so we start fresh.
        let _ = fs::remove_file(output_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create export directory {}", parent.display())
            })?;
        }

        let export_conn = Connection::open(output_path).with_context(|| {
            format!("failed to create export database {}", output_path.display())
        })?;
        export_conn.execute_batch("PRAGMA foreign_keys = OFF")?;
        export_conn.execute_batch(crate::schema::EXPORT_SCHEMA_SQL)?;

        let sid_strs: Vec<String> = session_ids.iter().map(|id| id.to_string()).collect();
        let placeholder = sid_strs.iter().map(|_| "?").collect::<Vec<_>>().join(",");

        let tx = export_conn.unchecked_transaction()?;

        // ── 1. meta ── copy all rows ─────────────────────────────────────
        {
            let rows = self.read_query("SELECT key, value FROM meta", [], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut insert =
                tx.prepare("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)")?;
            for (key, value) in &rows {
                insert.execute(params![key, value])?;
            }
        }

        // ── 2. sessions ── only the requested ones ───────────────────────
        {
            let sql = format!(
                "SELECT id, parent_session_id, provider_id, provider_display_name, \
                        model_id, model_display_name, title, created_at, updated_at, \
                        status, ended_at, CAST(context_summary AS BLOB), context_retained_from, \
                        CAST(system_prompt AS BLOB), workspace_root, snapshot_start_hash, \
                        instruction_sources, todos, revert_message_id, revert_redo_snapshot \
                 FROM sessions WHERE id IN ({placeholder})"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = sid_strs
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = self.read_query(&sql, params.as_slice(), |row| {
                let parent: Option<String> = row.get(1)?;
                let context_summary = blob_or_empty_to_text(row, 11)?;
                let system_prompt = blob_or_empty_to_text(row, 13)?;
                Ok((
                    row.get::<_, String>(0)?,
                    parent,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    context_summary,
                    row.get::<_, i64>(12)?,
                    system_prompt,
                    row.get::<_, String>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, String>(16)?,
                    row.get::<_, String>(17)?,
                    row.get::<_, Option<String>>(18)?,
                    row.get::<_, Option<String>>(19)?,
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO sessions \
                 (id, parent_session_id, provider_id, provider_display_name, \
                  model_id, model_display_name, title, created_at, updated_at, \
                  status, ended_at, context_summary, context_retained_from, system_prompt, \
                  workspace_root, snapshot_start_hash, instruction_sources, todos, \
                  revert_message_id, revert_redo_snapshot) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            )?;
            for row in &rows {
                insert.execute(params![
                    row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8, row.9, row.10,
                    row.11, row.12, row.13, row.14, row.15, row.16, row.17, row.18, row.19,
                ])?;
            }
        }

        // ── 3. messages ── decompress BLOB columns → TEXT ────────────────
        {
            let sql = format!(
                "SELECT id, session_id, role, CAST(content AS BLOB), attachments, \
                        CAST(reasoning AS BLOB), CAST(tool_calls AS BLOB), tool_call_id, \
                        tool_name, CAST(metadata AS BLOB), created_at, completed_at, \
                        streaming, input_tokens, output_tokens, total_tokens, \
                        cache_read_tokens, cache_write_tokens, model_id, tokens_per_second, \
                        mode, thinking_level, CAST(app_data AS BLOB) \
                 FROM messages WHERE session_id IN ({placeholder}) ORDER BY created_at ASC, rowid ASC"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = sid_strs
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = self.read_query(&sql, params.as_slice(), |row| {
                let content = blob_or_empty_to_text(row, 3)?;
                let reasoning = opt_blob_to_text(row, 5)?;
                let tool_calls = blob_or_empty_to_text(row, 6)?;
                let metadata = blob_or_empty_to_text(row, 9)?;
                let app_data = blob_or_empty_to_text(row, 22)?;
                Ok((
                    row.get::<_, String>(0)?, // id
                    row.get::<_, String>(1)?, // session_id
                    row.get::<_, String>(2)?, // role
                    content,
                    row.get::<_, String>(4)?, // attachments
                    reasoning,
                    tool_calls,
                    row.get::<_, Option<String>>(7)?, // tool_call_id
                    row.get::<_, Option<String>>(8)?, // tool_name
                    metadata,
                    row.get::<_, String>(10)?,         // created_at
                    row.get::<_, Option<String>>(11)?, // completed_at
                    row.get::<_, i64>(12)?,            // streaming
                    row.get::<_, Option<i64>>(13)?,
                    row.get::<_, Option<i64>>(14)?,
                    row.get::<_, Option<i64>>(15)?,
                    row.get::<_, Option<i64>>(16)?,
                    row.get::<_, Option<i64>>(17)?,
                    row.get::<_, Option<String>>(18)?,
                    row.get::<_, Option<f64>>(19)?,
                    row.get::<_, Option<String>>(20)?, // mode
                    row.get::<_, Option<String>>(21)?, // thinking_level
                    app_data,
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO messages \
                 (id, session_id, role, content, attachments, reasoning, tool_calls, \
                  tool_call_id, tool_name, metadata, created_at, completed_at, streaming, \
                  input_tokens, output_tokens, total_tokens, cache_read_tokens, \
                  cache_write_tokens, model_id, tokens_per_second, mode, thinking_level, app_data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, \
                         ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
            )?;
            for row in &rows {
                insert.execute(params![
                    row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8, row.9, row.10,
                    row.11, row.12, row.13, row.14, row.15, row.16, row.17, row.18, row.19, row.20,
                    row.21, row.22,
                ])?;
            }
        }

        // ── 4. tool_outputs ── decompress BLOB columns → TEXT ────────────
        {
            let sql = format!(
                "SELECT id, session_id, message_id, tool_call_id, tool_name, CAST(output AS BLOB), \
                        byte_size, line_count, created_at \
                 FROM tool_outputs WHERE session_id IN ({placeholder}) ORDER BY created_at ASC, rowid ASC"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = sid_strs
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = self.read_query(&sql, params.as_slice(), |row| {
                let output = opt_blob_to_text(row, 5)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    output,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO tool_outputs \
                 (id, session_id, message_id, tool_call_id, tool_name, output, byte_size, line_count, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for row in &rows {
                insert.execute(params![
                    row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8,
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Import sessions from an uncompressed SQLite file created by
    /// [`export_to_sqlite`].
    ///
    /// Returns the list of session UUIDs that were imported.
    ///
    /// * `import_path` — path to the export SQLite file.
    /// * `session_ids` — if `Some`, only import these sessions from the file;
    ///   if `None`, import all sessions found.
    /// * `replace` — if `true`, overwrite existing sessions with the same UUID;
    ///   if `false`, skip sessions that already exist.
    pub fn import_from_sqlite(
        &self,
        import_path: &Path,
        session_ids: Option<&[Uuid]>,
        replace: bool,
    ) -> Result<Vec<Uuid>> {
        let import_conn = Connection::open(import_path)
            .with_context(|| format!("failed to open import file {}", import_path.display()))?;

        // Determine which sessions to import.
        let sid_strs: Vec<String> = if let Some(ids) = session_ids {
            ids.iter().map(|id| id.to_string()).collect()
        } else {
            import_conn
                .prepare("SELECT id FROM sessions")?
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        if sid_strs.is_empty() {
            return Ok(Vec::new());
        }

        let placeholder = sid_strs.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sid_params: Vec<&dyn rusqlite::types::ToSql> = sid_strs
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        // Filter out sessions that already exist (unless replace).
        let existing: Vec<String> = if replace {
            Vec::new()
        } else {
            let sql = format!("SELECT id FROM sessions WHERE id IN ({placeholder})");
            self.read_query(&sql, sid_params.as_slice(), |row| row.get::<_, String>(0))?
        };

        let to_import: Vec<&str> = sid_strs
            .iter()
            .map(|s| s.as_str())
            .filter(|s| !existing.contains(&s.to_string()))
            .collect();

        if to_import.is_empty() {
            return Ok(Vec::new());
        }

        let imp_placeholder = to_import.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let imp_params: Vec<&dyn rusqlite::types::ToSql> = to_import
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        // ── 1. sessions ───────────────────────────────────────────────────
        {
            let sql = format!(
                "SELECT id, parent_session_id, provider_id, provider_display_name, \
                        model_id, model_display_name, title, created_at, updated_at, \
                        status, ended_at, context_summary, context_retained_from, system_prompt, \
                        workspace_root, snapshot_start_hash, instruction_sources, todos, \
                        revert_message_id, revert_redo_snapshot \
                 FROM sessions WHERE id IN ({imp_placeholder})"
            );
            let mut stmt = import_conn.prepare(&sql)?;
            let rows = stmt
                .query_map(imp_params.as_slice(), |row| {
                    let context_summary: String = row.get(11)?;
                    let system_prompt: String = row.get(13)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        compress_text(&context_summary),
                        row.get::<_, i64>(12)?,
                        compress_text(&system_prompt),
                        row.get::<_, String>(14)?,
                        row.get::<_, Option<String>>(15)?,
                        row.get::<_, String>(16)?,
                        row.get::<_, String>(17)?,
                        row.get::<_, Option<String>>(18)?,
                        row.get::<_, Option<String>>(19)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let conn = self.write_conn.lock().unwrap();
            for r in &rows {
                conn.execute(
                    "INSERT OR REPLACE INTO sessions \
                     (id, parent_session_id, provider_id, provider_display_name, model_id, \
                      model_display_name, title, created_at, updated_at, status, ended_at, \
                      context_summary, context_retained_from, system_prompt, \
                      workspace_root, snapshot_start_hash, instruction_sources, todos, \
                      revert_message_id, revert_redo_snapshot) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                    params![
                        r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8, r.9, r.10, r.11, r.12, r.13,
                        r.14, r.15, r.16, r.17, r.18, r.19,
                    ],
                )?;
            }
        }

        // ── 2. messages ── compress TEXT columns → BLOB ───────────────────
        {
            let sql = format!(
                "SELECT id, session_id, role, content, attachments, reasoning, tool_calls, \
                        tool_call_id, tool_name, metadata, created_at, completed_at, streaming, \
                        input_tokens, output_tokens, total_tokens, cache_read_tokens, \
                        cache_write_tokens, model_id, tokens_per_second, mode, \
                        thinking_level, app_data \
                 FROM messages WHERE session_id IN ({imp_placeholder}) ORDER BY created_at ASC, rowid ASC"
            );
            let mut stmt = import_conn.prepare(&sql)?;
            let rows = stmt
                .query_map(imp_params.as_slice(), |row| {
                    let content: String = row.get(3)?;
                    let reasoning: Option<String> = row.get(5)?;
                    let tool_calls: String = row.get(6)?;
                    let metadata: String = row.get(9)?;
                    let app_data: String = row.get(22).unwrap_or_else(|_| "{}".to_string());
                    let tool_calls_blob = if tool_calls.trim() == "[]" || tool_calls.is_empty() {
                        Vec::new()
                    } else {
                        compress_text(&tool_calls)
                    };
                    let metadata_blob = if metadata.trim() == "{}" || metadata.is_empty() {
                        Vec::new()
                    } else {
                        compress_text(&metadata)
                    };
                    let app_data_blob = if app_data.trim() == "{}" || app_data.is_empty() {
                        Vec::new()
                    } else {
                        compress_text(&app_data)
                    };
                    let reasoning_blob = match reasoning {
                        Some(r) if !r.is_empty() => Some(compress_text(&r)),
                        _ => None,
                    };
                    Ok((
                        row.get::<_, String>(0)?, // id
                        row.get::<_, String>(1)?, // session_id
                        row.get::<_, String>(2)?, // role
                        compress_text(&content),
                        row.get::<_, String>(4)?, // attachments (JSON, stored as TEXT in both)
                        reasoning_blob,
                        tool_calls_blob,
                        row.get::<_, Option<String>>(7)?, // tool_call_id
                        row.get::<_, Option<String>>(8)?, // tool_name
                        metadata_blob,
                        row.get::<_, String>(10)?,         // created_at
                        row.get::<_, Option<String>>(11)?, // completed_at
                        row.get::<_, i64>(12)?,            // streaming
                        row.get::<_, Option<i64>>(13)?,
                        row.get::<_, Option<i64>>(14)?,
                        row.get::<_, Option<i64>>(15)?,
                        row.get::<_, Option<i64>>(16)?,
                        row.get::<_, Option<i64>>(17)?,
                        row.get::<_, Option<String>>(18)?, // model_id
                        row.get::<_, Option<f64>>(19)?,    // tokens_per_second
                        row.get::<_, Option<String>>(20)?, // mode
                        row.get::<_, Option<String>>(21)?, // thinking_level
                        app_data_blob,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let conn = self.write_conn.lock().unwrap();
            for r in &rows {
                conn.execute(
                    "INSERT OR REPLACE INTO messages \
                     (id, session_id, role, content, attachments, reasoning, tool_calls, \
                      tool_call_id, tool_name, metadata, created_at, completed_at, streaming, \
                      input_tokens, output_tokens, total_tokens, cache_read_tokens, \
                      cache_write_tokens, model_id, tokens_per_second, mode, \
                      thinking_level, app_data) \
                      VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, \
                              ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
                    params![
                        r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8, r.9, r.10, r.11, r.12, r.13,
                        r.14, r.15, r.16, r.17, r.18, r.19, r.20, r.21, r.22,
                    ],
                )?;
            }
        }

        // ── 3. tool_outputs ── compress TEXT columns → BLOB ───────────────
        {
            let sql = format!(
                "SELECT id, session_id, message_id, tool_call_id, tool_name, output, \
                        byte_size, line_count, created_at \
                 FROM tool_outputs WHERE session_id IN ({imp_placeholder}) ORDER BY created_at ASC, rowid ASC"
            );
            let mut stmt = import_conn.prepare(&sql)?;
            let rows = stmt
                .query_map(imp_params.as_slice(), |row| {
                    let output_text: Option<String> = row.get(5)?;
                    let output_blob = match output_text {
                        Some(t) if !t.is_empty() => Some(compress_text(&t)),
                        Some(_) => Some(Vec::new()),
                        None => None,
                    };
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        output_blob,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let conn = self.write_conn.lock().unwrap();
            for r in &rows {
                conn.execute(
                    "INSERT OR REPLACE INTO tool_outputs \
                     (id, session_id, message_id, tool_call_id, tool_name, output, byte_size, line_count, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8,
                    ],
                )?;
            }
        }

        let imported = to_import
            .iter()
            .filter_map(|s| Uuid::parse_str(s).ok())
            .collect();
        Ok(imported)
    }
}

// ── Export helpers ───────────────────────────────────────────────────

/// Read a non-nullable BLOB (or TEXT) column, decompress it to text.
/// Returns an empty string if the value is empty or NULL.
fn blob_or_empty_to_text(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<String> {
    match row.get_ref(idx)? {
        rusqlite::types::ValueRef::Null => Ok(String::new()),
        rusqlite::types::ValueRef::Blob(b) => {
            if b.is_empty() {
                Ok(String::new())
            } else {
                Ok(decompress_text(b))
            }
        }
        rusqlite::types::ValueRef::Text(t) => {
            if t.is_empty() {
                Ok(String::new())
            } else {
                Ok(decompress_text(t))
            }
        }
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            idx,
            other.data_type(),
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expected BLOB, TEXT, or NULL",
            )),
        )),
    }
}

/// Read an optional BLOB (or TEXT) column, decompress it to optional text.
fn opt_blob_to_text(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Option<String>> {
    match row.get_ref(idx)? {
        rusqlite::types::ValueRef::Null => Ok(None),
        rusqlite::types::ValueRef::Blob(b) => {
            if b.is_empty() {
                Ok(None)
            } else {
                let text = decompress_text(b);
                if text.trim().is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(text))
                }
            }
        }
        rusqlite::types::ValueRef::Text(t) => {
            if t.is_empty() {
                Ok(None)
            } else {
                let text = decompress_text(t);
                if text.trim().is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(text))
                }
            }
        }
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            idx,
            other.data_type(),
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expected BLOB, TEXT, or NULL",
            )),
        )),
    }
}
