//! Core events: the envelope, the vocabulary and the bus that numbers them.
//!
//! Wire shape of every event (addendum §6 names, T010 envelope):
//!
//! ```json
//! { "name": "OperationProgress", "seq": 12, "workspaceId": "...", "correlationId": "...",
//!   "actor": { "type": "USER" }, "payload": { ... } }
//! ```
//!
//! `seq` increases strictly per process, in the order the sink observes the
//! events. The core never talks to Tauri: it emits through an [`EventSink`]
//! the composition root provides. All events travel on one channel
//! ([`crate::CORE_EVENT_CHANNEL`]); the client demultiplexes by `name`.
//!
//! Gritty is not event-sourced: events notify, domain state stays authoritative.

use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::operation::{Operation, OperationProgress};
use crate::protocol::actor::Actor;
use crate::protocol::error::CoreError;
use crate::protocol::ids::{CorrelationId, OperationId, WorkspaceId};

/// Envelope fields that are not the event itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventMeta {
    /// Strictly increasing per process. Gaps or reordering on the receiving side
    /// indicate lost or misordered delivery.
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_id: Option<WorkspaceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub correlation_id: Option<CorrelationId>,
    pub actor: Actor,
}

/// The domain event vocabulary, adjacently tagged as `{ name, payload }`.
/// Further families (`Workspace*`, `Document*`, ...) are added by the tasks that own them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "name", content = "payload")]
pub enum CoreEvent {
    OperationStarted(OperationStartedPayload),
    OperationProgress(OperationProgressPayload),
    OperationCompleted(OperationCompletedPayload),
    OperationFailed(OperationFailedPayload),
    OperationCancelled(OperationCancelledPayload),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperationStartedPayload {
    pub operation: Operation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperationProgressPayload {
    pub operation_id: OperationId,
    pub progress: OperationProgress,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperationCompletedPayload {
    pub operation: Operation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperationFailedPayload {
    pub operation: Operation,
    pub error: CoreError,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperationCancelledPayload {
    pub operation: Operation,
}

impl CoreEvent {
    /// The `name` discriminator as it appears on the wire.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::OperationStarted(_) => "OperationStarted",
            Self::OperationProgress(_) => "OperationProgress",
            Self::OperationCompleted(_) => "OperationCompleted",
            Self::OperationFailed(_) => "OperationFailed",
            Self::OperationCancelled(_) => "OperationCancelled",
        }
    }

    /// The operation an `Operation*` event is about.
    pub fn operation_id(&self) -> Option<&OperationId> {
        match self {
            Self::OperationStarted(p) => Some(&p.operation.id),
            Self::OperationProgress(p) => Some(&p.operation_id),
            Self::OperationCompleted(p) => Some(&p.operation.id),
            Self::OperationFailed(p) => Some(&p.operation.id),
            Self::OperationCancelled(p) => Some(&p.operation.id),
        }
    }
}

/// What actually crosses the boundary: meta and event flattened into one object.
/// Not exported to TypeScript directly; the client composes `EventMeta & CoreEvent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    #[serde(flatten)]
    pub meta: EventMeta,
    #[serde(flatten)]
    pub event: CoreEvent,
}

/// Attribution attached to an emitted event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventOrigin {
    pub workspace_id: Option<WorkspaceId>,
    pub correlation_id: Option<CorrelationId>,
    pub actor: Actor,
}

/// Delivery target for envelopes; implemented by the desktop shell (Tauri `emit`)
/// and by recording sinks in tests. Must be cheap: it is called under the bus lock.
pub trait EventSink: Send + Sync {
    fn emit(&self, envelope: &EventEnvelope) -> Result<(), CoreError>;
}

/// Assigns sequence numbers and forwards envelopes to the sink.
///
/// Sequence assignment and delivery happen under one lock so the order the
/// sink sees equals `seq` order even when several tasks emit concurrently.
/// Sink failures are logged with the correlation id and never propagated:
/// an operation must not fail because the renderer went away.
pub struct EventBus {
    sink: Arc<dyn EventSink>,
    next_seq: Mutex<u64>,
}

impl EventBus {
    pub fn new(sink: Arc<dyn EventSink>) -> Arc<Self> {
        Arc::new(Self {
            sink,
            next_seq: Mutex::new(0),
        })
    }

    /// Emit one event; returns the sequence number it was assigned.
    pub fn emit(&self, origin: &EventOrigin, event: CoreEvent) -> u64 {
        let mut next_seq = self.next_seq.lock().unwrap_or_else(PoisonError::into_inner);
        *next_seq += 1;
        let seq = *next_seq;
        let envelope = EventEnvelope {
            meta: EventMeta {
                seq,
                workspace_id: origin.workspace_id.clone(),
                correlation_id: origin.correlation_id.clone(),
                actor: origin.actor.clone(),
            },
            event,
        };
        if let Err(error) = self.sink.emit(&envelope) {
            tracing::error!(
                seq,
                name = envelope.event.name(),
                correlation_id = ?envelope.meta.correlation_id,
                %error,
                "failed to deliver core event"
            );
        }
        seq
    }

