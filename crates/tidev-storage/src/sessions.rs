use super::*;

// ---------------------------------------------------------------------------
// Session CRUD
// ---------------------------------------------------------------------------

impl SessionStore {
    /// Create a new session.
    #[allow(clippy::too_many_arguments)]
    pub fn create_session(
        &self,
        session_id: Uuid,
        workspace_root: &str,
        provider_id: &str,
        provider_display_name: &str,
        model_id: &str,
        model_display_name: &str,
        title: &str,
        parent_session_id: Option<Uuid>,
        snapshot_start_hash: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, parent_session_id, workspace_root, provider_id, provider_display_name, model_id, model_display_name, title, created_at, updated_at, snapshot_start_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![session_id.to_string(), parent_session_id.map(|id| id.to_string()), workspace_root, provider_id, provider_display_name, model_id, model_display_name, title, now, now, snapshot_start_hash],
        )?;
        Ok(())
    }

    /// Load session record by ID.
    pub fn load_session(&self, session_id: Uuid) -> Result<Option<SessionRecord>> {
        let sql = format!("SELECT {SESSION_SELECT_COLUMNS} FROM sessions s WHERE s.id = ?1");
        self.read(|conn| {
            conn.query_row(&sql, params![session_id.to_string()], |row| {
                Ok(map_row!(SessionRecord, row,
                    session_id: 0 => Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    parent_session_id: 1 => row.get::<_, Option<String>>(1)?.and_then(|s| Uuid::parse_str(&s).ok()),
                    workspace_root: 14 => row.get::<_, String>(14)?,
                    provider_id: 2 => row.get::<_, String>(2)?,
                    provider_display_name: 3 => row.get::<_, String>(3)?,
                    model_id: 4 => row.get::<_, String>(4)?,
                    model_display_name: 5 => row.get::<_, String>(5)?,
                    title: 6 => row.get::<_, String>(6)?,
                    created_at: 7 => DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?).unwrap().with_timezone(&Utc),
                    updated_at: 8 => DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?).unwrap().with_timezone(&Utc),
                    status: 9 => row.get::<_, String>(9)?,
                    ended_at: 10 => row.get::<_, Option<String>>(10)?.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
                    context_summary: 11 => opt_blob_to_text(row, 11)?,
                    context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                    system_prompt: 13 => blob_or_empty_to_text(row, 13)?,
                    snapshot_start_hash: 15 => row.get::<_, Option<String>>(15)?,
                ))
            })
            .optional()
            .map_err(Into::into)
        })
    }

    /// Find session IDs whose canonical UUID starts with the supplied prefix.
    /// Hyphens in the prefix are ignored so compact UUID prefixes work too.
    pub fn find_session_ids_by_prefix(&self, prefix: &str) -> Result<Vec<Uuid>> {
        let prefix = prefix.trim().replace('-', "").to_ascii_lowercase();
        self.read_query(
            "SELECT id FROM sessions WHERE substr(replace(lower(id), '-', ''), 1, length(?1)) = ?1 ORDER BY created_at DESC, id DESC",
            params![prefix],
            |row| {
                let id = row.get::<_, String>(0)?;
                Uuid::parse_str(&id).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
                })
            },
        )
    }

    /// Load a conversation (session + messages).
    pub fn load_conversation(&self, session_id: Uuid) -> Result<Option<schema::Conversation>> {
        let Some(session) = self.load_session(session_id)? else {
            return Ok(None);
        };
        let messages = self.load_messages(session_id)?;
        Ok(Some(schema::Conversation {
            session_id: session.session_id,
            parent_session_id: session.parent_session_id,
            workspace_root: session.workspace_root,
            provider_id: session.provider_id,
            provider_display_name: session.provider_display_name,
            model_id: session.model_id,
            model_display_name: session.model_display_name,
            title: session.title,
            created_at: session.created_at,
            updated_at: session.updated_at,
            context_summary: session.context_summary,
            context_retained_from: session.context_retained_from,
            messages,
            revert_message_id: None,
        }))
    }

    /// Load a session and all read-only data needed to inspect its history.
    pub fn load_session_inspection(&self, session_id: Uuid) -> Result<Option<SessionInspection>> {
        let Some(session) = self.load_session(session_id)? else {
            return Ok(None);
        };

        let messages = self.load_messages(session_id)?;
        let app_data = self.load_message_app_data(session_id)?;
        let tool_outputs = self.load_tool_outputs(session_id)?;
        let messages = messages
            .into_iter()
            .enumerate()
            .map(|(sequence, message)| {
                let message_id = message.id;
                StoredMessageView {
                    sequence,
                    message,
                    app_data: app_data.get(&message_id).cloned().unwrap_or_default(),
                    tool_output: tool_outputs.get(&message_id).cloned(),
                }
            })
            .collect();

        Ok(Some(SessionInspection { session, messages }))
    }

    /// Search textual session history without materializing complete messages.
    ///
    /// The query deliberately projects only the columns needed by the selected
    /// fields. In particular, image attachment data is never deserialized as
    /// part of a history search.
    pub fn search_history(&self, options: &SessionSearchOptions) -> Result<Vec<SessionSearchHit>> {
        let mut options = options.clone();
        options.query = options.query.trim().to_string();
        if options.query.is_empty() {
            anyhow::bail!("search query must not be empty");
        }

        let fields = options.fields;
        let mut hits = Vec::new();

        self.read(|conn| {
            let include_session_details = fields.session;
            let session_details = if include_session_details {
                ", s.context_summary, s.system_prompt"
            } else {
                ""
            };
            let mut session_conditions = vec![String::from("1 = 1")];
            let mut session_values: Vec<Box<dyn ToSql>> = Vec::new();
            if let Some(session_id) = options.session_id {
                session_conditions.push("s.id = ?".to_string());
                session_values.push(Box::new(session_id.to_string()));
            }
            if let Some(workspace_root) = options
                .workspace_root
                .as_deref()
                .filter(|root| !root.is_empty())
            {
                session_conditions.push("s.workspace_root = ?".to_string());
                session_values.push(Box::new(workspace_root.to_string()));
            }

            let session_sql = format!(
                "SELECT s.id, s.parent_session_id, s.title, s.workspace_root, \
                        s.provider_id, s.provider_display_name, s.model_id, \
                        s.model_display_name, s.created_at, s.updated_at, s.status, \
                        s.snapshot_start_hash{session_details} \
                 FROM sessions s WHERE {} \
                 ORDER BY s.updated_at DESC, s.id DESC",
                session_conditions.join(" AND ")
            );
            let mut session_stmt = conn.prepare(&session_sql)?;
            let session_rows = session_stmt.query_map(
                params_from_iter(session_values.iter().map(|value| value.as_ref())),
                |row| {
                    let parent_session_id = row
                        .get::<_, Option<String>>(1)?
                        .and_then(|value| Uuid::parse_str(&value).ok());
                    Ok(SearchSessionRow {
                        session_id: parse_search_uuid(&row.get::<_, String>(0)?),
                        parent_session_id,
                        title: row.get(2)?,
                        workspace_root: row.get(3)?,
                        provider_id: row.get(4)?,
                        provider_display_name: row.get(5)?,
                        model_id: row.get(6)?,
                        model_display_name: row.get(7)?,
                        created_at: parse_search_datetime(&row.get::<_, String>(8)?),
                        updated_at: parse_search_datetime(&row.get::<_, String>(9)?),
                        status: row.get(10)?,
                        snapshot_start_hash: row.get(11)?,
                        context_summary: if include_session_details {
                            row.get(12)?
                        } else {
                            None
                        },
                        system_prompt: if include_session_details {
                            row.get(13)?
                        } else {
                            None
                        },
                    })
                },
            )?;
            let mut sessions = Vec::new();
            for row in session_rows {
                sessions.push(row?);
            }
            drop(session_stmt);

            if fields.session {
                for session in &sessions {
                    let parent_session_id = session.parent_session_id.map(|id| id.to_string());
                    let scalar_fields = vec![
                        ("session.id", session.session_id.to_string()),
                        (
                            "session.parent_session_id",
                            parent_session_id.unwrap_or_default(),
                        ),
                        ("session.title", session.title.clone()),
                        ("session.workspace_root", session.workspace_root.clone()),
                        ("session.provider_id", session.provider_id.clone()),
                        (
                            "session.provider_display_name",
                            session.provider_display_name.clone(),
                        ),
                        ("session.model_id", session.model_id.clone()),
                        (
                            "session.model_display_name",
                            session.model_display_name.clone(),
                        ),
                        ("session.status", session.status.clone()),
                        (
                            "session.snapshot_start_hash",
                            session.snapshot_start_hash.clone().unwrap_or_default(),
                        ),
                    ];
                    for (field, text) in scalar_fields {
                        push_search_hit(
                            &mut hits,
                            SessionSearchHitKind::Session,
                            session.session_id,
                            &session.title,
                            &session.workspace_root,
                            None,
                            None,
                            None,
                            field,
                            &text,
                            None,
                            session.created_at,
                            &options,
                        );
                    }

                    if let Some(context_summary) = &session.context_summary {
                        let context_summary = decompress_text(context_summary);
                        push_search_hit(
                            &mut hits,
                            SessionSearchHitKind::Session,
                            session.session_id,
                            &session.title,
                            &session.workspace_root,
                            None,
                            None,
                            None,
                            "session.context_summary",
                            &context_summary,
                            None,
                            session.created_at,
                            &options,
                        );
                    }
                    if let Some(system_prompt) = &session.system_prompt {
                        let system_prompt = decompress_text(system_prompt);
                        push_search_hit(
                            &mut hits,
                            SessionSearchHitKind::Session,
                            session.session_id,
                            &session.title,
                            &session.workspace_root,
                            None,
                            None,
                            None,
                            "session.system_prompt",
                            &system_prompt,
                            None,
                            session.updated_at,
                            &options,
                        );
                    }
                }
            }

            if fields.includes_message() {
                let attachment_select = if fields.attachment {
                    ", m.attachments"
                } else {
                    ""
                };
                let mut message_conditions = vec![String::from("1 = 1")];
                let mut message_values: Vec<Box<dyn ToSql>> = Vec::new();
                if let Some(session_id) = options.session_id {
                    message_conditions.push("s.id = ?".to_string());
                    message_values.push(Box::new(session_id.to_string()));
                }
                if let Some(workspace_root) = options
                    .workspace_root
                    .as_deref()
                    .filter(|root| !root.is_empty())
                {
                    message_conditions.push("s.workspace_root = ?".to_string());
                    message_values.push(Box::new(workspace_root.to_string()));
                }
                if !options.roles.is_empty() {
                    let placeholders = std::iter::repeat_n("?", options.roles.len())
                        .collect::<Vec<_>>()
                        .join(", ");
                    message_conditions.push(format!("m.role IN ({placeholders})"));
                    for role in &options.roles {
                        message_values.push(Box::new(role.clone()));
                    }
                }

                let message_sql = format!(
                    "SELECT m.id, m.session_id, s.title, s.workspace_root, m.role, \
                            m.created_at, m.content, m.reasoning, m.tool_calls, \
                            m.tool_call_id, m.tool_name, m.metadata, m.mode, m.app_data, \
                            ROW_NUMBER() OVER (PARTITION BY m.session_id \
                                ORDER BY m.created_at ASC, m.rowid ASC) - 1 AS sequence\
                            {attachment_select} \
                     FROM messages m INNER JOIN sessions s ON s.id = m.session_id \
                     WHERE {} ORDER BY m.created_at DESC, m.rowid DESC",
                    message_conditions.join(" AND ")
                );
                let mut message_stmt = conn.prepare(&message_sql)?;
                let include_attachments = fields.attachment;
                let message_rows = message_stmt.query_map(
                    params_from_iter(message_values.iter().map(|value| value.as_ref())),
                    |row| {
                        Ok(SearchMessageRow {
                            message_id: parse_search_uuid(&row.get::<_, String>(0)?),
                            session_id: parse_search_uuid(&row.get::<_, String>(1)?),
                            title: row.get(2)?,
                            workspace_root: row.get(3)?,
                            role: row.get(4)?,
                            created_at: parse_search_datetime(&row.get::<_, String>(5)?),
                            content: row.get(6)?,
                            reasoning: row.get::<_, Option<Vec<u8>>>(7)?.unwrap_or_default(),
                            tool_calls: row.get(8)?,
                            tool_call_id: row.get(9)?,
                            tool_name: row.get(10)?,
                            metadata: row.get(11)?,
                            mode: row.get(12)?,
                            app_data: row.get(13)?,
                            sequence: row.get::<_, i64>(14)?.max(0) as usize,
                            attachments: if include_attachments {
                                Some(row.get(15)?)
                            } else {
                                None
                            },
                        })
                    },
                )?;

                for row in message_rows {
                    let row = row?;
                    if fields.content {
                        let content = decompress_text(&row.content);
                        push_search_hit(
                            &mut hits,
                            SessionSearchHitKind::Message,
                            row.session_id,
                            &row.title,
                            &row.workspace_root,
                            Some(row.message_id),
                            Some(row.sequence),
                            Some(&row.role),
                            "message.content",
                            &content,
                            None,
                            row.created_at,
                            &options,
                        );
                    }
                    if fields.reasoning {
                        let reasoning = decompress_text(&row.reasoning);
                        push_search_hit(
                            &mut hits,
                            SessionSearchHitKind::Message,
                            row.session_id,
                            &row.title,
                            &row.workspace_root,
                            Some(row.message_id),
                            Some(row.sequence),
                            Some(&row.role),
                            "message.reasoning",
                            &reasoning,
                            None,
                            row.created_at,
                            &options,
                        );
                    }
                    if fields.tool_call {
                        let tool_calls = decompress_text(&row.tool_calls);
                        push_search_hit(
                            &mut hits,
                            SessionSearchHitKind::Message,
                            row.session_id,
                            &row.title,
                            &row.workspace_root,
                            Some(row.message_id),
                            Some(row.sequence),
                            Some(&row.role),
                            "message.tool_calls",
                            &tool_calls,
                            None,
                            row.created_at,
                            &options,
                        );
                        if let Some(tool_call_id) = &row.tool_call_id {
                            push_search_hit(
                                &mut hits,
                                SessionSearchHitKind::Message,
                                row.session_id,
                                &row.title,
                                &row.workspace_root,
                                Some(row.message_id),
                                Some(row.sequence),
                                Some(&row.role),
                                "message.tool_call_id",
                                tool_call_id,
                                None,
                                row.created_at,
                                &options,
                            );
                        }
                        if let Some(tool_name) = &row.tool_name {
                            push_search_hit(
                                &mut hits,
                                SessionSearchHitKind::Message,
                                row.session_id,
                                &row.title,
                                &row.workspace_root,
                                Some(row.message_id),
                                Some(row.sequence),
                                Some(&row.role),
                                "message.tool_name",
                                tool_name,
                                None,
                                row.created_at,
                                &options,
                            );
                        }
                    }
                    if fields.metadata {
                        let metadata = decompress_text(&row.metadata);
                        push_search_hit(
                            &mut hits,
                            SessionSearchHitKind::Message,
                            row.session_id,
                            &row.title,
                            &row.workspace_root,
                            Some(row.message_id),
                            Some(row.sequence),
                            Some(&row.role),
                            "message.metadata",
                            &metadata,
                            None,
                            row.created_at,
                            &options,
                        );
                    }
                    if fields.app_data {
                        let app_data = decompress_text(&row.app_data);
                        push_search_hit(
                            &mut hits,
                            SessionSearchHitKind::Message,
                            row.session_id,
                            &row.title,
                            &row.workspace_root,
                            Some(row.message_id),
                            Some(row.sequence),
                            Some(&row.role),
                            "message.app_data",
                            &app_data,
                            None,
                            row.created_at,
                            &options,
                        );
                        if let Some(mode) = &row.mode {
                            push_search_hit(
                                &mut hits,
                                SessionSearchHitKind::Message,
                                row.session_id,
                                &row.title,
                                &row.workspace_root,
                                Some(row.message_id),
                                Some(row.sequence),
                                Some(&row.role),
                                "message.mode",
                                mode,
                                None,
                                row.created_at,
                                &options,
                            );
                        }
                    }
                    if fields.attachment
                        && let Some(attachments) = &row.attachments
                    {
                        push_attachment_search_hits(&mut hits, &row, attachments, &options);
                    }
                }
            }

            if fields.tool_output {
                let mut tool_output_conditions = vec![String::from("1 = 1")];
                let mut tool_output_values: Vec<Box<dyn ToSql>> = Vec::new();
                if let Some(session_id) = options.session_id {
                    tool_output_conditions.push("s.id = ?".to_string());
                    tool_output_values.push(Box::new(session_id.to_string()));
                }
                if let Some(workspace_root) = options
                    .workspace_root
                    .as_deref()
                    .filter(|root| !root.is_empty())
                {
                    tool_output_conditions.push("s.workspace_root = ?".to_string());
                    tool_output_values.push(Box::new(workspace_root.to_string()));
                }
                if !options.roles.is_empty() {
                    let placeholders = std::iter::repeat_n("?", options.roles.len())
                        .collect::<Vec<_>>()
                        .join(", ");
                    tool_output_conditions.push(format!("m.role IN ({placeholders})"));
                    for role in &options.roles {
                        tool_output_values.push(Box::new(role.clone()));
                    }
                }

                let tool_output_sql = format!(
                    "SELECT t.id, t.session_id, t.message_id, t.tool_call_id, \
                            t.tool_name, t.output, t.created_at, s.title, \
                            s.workspace_root, m.role \
                     FROM tool_outputs t INNER JOIN sessions s ON s.id = t.session_id \
                     LEFT JOIN messages m ON m.id = t.message_id \
                     WHERE {} ORDER BY t.created_at DESC, t.rowid DESC",
                    tool_output_conditions.join(" AND ")
                );
                let mut tool_output_stmt = conn.prepare(&tool_output_sql)?;
                let tool_output_rows = tool_output_stmt.query_map(
                    params_from_iter(tool_output_values.iter().map(|value| value.as_ref())),
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            parse_search_uuid(&row.get::<_, String>(1)?),
                            parse_search_uuid(&row.get::<_, String>(2)?),
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, Option<Vec<u8>>>(5)?,
                            parse_search_datetime(&row.get::<_, String>(6)?),
                            row.get::<_, String>(7)?,
                            row.get::<_, String>(8)?,
                            row.get::<_, Option<String>>(9)?,
                        ))
                    },
                )?;
                for row in tool_output_rows {
                    let (
                        output_id,
                        session_id,
                        message_id,
                        tool_call_id,
                        tool_name,
                        output,
                        created_at,
                        title,
                        workspace_root,
                        role,
                    ) = row?;
                    if let Some(output) = output {
                        let output = decompress_text(&output);
                        push_search_hit(
                            &mut hits,
                            SessionSearchHitKind::ToolOutput,
                            session_id,
                            &title,
                            &workspace_root,
                            Some(message_id),
                            None,
                            role.as_deref(),
                            "tool_output.content",
                            &output,
                            Some(&output_id),
                            created_at,
                            &options,
                        );
                    }
                    push_search_hit(
                        &mut hits,
                        SessionSearchHitKind::ToolOutput,
                        session_id,
                        &title,
                        &workspace_root,
                        Some(message_id),
                        None,
                        role.as_deref(),
                        "tool_output.tool_call_id",
                        &tool_call_id,
                        Some(&output_id),
                        created_at,
                        &options,
                    );
                    push_search_hit(
                        &mut hits,
                        SessionSearchHitKind::ToolOutput,
                        session_id,
                        &title,
                        &workspace_root,
                        Some(message_id),
                        None,
                        role.as_deref(),
                        "tool_output.tool_name",
                        &tool_name,
                        Some(&output_id),
                        created_at,
                        &options,
                    );
                }
            }

            Ok(())
        })?;

        hits.sort_unstable_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.session_id.cmp(&right.session_id))
                .then_with(|| left.message_id.cmp(&right.message_id))
                .then_with(|| left.field.cmp(&right.field))
        });

        let start = options.offset.min(hits.len());
        let end = start.saturating_add(options.limit).min(hits.len());
        Ok(hits[start..end].to_vec())
    }

    /// List all sessions ordered by creation time (newest first).
    pub fn list_sessions(&self, limit: i64, offset: i64) -> Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
             WHERE s.parent_session_id IS NULL \
             ORDER BY s.created_at DESC LIMIT ?1 OFFSET ?2"
        );
        self.read(|conn| {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit, offset], |row| {
            Ok(map_row!(SessionRecord, row,
                session_id: 0 => Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                parent_session_id: 1 => row.get::<_, Option<String>>(1)?.and_then(|s| Uuid::parse_str(&s).ok()),
                workspace_root: 14 => row.get::<_, String>(14)?,
                provider_id: 2 => row.get::<_, String>(2)?,
                provider_display_name: 3 => row.get::<_, String>(3)?,
                model_id: 4 => row.get::<_, String>(4)?,
                model_display_name: 5 => row.get::<_, String>(5)?,
                title: 6 => row.get::<_, String>(6)?,
                created_at: 7 => DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?).unwrap().with_timezone(&Utc),
                updated_at: 8 => DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?).unwrap().with_timezone(&Utc),
                status: 9 => row.get::<_, String>(9)?,
                ended_at: 10 => row.get::<_, Option<String>>(10)?.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
                context_summary: 11 => opt_blob_to_text(row, 11)?,
                context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                system_prompt: 13 => blob_or_empty_to_text(row, 13)?,
                snapshot_start_hash: 15 => row.get::<_, Option<String>>(15)?,
            ))
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
        })
    }

    /// List top-level sessions by their most recent activity.
    ///
    /// The cursor is exclusive. Ordering by both timestamp and ID keeps a page
    /// stable when several sessions share the same update time.
    pub fn list_sessions_by_activity(
        &self,
        limit: i64,
        cursor: Option<(DateTime<Utc>, Uuid)>,
        query: Option<&str>,
        workspace_root: Option<&str>,
    ) -> Result<Vec<SessionRecord>> {
        let mut clauses = vec![String::from("s.parent_session_id IS NULL")];
        let mut values: Vec<Box<dyn ToSql>> = Vec::new();

        if let Some(workspace_root) = workspace_root.filter(|root| !root.trim().is_empty()) {
            clauses.push("s.workspace_root = ?".to_string());
            values.push(Box::new(workspace_root.to_string()));
        }

        if let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) {
            let escaped = query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let pattern = format!("%{escaped}%");
            clauses.push(
                "(s.title LIKE ? ESCAPE '\\' OR s.workspace_root LIKE ? ESCAPE '\\')".to_string(),
            );
            values.push(Box::new(pattern.clone()));
            values.push(Box::new(pattern));
        }

        if let Some((updated_at, session_id)) = cursor {
            clauses.push("(s.updated_at < ? OR (s.updated_at = ? AND s.id < ?))".to_string());
            let timestamp = updated_at.to_rfc3339();
            values.push(Box::new(timestamp.clone()));
            values.push(Box::new(timestamp));
            values.push(Box::new(session_id.to_string()));
        }

        values.push(Box::new(limit.max(1)));
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
             WHERE {} \
             ORDER BY s.updated_at DESC, s.id DESC LIMIT ?",
            clauses.join(" AND ")
        );

        self.read_query(
            &sql,
            params_from_iter(values.iter().map(|value| value.as_ref())),
            Self::session_from_row,
        )
    }

    /// List each workspace that has at least one top-level session.
    pub fn list_session_workspace_roots(&self) -> Result<Vec<String>> {
        self.read_query(
            "SELECT s.workspace_root FROM sessions s \
             WHERE s.parent_session_id IS NULL \
             GROUP BY s.workspace_root \
             ORDER BY MAX(s.updated_at) DESC, s.workspace_root ASC",
            [],
            |row| row.get(0),
        )
    }

    /// List all sessions including children (no parent_session_id filter).
    /// Used internally for subsession navigation; session panel should use list_sessions instead.
    pub fn list_sessions_unfiltered(&self, limit: i64, offset: i64) -> Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
             ORDER BY s.created_at DESC LIMIT ?1 OFFSET ?2"
        );
        self.read(|conn| {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit, offset], |row| {
            Ok(map_row!(SessionRecord, row,
                session_id: 0 => Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                parent_session_id: 1 => row.get::<_, Option<String>>(1)?.and_then(|s| Uuid::parse_str(&s).ok()),
                workspace_root: 14 => row.get::<_, String>(14)?,
                provider_id: 2 => row.get::<_, String>(2)?,
                provider_display_name: 3 => row.get::<_, String>(3)?,
                model_id: 4 => row.get::<_, String>(4)?,
                model_display_name: 5 => row.get::<_, String>(5)?,
                title: 6 => row.get::<_, String>(6)?,
                created_at: 7 => DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?).unwrap().with_timezone(&Utc),
                updated_at: 8 => DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?).unwrap().with_timezone(&Utc),
                status: 9 => row.get::<_, String>(9)?,
                ended_at: 10 => row.get::<_, Option<String>>(10)?.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
                context_summary: 11 => opt_blob_to_text(row, 11)?,
                context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                system_prompt: 13 => blob_or_empty_to_text(row, 13)?,
                snapshot_start_hash: 15 => row.get::<_, Option<String>>(15)?,
            ))
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
        })
    }

    /// List sessions for a specific workspace, ordered by creation time (newest first).
    pub fn list_sessions_for_workspace(
        &self,
        workspace_root: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
             WHERE s.workspace_root = ?1 AND s.parent_session_id IS NULL \
             ORDER BY s.created_at DESC LIMIT ?2 OFFSET ?3"
        );
        self.read(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params![workspace_root, limit, offset],
                |row| {
                    Ok(map_row!(SessionRecord, row,
                        session_id: 0 => Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                        parent_session_id: 1 => row.get::<_, Option<String>>(1)?.and_then(|s| Uuid::parse_str(&s).ok()),
                        workspace_root: 14 => row.get::<_, String>(14)?,
                        provider_id: 2 => row.get::<_, String>(2)?,
                        provider_display_name: 3 => row.get::<_, String>(3)?,
                        model_id: 4 => row.get::<_, String>(4)?,
                        model_display_name: 5 => row.get::<_, String>(5)?,
                        title: 6 => row.get::<_, String>(6)?,
                        created_at: 7 => DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?).unwrap().with_timezone(&Utc),
                        updated_at: 8 => DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?).unwrap().with_timezone(&Utc),
                        status: 9 => row.get::<_, String>(9)?,
                        ended_at: 10 => row.get::<_, Option<String>>(10)?.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
                        context_summary: 11 => opt_blob_to_text(row, 11)?,
                        context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                        system_prompt: 13 => blob_or_empty_to_text(row, 13)?,
                        snapshot_start_hash: 15 => row.get::<_, Option<String>>(15)?,
                    ))
                },
            )?;
            let mut sessions = Vec::new();
            for row in rows {
                sessions.push(row?);
            }
            Ok(sessions)
        })
    }

    /// Update session metadata.
    #[allow(clippy::too_many_arguments)]
    pub fn update_session(
        &self,
        session_id: Uuid,
        title: Option<&str>,
        status: Option<&str>,
        context_summary: Option<&str>,
        context_retained_from: Option<usize>,
        system_prompt: Option<&str>,
        provider_id: Option<&str>,
        provider_display_name: Option<&str>,
        model_id: Option<&str>,
        model_display_name: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.write_conn.lock().unwrap();
        let mut sets = vec!["updated_at = ?1".to_string()];
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
        let mut idx = 2;

        if let Some(v) = title {
            sets.push(format!("title = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }
        if let Some(v) = status {
            sets.push(format!("status = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }
        if let Some(v) = context_summary {
            sets.push(format!("context_summary = ?{idx}"));
            params.push(Box::new(compress_text(v)));
            idx += 1;
        }
        if let Some(v) = context_retained_from {
            sets.push(format!("context_retained_from = ?{idx}"));
            params.push(Box::new(v as i64));
            idx += 1;
        }
        if let Some(v) = system_prompt {
            sets.push(format!("system_prompt = ?{idx}"));
            params.push(Box::new(compress_text(v)));
            idx += 1;
        }
        if let Some(v) = provider_id {
            sets.push(format!("provider_id = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }
        if let Some(v) = provider_display_name {
            sets.push(format!("provider_display_name = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }
        if let Some(v) = model_id {
            sets.push(format!("model_id = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }
        if let Some(v) = model_display_name {
            sets.push(format!("model_display_name = ?{idx}"));
            params.push(Box::new(v.to_string()));
            idx += 1;
        }

        let sql = format!("UPDATE sessions SET {} WHERE id = ?{idx}", sets.join(", "));
        params.push(Box::new(session_id.to_string()));

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        stmt.execute(param_refs.as_slice())?;
        Ok(())
    }

    /// Persist the snapshot start hash for a session.
    pub fn update_session_start_hash(
        &self,
        session_id: Uuid,
        snapshot_start_hash: &str,
    ) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET snapshot_start_hash = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                snapshot_start_hash,
                Utc::now().to_rfc3339(),
                session_id.to_string()
            ],
        )?;
        Ok(())
    }

    /// End a session (set status to 'ended' and ended_at).
    pub fn end_session(&self, session_id: Uuid) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET status = 'ended', ended_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, session_id.to_string()],
        )?;
        Ok(())
    }

    /// Delete a session and all related data (CASCADE).
    pub fn delete_session(&self, session_id: Uuid) -> Result<()> {
        let conn = self.write_conn.lock().unwrap();
        conn.execute(
            "DELETE FROM sessions WHERE id = ?1",
            params![session_id.to_string()],
        )?;
        Ok(())
    }

    /// Delete multiple sessions at once.
    pub fn delete_sessions(&self, session_ids: &[Uuid]) -> Result<()> {
        let ids: Vec<String> = session_ids.iter().map(|id| id.to_string()).collect();
        self.delete_sessions_by_ids(&ids)
    }

    /// Return sessions older than the given duration.
    pub fn get_sessions_older_than_preview(
        &self,
        duration: Duration,
    ) -> Result<Vec<SessionRecord>> {
        let cutoff = Utc::now() - duration;
        let cutoff_text = cutoff.to_rfc3339();
        self.read_query(
            &format!(
                "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
                 WHERE s.updated_at < :cutoff AND s.parent_session_id IS NULL \
                 ORDER BY s.workspace_root, s.updated_at DESC"
            ),
            named_params! { ":cutoff": cutoff_text },
            Self::session_from_row,
        )
    }

    /// Delete sessions older than the given duration. Returns the deleted records.
    pub fn delete_sessions_older_than(&self, duration: Duration) -> Result<Vec<SessionRecord>> {
        let cutoff = Utc::now() - duration;
        let cutoff_text = cutoff.to_rfc3339();

        let records: Vec<SessionRecord> = self.read_query(
            &format!(
                "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
                 WHERE s.updated_at < :cutoff AND s.parent_session_id IS NULL \
                 ORDER BY s.updated_at DESC"
            ),
            named_params! { ":cutoff": cutoff_text },
            Self::session_from_row,
        )?;

        let session_ids: Vec<String> = records.iter().map(|r| r.session_id.to_string()).collect();
        self.delete_sessions_by_ids(&session_ids)?;

        Ok(records)
    }

    /// Delete all sessions in a workspace. Returns the deleted records.
    pub fn delete_sessions_in_workspace(
        &self,
        workspace_root: &Path,
    ) -> Result<Vec<SessionRecord>> {
        let root = workspace_root.display().to_string();

        let records: Vec<SessionRecord> = self.read_query(
            &format!(
                "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
                 WHERE s.workspace_root = :workspace_root AND s.parent_session_id IS NULL \
                 ORDER BY s.updated_at DESC"
            ),
            named_params! { ":workspace_root": root },
            Self::session_from_row,
        )?;

        let session_ids: Vec<String> = records.iter().map(|r| r.session_id.to_string()).collect();
        self.delete_sessions_by_ids(&session_ids)?;

        Ok(records)
    }

    /// Export a session as JSONL file. Returns the file path.
    pub fn export_session_to_jsonl(&self, session_id: Uuid, export_dir: &Path) -> Result<PathBuf> {
        let messages = self.load_messages(session_id)?;
        std::fs::create_dir_all(export_dir)?;
        let file_path = export_dir.join(format!("session_{session_id}.jsonl"));
        let mut file = std::fs::File::create(&file_path)?;
        for msg in &messages {
            let line = serde_json::to_string(msg)?;
            writeln!(file, "{line}")?;
        }
        Ok(file_path)
    }

    /// Export one or more sessions to a single JSONL message stream.
    ///
    /// Each line contains the session ID, a stable per-session sequence, and
    /// the protocol message. This format is intended for CLI inspection and
    /// scripting; the existing TUI export keeps its legacy one-message shape.
    pub fn export_to_jsonl(&self, session_ids: &[Uuid], output_path: &Path) -> Result<usize> {
        for session_id in session_ids {
            if self.load_session(*session_id)?.is_none() {
                anyhow::bail!("session not found: {session_id}");
            }
        }

        if let Some(parent) = output_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create export directory {}", parent.display())
            })?;
        }

        let mut file = fs::File::create(output_path)
            .with_context(|| format!("failed to create JSONL export {}", output_path.display()))?;
        let mut message_count = 0;

        for session_id in session_ids {
            let messages = self.load_messages(*session_id)?;
            for (sequence, message) in messages.iter().enumerate() {
                let record = JsonlMessageRecord {
                    session_id: *session_id,
                    sequence,
                    message,
                };
                serde_json::to_writer(&mut file, &record)?;
                file.write_all(b"\n")?;
                message_count += 1;
            }
        }

        file.flush()?;
        Ok(message_count)
    }

    /// Count sessions in a workspace.
    pub fn get_current_workspace_sessions_count(&self, workspace_root: &Path) -> Result<i64> {
        self.read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM sessions WHERE workspace_root = ?1 AND parent_session_id IS NULL",
                params![workspace_root.to_string_lossy().to_string()],
                |row| row.get(0),
            )
            .map_err(Into::into)
        })
    }

    /// Alias for load_session (compatibility).
    pub fn load_session_record(&self, session_id: Uuid) -> Result<Option<SessionRecord>> {
        self.load_session(session_id)
    }

    /// Load all retained tool outputs for a session.
    pub fn load_tool_outputs(&self, session_id: Uuid) -> Result<HashMap<Uuid, ToolOutputRecord>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, message_id, tool_call_id, tool_name, byte_size, line_count, created_at \
                 FROM tool_outputs WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
            )?;
            let rows = stmt.query_map(params![session_id.to_string()], |row| {
                let id: String = row.get(0)?;
                let stored_session_id =
                    Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or(session_id);
                let message_id = Uuid::parse_str(&row.get::<_, String>(2)?).unwrap_or_default();
                let tool_call_id: String = row.get(3)?;
                let tool_name: String = row.get(4)?;
                let byte_size: usize = row.get::<_, i64>(5)? as usize;
                let line_count: usize = row.get::<_, i64>(6)? as usize;
                let created_at: String = row.get(7)?;

                Ok(ToolOutputRecord {
                    id,
                    session_id: stored_session_id,
                    message_id,
                    tool_call_id,
                    tool_name,
                    byte_size,
                    line_count,
                    created_at,
                })
            })?;
            let mut tool_outputs = HashMap::new();
            for row in rows {
                let record = row?;
                tool_outputs.insert(record.message_id, record);
            }
            Ok(tool_outputs)
        })
    }

    /// Load tool output content by output id, message_id, or tool_call_id.
    pub fn load_tool_output(&self, id_or_alias: &str) -> Result<Option<ToolOutputContent>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, message_id, tool_call_id, tool_name, output, byte_size, line_count, created_at \
                 FROM tool_outputs \
                 WHERE id = ?1 OR message_id = ?1 OR tool_call_id = ?1 \
                 LIMIT 1",
            )?;
            let mut rows = stmt.query_map(params![id_or_alias], |row| {
                let id: String = row.get(0)?;
                let session_id = Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default();
                let message_id = Uuid::parse_str(&row.get::<_, String>(2)?).unwrap_or_default();
                let tool_call_id: String = row.get(3)?;
                let tool_name: String = row.get(4)?;
                let output_blob: Option<Vec<u8>> = row.get(5)?;
                let byte_size: usize = row.get::<_, i64>(6)? as usize;
                let line_count: usize = row.get::<_, i64>(7)? as usize;
                let created_at: String = row.get(8)?;

                let record = ToolOutputRecord {
                    id,
                    session_id,
                    message_id,
                    tool_call_id,
                    tool_name,
                    byte_size,
                    line_count,
                    created_at,
                };

                match output_blob {
                    Some(blob) => Ok(ToolOutputContent::Available {
                        record,
                        output: decompress_text(&blob),
                    }),
                    None => Ok(ToolOutputContent::Expired { record }),
                }
            })?;
            match rows.next() {
                Some(Ok(content)) => Ok(Some(content)),
                _ => Ok(None),
            }
        })
    }

    /// Save tool output for a tool call.
    pub fn save_tool_output(
        &self,
        id: &str,
        session_id: Uuid,
        message_id: Uuid,
        tool_call_id: &str,
        tool_name: &str,
        raw_output: &str,
    ) -> Result<()> {
        let compressed = compress_text(raw_output);
        let byte_size = raw_output.len() as i64;
        let line_count = raw_output.lines().count() as i64;
        let now = Utc::now().to_rfc3339();
        self.write_execute(
            "INSERT OR REPLACE INTO tool_outputs \
             (id, session_id, message_id, tool_call_id, tool_name, output, byte_size, line_count, created_at) \
             VALUES (:id, :session_id, :message_id, :tool_call_id, :tool_name, :output, :byte_size, :line_count, :created_at)",
            named_params! {
                ":id": id,
                ":session_id": session_id.to_string(),
                ":message_id": message_id.to_string(),
                ":tool_call_id": tool_call_id,
                ":tool_name": tool_name,
                ":output": compressed,
                ":byte_size": byte_size,
                ":line_count": line_count,
                ":created_at": now,
            },
        )?;
        Ok(())
    }

    /// Clear big payloads for tool outputs older than `max_age_days` (tombstone pattern).
    /// Sets `output` column to NULL while preserving metadata for inspection.
    pub fn clear_expired_tool_outputs(&self, max_age_days: i64) -> Result<usize> {
        let cutoff = (Utc::now() - Duration::days(max_age_days)).to_rfc3339();
        self.write_execute(
            "UPDATE tool_outputs SET output = NULL WHERE created_at < :cutoff AND output IS NOT NULL",
            named_params! { ":cutoff": cutoff },
        )
    }

    /// Delete tombstone records older than `max_age_days`.
    pub fn delete_tombstones_older_than(&self, max_age_days: i64) -> Result<usize> {
        let cutoff = (Utc::now() - Duration::days(max_age_days)).to_rfc3339();
        self.write_execute(
            "DELETE FROM tool_outputs WHERE created_at < :cutoff AND output IS NULL",
            named_params! { ":cutoff": cutoff },
        )
    }

    /// Count total number of sessions.
    pub fn count_sessions(&self) -> Result<i64> {
        self.read(|conn| {
            conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
                .map_err(Into::into)
        })
    }

    /// Count sessions per workspace.
    pub fn count_sessions_by_workspace(&self) -> Result<Vec<WorkspaceSessionCount>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.workspace_root, COUNT(*) as cnt FROM sessions s \
                 WHERE s.workspace_root != '' \
                 GROUP BY s.workspace_root ORDER BY cnt DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(WorkspaceSessionCount {
                    workspace_root: row.get(0)?,
                    session_count: row.get(1)?,
                })
            })?;
            let mut counts = Vec::new();
            for row in rows {
                counts.push(row?);
            }
            Ok(counts)
        })
    }

    /// Search sessions by title.
    pub fn search_sessions(&self, query: &str, limit: i64) -> Result<Vec<SessionRecord>> {
        let sql = format!(
            "SELECT {SESSION_SELECT_COLUMNS} FROM sessions s \
             WHERE s.title LIKE ?1 OR s.id = ?2 \
             ORDER BY s.created_at DESC LIMIT ?3"
        );
        let pattern = format!("%{}%", query);
        let id_match = Uuid::parse_str(query)
            .map(|id| id.to_string())
            .unwrap_or_default();
        self.read(|conn| {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![pattern, id_match, limit], |row| {
            Ok(map_row!(SessionRecord, row,
                session_id: 0 => Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                parent_session_id: 1 => row.get::<_, Option<String>>(1)?.and_then(|s| Uuid::parse_str(&s).ok()),
                workspace_root: 14 => row.get::<_, String>(14)?,
                provider_id: 2 => row.get::<_, String>(2)?,
                provider_display_name: 3 => row.get::<_, String>(3)?,
                model_id: 4 => row.get::<_, String>(4)?,
                model_display_name: 5 => row.get::<_, String>(5)?,
                title: 6 => row.get::<_, String>(6)?,
                created_at: 7 => DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?).unwrap().with_timezone(&Utc),
                updated_at: 8 => DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?).unwrap().with_timezone(&Utc),
                status: 9 => row.get::<_, String>(9)?,
                ended_at: 10 => row.get::<_, Option<String>>(10)?.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
                context_summary: 11 => opt_blob_to_text(row, 11)?,
                context_retained_from: 12 => row.get::<_, i64>(12)? as usize,
                system_prompt: 13 => blob_or_empty_to_text(row, 13)?,
                snapshot_start_hash: 15 => row.get::<_, Option<String>>(15)?,
            ))
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
        })
    }

    /// Get workspaces that have sessions.
    pub fn list_workspaces(&self, limit: i64, offset: i64) -> Result<Vec<String>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT workspace_root FROM sessions \
                 WHERE workspace_root != '' \
                 ORDER BY workspace_root LIMIT ?1 OFFSET ?2",
            )?;
            let rows = stmt.query_map(params![limit, offset], |row| row.get(0))?;
            let mut workspaces = Vec::new();
            for row in rows {
                workspaces.push(row?);
            }
            Ok(workspaces)
        })
    }
}
