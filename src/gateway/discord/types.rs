//! Discord API data types for Gateway WebSocket and REST API.

use serde::Deserialize;

// ── Gateway opcodes ─────────────────────────────────────────────────────────

pub const OP_DISPATCH: u8 = 0;
pub const OP_HEARTBEAT: u8 = 1;
pub const OP_IDENTIFY: u8 = 2;
pub const OP_HELLO: u8 = 10;
pub const OP_HEARTBEAT_ACK: u8 = 11;

// ── Gateway intents ─────────────────────────────────────────────────────────

pub const INTENT_GUILDS: u32 = 1 << 0;
pub const INTENT_GUILD_MESSAGES: u32 = 1 << 9;
pub const INTENT_MESSAGE_CONTENT: u32 = 1 << 15;
pub const INTENT_DIRECT_MESSAGES: u32 = 1 << 17;

/// Default intents: GUILDS | GUILD_MESSAGES | MESSAGE_CONTENT | DIRECT_MESSAGES
pub const DEFAULT_INTENTS: u32 =
    INTENT_GUILDS | INTENT_GUILD_MESSAGES | INTENT_MESSAGE_CONTENT | INTENT_DIRECT_MESSAGES;

// ── Gateway WebSocket payload ───────────────────────────────────────────────

/// Generic gateway event frame.
#[derive(Debug, Deserialize)]
pub struct GatewayPayload {
    pub op: u8,
    pub d: Option<serde_json::Value>,
    pub s: Option<u32>,
    pub t: Option<String>,
}

/// Data inside opcode 10 (Hello).
#[derive(Debug, Deserialize)]
pub struct HelloData {
    pub heartbeat_interval: u64,
}

// ── Discord message objects (REST + Gateway) ────────────────────────────────

/// Minimal Discord user object.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct DiscordUser {
    pub id: String,
    pub username: String,
    pub discriminator: Option<String>,
    pub global_name: Option<String>,
    pub bot: Option<bool>,
}

/// Minimal Discord attachment object.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct DiscordAttachment {
    pub id: String,
    pub filename: String,
    pub content_type: Option<String>,
    pub size: u64,
    pub url: String,
}

/// Minimal Discord message object (as received via Gateway).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct DiscordMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: Option<u8>,
    pub channel_id: String,
    pub guild_id: Option<String>,
    pub author: DiscordUser,
    pub content: String,
    pub attachments: Vec<DiscordAttachment>,
    pub timestamp: String,
    pub edited_timestamp: Option<String>,
    pub mention_everyone: bool,
    pub mention_roles: Option<Vec<String>>,
    pub mentions: Option<Vec<DiscordUser>>,
}

/// Gateway bot info returned from GET /gateway/bot.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct BotGatewayResponse {
    pub url: String,
    pub shards: Option<u32>,
    pub session_start_limit: Option<SessionStartLimit>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SessionStartLimit {
    pub total: u32,
    pub remaining: u32,
    pub reset_after: u64,
    pub max_concurrency: u32,
}

/// A sent message response from the REST API (id is what we need).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SentMessageResponse {
    pub id: String,
}