    /// The last sequence number handed out (`0` before the first event).
    pub fn last_seq(&self) -> u64 {
        *self.next_seq.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::{Arc, Mutex, PoisonError};

    use tokio::sync::mpsc;

    use super::{EventEnvelope, EventSink};
    use crate::protocol::error::CoreError;

    /// Records every envelope and optionally forwards it to a channel so async
    /// tests can await delivery.
    pub struct RecordingSink {
        pub envelopes: Mutex<Vec<EventEnvelope>>,
        forward: Mutex<Option<mpsc::UnboundedSender<EventEnvelope>>>,
        pub fail: Mutex<bool>,
    }

    impl RecordingSink {
        pub fn new() -> Arc<Self> {
            Arc::new(Self {
                envelopes: Mutex::new(Vec::new()),
                forward: Mutex::new(None),
                fail: Mutex::new(false),
            })
        }

        /// A sink plus a receiver that yields envelopes as they are emitted.
        pub fn with_channel() -> (Arc<Self>, mpsc::UnboundedReceiver<EventEnvelope>) {
            let (tx, rx) = mpsc::unbounded_channel();
            let sink = Self::new();
            *sink.forward.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);
            (sink, rx)
        }

        pub fn recorded(&self) -> Vec<EventEnvelope> {
            self.envelopes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        pub fn names(&self) -> Vec<&'static str> {
            self.recorded().iter().map(|e| e.event.name()).collect()
        }
    }

    impl EventSink for RecordingSink {
        fn emit(&self, envelope: &EventEnvelope) -> Result<(), CoreError> {
            if *self.fail.lock().unwrap_or_else(PoisonError::into_inner) {
                return Err(CoreError::internal("sink failure"));
            }
            self.envelopes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(envelope.clone());
            if let Some(tx) = self
                .forward
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
            {
                // A dropped receiver only means the test stopped listening.
                let _ = tx.send(envelope.clone());
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::testing::RecordingSink;
    use super::*;
    use crate::operation::{OperationKind, OperationStatus};
    use crate::protocol::time::Timestamp;

    fn origin() -> EventOrigin {
        EventOrigin {
            workspace_id: Some(WorkspaceId::from_raw("ws-1")),
            correlation_id: Some(CorrelationId::from_raw("corr-1")),
            actor: Actor::User,
        }
    }

    fn operation() -> Operation {
        Operation {
            id: OperationId::from_raw("op-1"),
            kind: OperationKind::new("operations.demo"),
            actor: Actor::User,
            workspace_id: None,
            status: OperationStatus::Running,
            created_at: Timestamp::from_millis(1),
            started_at: Some(Timestamp::from_millis(2)),
            finished_at: None,
            cancellation_requested: false,
            correlation_id: CorrelationId::from_raw("corr-1"),
            progress: None,
            error: None,
        }
    }

    #[test]
    fn envelope_is_flat_with_name_seq_meta_and_payload() {
        let sink = RecordingSink::new();
        let bus = EventBus::new(sink.clone());
        let seq = bus.emit(
            &origin(),
            CoreEvent::OperationProgress(OperationProgressPayload {
                operation_id: OperationId::from_raw("op-1"),
                progress: OperationProgress {
                    current: 3,
                    total: Some(10),
                    message: None,
                },
            }),
        );
        assert_eq!(seq, 1);
        let json = serde_json::to_value(&sink.recorded()[0]).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "name": "OperationProgress",
                "seq": 1,
                "workspaceId": "ws-1",
                "correlationId": "corr-1",
                "actor": { "type": "USER" },
                "payload": { "operationId": "op-1", "progress": { "current": 3, "total": 10 } },
            })
        );
        let back: EventEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(back, sink.recorded()[0]);
    }

    #[test]
    fn every_event_family_round_trips() {
        let events = [
            CoreEvent::OperationStarted(OperationStartedPayload {
                operation: operation(),
            }),
            CoreEvent::OperationCompleted(OperationCompletedPayload {
                operation: operation(),
                result: Some(serde_json::json!({ "steps": 3 })),
            }),
            CoreEvent::OperationFailed(OperationFailedPayload {
                operation: operation(),
                error: CoreError::internal("boom"),
            }),
            CoreEvent::OperationCancelled(OperationCancelledPayload {
                operation: operation(),
            }),
        ];
        for event in events {
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(json["name"], event.name());
            assert_eq!(event.operation_id(), Some(&OperationId::from_raw("op-1")));
            let back: CoreEvent = serde_json::from_value(json).unwrap();
            assert_eq!(back, event);
        }
    }

    #[test]
    fn seq_is_strictly_increasing_across_threads() {
        let sink = RecordingSink::new();
        let bus = EventBus::new(sink.clone());
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let bus = bus.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        bus.emit(
                            &origin(),
                            CoreEvent::OperationStarted(OperationStartedPayload {
                                operation: operation(),
                            }),
                        );
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let seqs: Vec<u64> = sink.recorded().iter().map(|e| e.meta.seq).collect();
        assert_eq!(seqs, (1..=800).collect::<Vec<_>>());
        assert_eq!(bus.last_seq(), 800);
    }

    #[test]
    fn sink_failure_is_swallowed_and_seq_still_advances() {
        let sink = RecordingSink::new();
        *sink.fail.lock().unwrap() = true;
        let bus = EventBus::new(sink.clone());
        let seq = bus.emit(
            &origin(),
            CoreEvent::OperationStarted(OperationStartedPayload {
                operation: operation(),
            }),
        );
        assert_eq!(seq, 1);
        assert!(sink.recorded().is_empty());
        assert_eq!(bus.last_seq(), 1);
    }
}
