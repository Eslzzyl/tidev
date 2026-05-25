use tokio::sync::broadcast;

use crate::types::CronDeliveryMessage;

/// A broadcast-based delivery bus that connects the scheduler to gateway
/// channels.
///
/// The scheduler sends [`CronDeliveryMessage`]s through this bus, and each
/// gateway channel subscribes via [`DeliveryBus::subscribe`] to receive
/// messages destined for that channel type.
///
/// # How it works
///
/// 1. The gateway creates a single `DeliveryBus` and passes a sender to
///    the scheduler.
/// 2. Each platform channel (`TelegramChannel`, `QQChannel`, etc.) receives
///    a subscriber and integrates it into its run loop via `tokio::select!`.
/// 3. When a cron job completes with `delivery.mode == "announce"`, the
///    scheduler sends a `CronDeliveryMessage`.
/// 4. Each channel checks `message.delivery.channel` — if it matches its
///    platform name, it sends the output using its native messaging API.
#[derive(Clone)]
pub struct DeliveryBus {
    tx: broadcast::Sender<CronDeliveryMessage>,
}

impl DeliveryBus {
    /// Create a new delivery bus with the given channel capacity.
    ///
    /// Returns the bus (sender) and a receiver.  Call [`subscribe`](Self::subscribe)
    /// on the bus to create additional receivers for each channel.
    pub fn new(capacity: usize) -> (Self, broadcast::Receiver<CronDeliveryMessage>) {
        let (tx, rx) = broadcast::channel(capacity);
        (Self { tx }, rx)
    }

    /// Subscribe to receive delivery messages.
    ///
    /// Each channel should call this once and integrate the receiver into
    /// its event loop.
    pub fn subscribe(&self) -> broadcast::Receiver<CronDeliveryMessage> {
        self.tx.subscribe()
    }

    /// Send a delivery message to all subscribed channels.
    ///
    /// This is called by the scheduler after a job completes.  Errors are
    /// logged but not propagated (best-effort delivery).
    pub fn send(&self, msg: CronDeliveryMessage) {
        if let Err(e) = self.tx.send(msg) {
            log::warn!("DeliveryBus: failed to send message: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DeliveryConfig, CronDeliveryMessage};
    use chrono::Utc;

    #[test]
    fn test_delivery_bus_send_and_receive() {
        let (bus, mut rx) = DeliveryBus::new(16);

        let msg = CronDeliveryMessage {
            job_id: "test-1".into(),
            job_name: "Test Job".into(),
            output: "Hello".into(),
            delivery: DeliveryConfig {
                mode: "announce".into(),
                channel: Some("telegram".into()),
                to: Some("12345".into()),
                thread_id: None,
                best_effort: true,
            },
            success: true,
            executed_at: Utc::now(),
        };

        bus.send(msg.clone());

        // Since broadcast is async, we use try_recv for sync testing
        let received = rx.try_recv().unwrap();
        assert_eq!(received.job_id, "test-1");
        assert_eq!(received.output, "Hello");
    }

    #[test]
    fn test_delivery_bus_multiple_subscribers() {
        let (bus, _rx1) = DeliveryBus::new(16);
        let mut rx2 = bus.subscribe();
        let mut rx3 = bus.subscribe();

        let msg = CronDeliveryMessage {
            job_id: "test-2".into(),
            job_name: "Multi".into(),
            output: "broadcast".into(),
            delivery: DeliveryConfig::default(),
            success: true,
            executed_at: Utc::now(),
        };

        bus.send(msg);

        // Both subscribers should receive the message
        assert!(rx2.try_recv().is_ok());
        assert!(rx3.try_recv().is_ok());
    }
}
