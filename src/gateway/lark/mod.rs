//! Lark/Feishu channel gateway module.
//!
//! Connects to Lark/Feishu Open Platform via WebSocket (protobuf-framed)
//! for real-time event reception and uses the REST API for sending replies.

mod channel;
mod client;
mod types;

pub use channel::LarkChannel;
