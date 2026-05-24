//! Lark/Feishu API data types and Protobuf WebSocket frame definitions.

use serde::Deserialize;

// ── Protobuf frame types (pbbp2.proto) ──────────────────────────────────────

/// Feishu WS frame header.
#[derive(Clone, PartialEq, prost::Message)]
pub struct PbHeader {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

/// Feishu WS frame (pbbp2.proto).
/// method=0 → CONTROL (ping/pong), method=1 → DATA (events)
#[derive(Clone, PartialEq, prost::Message)]
pub struct PbFrame {
    #[prost(uint64, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, tag = "2")]
    pub log_id: u64,
    #[prost(int32, tag = "3")]
    pub service: i32,
    #[prost(int32, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<PbHeader>,
    #[prost(bytes = "vec", optional, tag = "8")]
    pub payload: Option<Vec<u8>>,
}

impl PbFrame {
    pub fn header_value(&self, key: &str) -> &str {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
            .unwrap_or("")
    }
}

// ── WS endpoint response ────────────────────────────────────────────────────

/// Server-sent client config (parsed from pong payload).
#[derive(Debug, Deserialize, Default, Clone)]
pub struct WsClientConfig {
    #[serde(rename = "PingInterval")]
    pub ping_interval: Option<u64>,
}

/// Response from POST /callback/ws/endpoint.
#[derive(Debug, Deserialize)]
pub struct WsEndpointResp {
    pub code: i32,
    #[serde(default)]
    pub msg: Option<String>,
    #[serde(default)]
    pub data: Option<WsEndpoint>,
}

#[derive(Debug, Deserialize)]
pub struct WsEndpoint {
    #[serde(rename = "URL")]
    pub url: String,
    #[serde(default)]
    pub client_config: Option<WsClientConfig>,
}

// ── Authentication ──────────────────────────────────────────────────────────

/// Response from POST /auth/v3/tenant_access_token/internal.
#[derive(Debug, Deserialize)]
pub struct TenantAccessTokenResp {
    pub code: i32,
    #[serde(default)]
    pub msg: Option<String>,
    #[serde(default)]
    pub tenant_access_token: Option<String>,
    #[serde(default)]
    pub expire: Option<u64>,
}

// ── Bot info ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BotInfoResp {
    pub code: i32,
    #[serde(default)]
    pub msg: Option<String>,
    #[serde(default)]
    pub bot: Option<BotInfo>,
}

#[derive(Debug, Deserialize)]
pub struct BotInfo {
    #[serde(default)]
    pub open_id: String,
}

// ── Event data types (im.message.receive_v1) ────────────────────────────────

/// Payload wrapper for push events.
#[derive(Debug, Deserialize, Default)]
pub struct EventPayload {
    #[serde(default)]
    pub sender: EventSender,
    #[serde(default)]
    pub message: EventMessage,
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
pub struct EventSender {
    #[serde(rename = "sender_id")]
    pub sender_id: EventId,
    #[serde(rename = "sender_type")]
    pub sender_type: EventSenderType,
    #[serde(rename = "tenant_key")]
    pub tenant_key: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
pub struct EventSenderType {
    #[serde(rename = "app_id")]
    pub app_id: Option<String>,
    #[serde(rename = "open_id")]
    pub open_id: Option<String>,
    #[serde(rename = "user_id")]
    pub user_id: Option<String>,
}

impl EventSenderType {
    pub fn open_id(&self) -> Option<&str> {
        self.open_id.as_deref()
    }
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
pub struct EventId {
    #[serde(rename = "open_id")]
    pub open_id: String,
    #[serde(rename = "union_id")]
    pub union_id: Option<String>,
    #[serde(rename = "user_id")]
    pub user_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
pub struct EventMessage {
    #[serde(rename = "message_id")]
    pub message_id: String,
    #[serde(rename = "root_id")]
    pub root_id: Option<String>,
    #[serde(rename = "parent_id")]
    pub parent_id: Option<String>,
    #[serde(rename = "msg_type")]
    pub msg_type: String,
    pub content: String,
    #[serde(rename = "chat_id")]
    pub chat_id: String,
    #[serde(rename = "chat_type")]
    pub chat_type: String,
}

// ── Send message responses ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SendMessageResp {
    pub code: i32,
    #[serde(default)]
    pub msg: Option<String>,
    #[serde(default)]
    pub data: Option<SendMessageData>,
}

#[derive(Debug, Deserialize)]
pub struct SendMessageData {
    #[serde(rename = "message_id")]
    pub message_id: String,
}
