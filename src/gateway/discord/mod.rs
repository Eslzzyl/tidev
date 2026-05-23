//! Discord channel gateway module.
//!
//! Connects to Discord via Gateway WebSocket for real-time message
//! reception and uses the REST API for sending replies.

mod channel;
mod client;
mod types;

pub use channel::DiscordChannel;
