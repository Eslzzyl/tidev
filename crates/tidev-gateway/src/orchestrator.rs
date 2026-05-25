//! Multi-channel orchestrator for concurrent gateway operation.
//!
//! The orchestrator manages multiple platform channels and runs them
//! concurrently, allowing a single gateway instance to serve all
//! configured platforms simultaneously.

use anyhow::{Result, bail};

use super::channel::Channel;

/// Orchestrator that runs multiple channels concurrently.
pub struct ChannelOrchestrator {
    channels: Vec<Box<dyn Channel>>,
}

impl ChannelOrchestrator {
    /// Create a new empty orchestrator.
    pub fn new() -> Self {
        Self {
            channels: Vec::new(),
        }
    }

    /// Add a channel to the orchestrator.
    pub fn add(&mut self, channel: Box<dyn Channel>) {
        self.channels.push(channel);
    }

    /// Check if any channels are registered.
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Get the list of channel names.
    pub fn channel_names(&self) -> Vec<&'static str> {
        self.channels.iter().map(|c| c.name()).collect()
    }

    /// Run all channels concurrently.
    ///
    /// This method spawns each channel in its own local task and waits
    /// for all channels to complete (or one to fail).
    pub async fn run(self) -> Result<()> {
        if self.channels.is_empty() {
            bail!("No channels configured");
        }

        let names = self.channel_names();
        log::info!("Starting {} channel(s): {}", names.len(), names.join(", "));

        // Restore sessions from persistent storage before starting channels.
        // Each channel has its own SessionStore, so we call restore_sessions on each.
        let mut total_restored = 0usize;
        let mut channels = self.channels;
        for channel in channels.iter_mut() {
            if let Some(store) = channel.store() {
                match channel.restore_sessions(store.clone()) {
                    Ok(count) => {
                        if count > 0 {
                            log::info!("Restored {} session(s) for {}", count, channel.name());
                        }
                        total_restored += count;
                    }
                    Err(e) => {
                        log::error!("Failed to restore sessions for {}: {}", channel.name(), e);
                    }
                }
            }
        }
        if total_restored > 0 {
            log::info!("Restored {} session(s) from disk", total_restored);
        }

        // Spawn each channel in its own local task
        let mut handles = Vec::new();
        for mut channel in channels {
            let handle = tokio::task::spawn_local(async move {
                let name = channel.name();
                if let Err(e) = channel.run().await {
                    log::error!("Channel {} failed: {}", name, e);
                    Err(e)
                } else {
                    Ok(())
                }
            });
            handles.push(handle);
        }

        // Wait for all channels to complete
        // If any channel fails, we log it but continue running others
        let mut any_error = None;
        for handle in handles {
            match handle.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    if any_error.is_none() {
                        any_error = Some(e);
                    }
                }
                Err(e) => {
                    log::error!("Channel task panicked: {}", e);
                    if any_error.is_none() {
                        any_error = Some(anyhow::anyhow!("Channel task panicked: {}", e));
                    }
                }
            }
        }

        if let Some(e) = any_error {
            Err(e)
        } else {
            Ok(())
        }
    }
}

impl Default for ChannelOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}
