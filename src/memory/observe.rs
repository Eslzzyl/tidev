use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use uuid::Uuid;

use crate::memory::types::{
    HookPayload, HookType, ObservationResult,
};

use super::DedupMap;

/// Handle observation capture from hook payloads.
/// Replicates agentmemory's `mem::observe` function.
pub struct ObservationService;

impl ObservationService {
    /// Process a hook payload and create a raw observation.
    /// Returns `ObservationResult::New(id)` if a new observation was created,
    /// or `ObservationResult::Deduplicated` if it's a duplicate within the TTL window.
    pub fn observe(
        db: &Connection,
        dedup: &mut DedupMap,
        payload: &HookPayload,
    ) -> Result<ObservationResult> {
        // 1. Basic validation
        if payload.hook_type == HookType::SessionStart {
            // Don't observe session_start events
            return Ok(ObservationResult::Deduplicated);
        }

        // 2. SHA256 dedup check
        let tool_name = payload.tool_name.as_deref().unwrap_or("");
        let tool_input = payload.tool_input.as_deref().unwrap_or("");
        let hash = dedup.compute_hash(
            &payload.session_id.to_string(),
            tool_name,
            tool_input,
        );

        if dedup.is_duplicate(&hash) {
            return Ok(ObservationResult::Deduplicated);
        }

        // 3. Create observation
        let obs_id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();

        db.execute(
            "INSERT INTO observations (id, session_id, timestamp, hook_type, tool_name, tool_input, tool_output, user_prompt, assistant_response, modality, image_data, dedup_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                obs_id.to_string(),
                payload.session_id.to_string(),
                now,
                payload.hook_type.as_str(),
                payload.tool_name,
                payload.tool_input,
                payload.tool_output,
                payload.user_prompt,
                payload.assistant_response,
                "text",
                None::<String>,
                hash,
            ],
        )
        .context("failed to insert observation")?;

        // 4. Record dedup hash
        dedup.record(hash);

        Ok(ObservationResult::New(obs_id))
    }
}
