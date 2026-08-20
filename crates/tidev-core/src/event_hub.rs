//! Ordered, replayable frontend event distribution.
//!
//! Runtime events are ephemeral while messages are persisted in SQLite.  This
//! hub provides a bounded in-memory replay window for reconnecting frontends;
//! a client that falls behind the window must reload its durable session
//! snapshot before consuming new events.

use std::collections::VecDeque;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{
    Mutex,
    mpsc::{UnboundedReceiver, UnboundedSender},
};
use uuid::Uuid;

use crate::BackendEvent;

/// Maximum number of recent frontend events retained for reconnecting
/// clients. Event payloads are not persisted here; SQLite remains the source
/// of truth for durable session state.
const REPLAY_CAPACITY: usize = 4_096;

/// Monotonic cursor assigned at the frontend event boundary.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct EventCursor(pub u64);

/// An application event with a cursor that is stable for the lifetime of the
/// Runtime process.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub cursor: EventCursor,
    pub session_id: Uuid,
    pub event: BackendEvent,
}

/// Initial replay state for a new event subscription.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EventReplay {
    /// Events emitted after the requested cursor, in original order.
    Events(Vec<EventEnvelope>),
    /// The requested cursor predates the retained in-memory window. Reload a
    /// session snapshot before accepting live events from this subscription.
    ResyncRequired {
        after: EventCursor,
        oldest_available: EventCursor,
        latest_available: EventCursor,
    },
}

/// A live event receiver paired with its atomic initial replay result.
pub struct EventSubscription {
    pub replay: EventReplay,
    receiver: UnboundedReceiver<EventEnvelope>,
}

impl EventSubscription {
    /// Consume the subscription and receive live events emitted after the
    /// replay snapshot was computed.
    pub fn into_receiver(self) -> UnboundedReceiver<EventEnvelope> {
        self.receiver
    }
}

#[derive(Clone)]
pub(crate) struct EventHub {
    state: Arc<Mutex<EventHubState>>,
}

struct EventHubState {
    next_cursor: u64,
    replay: VecDeque<EventEnvelope>,
    subscribers: Vec<UnboundedSender<EventEnvelope>>,
}

impl EventHub {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(EventHubState {
                next_cursor: 1,
                replay: VecDeque::with_capacity(REPLAY_CAPACITY),
                subscribers: Vec::new(),
            })),
        }
    }

    pub(crate) async fn publish(&self, event: BackendEvent) {
        let mut state = self.state.lock().await;
        let envelope = EventEnvelope {
            cursor: EventCursor(state.next_cursor),
            session_id: event.session_id(),
            event,
        };
        state.next_cursor += 1;
        state.replay.push_back(envelope.clone());
        if state.replay.len() > REPLAY_CAPACITY {
            state.replay.pop_front();
        }
        state
            .subscribers
            .retain(|subscriber| subscriber.send(envelope.clone()).is_ok());
    }

    pub(crate) async fn subscribe(&self, after: Option<EventCursor>) -> EventSubscription {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = self.state.lock().await;
        let latest = EventCursor(state.next_cursor.saturating_sub(1));
        let replay = match after {
            None => EventReplay::Events(Vec::new()),
            Some(after) => {
                let oldest = state
                    .replay
                    .front()
                    .map(|event| event.cursor)
                    .unwrap_or(latest);
                let replay_start = oldest.0.saturating_sub(1);
                if after.0 < replay_start || after.0 > latest.0 {
                    EventReplay::ResyncRequired {
                        after,
                        oldest_available: oldest,
                        latest_available: latest,
                    }
                } else {
                    EventReplay::Events(
                        state
                            .replay
                            .iter()
                            .filter(|event| event.cursor > after)
                            .cloned()
                            .collect(),
                    )
                }
            }
        };
        state.subscribers.push(tx);
        EventSubscription {
            replay,
            receiver: rx,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BackendEvent;
    use uuid::Uuid;

    fn event() -> BackendEvent {
        BackendEvent::MessagesTruncated {
            session_id: Uuid::new_v4(),
            kept_count: 0,
        }
    }

    #[tokio::test]
    async fn replays_events_after_cursor_in_order() {
        let hub = EventHub::new();
        hub.publish(event()).await;
        hub.publish(event()).await;
        hub.publish(event()).await;

        let subscription = hub.subscribe(Some(EventCursor(1))).await;
        let EventReplay::Events(replay) = subscription.replay else {
            panic!("expected replayed events");
        };
        assert_eq!(replay.len(), 2);
        assert_eq!(replay[0].cursor, EventCursor(2));
        assert_eq!(replay[1].cursor, EventCursor(3));
    }

    #[tokio::test]
    async fn subscription_receives_events_emitted_after_registration() {
        let hub = EventHub::new();
        let subscription = hub.subscribe(None).await;
        let mut receiver = subscription.into_receiver();

        hub.publish(event()).await;
        let event = receiver.recv().await.expect("live event should arrive");
        assert_eq!(event.cursor, EventCursor(1));
    }

    #[tokio::test]
    async fn multiple_frontends_observe_identical_live_order() {
        let hub = EventHub::new();
        let mut first = hub.subscribe(None).await.into_receiver();
        let mut second = hub.subscribe(None).await.into_receiver();

        hub.publish(event()).await;
        hub.publish(event()).await;

        let first_cursors = [
            first
                .recv()
                .await
                .expect("first event should arrive")
                .cursor,
            first
                .recv()
                .await
                .expect("second event should arrive")
                .cursor,
        ];
        let second_cursors = [
            second
                .recv()
                .await
                .expect("first event should arrive")
                .cursor,
            second
                .recv()
                .await
                .expect("second event should arrive")
                .cursor,
        ];
        assert_eq!(first_cursors, [EventCursor(1), EventCursor(2)]);
        assert_eq!(second_cursors, first_cursors);
    }

    #[tokio::test]
    async fn stale_cursor_requires_snapshot_resync() {
        let hub = EventHub::new();
        for _ in 0..=REPLAY_CAPACITY {
            hub.publish(event()).await;
        }

        let subscription = hub.subscribe(Some(EventCursor(0))).await;
        assert!(matches!(
            subscription.replay,
            EventReplay::ResyncRequired {
                after: EventCursor(0),
                oldest_available: EventCursor(2),
                latest_available,
            } if latest_available == EventCursor((REPLAY_CAPACITY + 1) as u64)
        ));
    }
}
