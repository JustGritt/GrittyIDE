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

use gritty_workspace::WorkspacePath;

use crate::agent_run::wire::{AgentRunChangedPayload, AgentRunEventAppendedPayload};
use crate::agent_session::wire::{AgentRoster, AgentTimelineAppendedPayload};
use crate::document::{
    Document, DocumentChangeCause, DocumentClosedReason, ExternalModificationState,
};
use crate::file_drop::{DropHoverPayload, PathsDroppedPayload};
use crate::operation::{Operation, OperationProgress};
use crate::ownership::wire::OwnershipMap;
use crate::protocol::actor::Actor;
use crate::protocol::error::CoreError;
use crate::protocol::ids::{CorrelationId, DocumentId, OperationId, WorkspaceId};
use crate::search::wire::SearchMatchesFoundPayload;
use crate::session::wire::PersistenceStatusPayload;
use crate::settings::wire::SettingsChangedPayload;
use crate::terminal::wire::{TerminalExitedPayload, TerminalOutputPayload};
use crate::workspace::watch::WorkspaceEntriesChangedPayload;
use crate::workspace::{Workspace, WorkspaceClosedReason};
use crate::workstream::wire::{
    DecisionRequestedPayload, DecisionResolvedPayload, WorkstreamChangedPayload,
    WorkstreamCreatedPayload, WorkstreamRemovedPayload,
};

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
/// Further families are added by the tasks that own them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "name", content = "payload")]
pub enum CoreEvent {
    OperationStarted(OperationStartedPayload),
    OperationProgress(OperationProgressPayload),
    OperationCompleted(OperationCompletedPayload),
    OperationFailed(OperationFailedPayload),
    OperationCancelled(OperationCancelledPayload),
    WorkspaceOpened(WorkspaceOpenedPayload),
    WorkspaceClosed(WorkspaceClosedPayload),
    /// Directories under the open workspace whose entries changed on disk
    /// (T070). Coalesced and capped by the watcher, never one event per
    /// filesystem event (rule 8); `overflowed` says the list was dropped
    /// rather than truncated. It reports *that* something changed, never what
    /// it means: a document's external-modification state stays the on-demand
    /// answer of `documents.status` (ADR-011).
    WorkspaceEntriesChanged(WorkspaceEntriesChangedPayload),
    DocumentOpened(DocumentOpenedPayload),
    /// The buffer or its mode changed (an edit, a reload, read-only lifted).
    /// Carries the document snapshot, never the text: the text travels in the
    /// command response of the caller that changed it.
    DocumentChanged(DocumentChangedPayload),
    DocumentSaved(DocumentSavedPayload),
    /// The on-disk state moved between `UNCHANGED`, `MODIFIED_EXTERNALLY`,
    /// `CONFLICTED` and `DELETED_EXTERNALLY` (emitted on transitions only).
    DocumentExternalChangeDetected(DocumentExternalChangePayload),
    DocumentClosed(DocumentClosedPayload),
    /// The session database became unavailable or healthy again, or was
    /// replaced after a failed integrity check (T060). Emitted on transitions.
    PersistenceStatusChanged(PersistenceStatusPayload),
    /// A coalesced batch of terminal output (T090). Batched and bounded by the
    /// session's flusher, never one event per read.
    TerminalOutput(TerminalOutputPayload),
    /// A terminal session ended, exactly once, after its last output.
    TerminalExited(TerminalExitedPayload),
    /// Files or directories were dropped onto the window, already classified
    /// against the open workspace. Nothing was opened: the renderer decides
    /// and calls `workspace.open` / `documents.open` itself.
    PathsDropped(PathsDroppedPayload),
    /// A drag entered or left the window, so the UI can show its drop target.
    /// Enter and leave only, never the moves in between (rule 8).
    DropHoverChanged(DropHoverPayload),
    /// A workstream was created (T200).
    WorkstreamCreated(WorkstreamCreatedPayload),
    /// Its title, goal, state or open-decision count changed. One event per
    /// change, carrying the whole workstream: the roster is small and a
    /// consumer that missed an event still converges.
    WorkstreamChanged(WorkstreamChangedPayload),
    /// A workstream and its decisions were deleted.
    WorkstreamRemoved(WorkstreamRemovedPayload),
    /// A question was raised against a workstream and is waiting for the user.
    DecisionRequested(DecisionRequestedPayload),
    /// It was answered or dismissed; the payload's `state` says which.
    DecisionResolved(DecisionResolvedPayload),
    /// Who owns what in the workspace changed (T325): a lease granted,
    /// released or overridden, or a request refused. Carries the whole map,
    /// which is small and bounded, so a consumer that missed an event still
    /// converges on the next one.
    OwnershipChanged(OwnershipMap),
    /// A user setting changed (T120), by a command or by an edit to
    /// `settings.json` picked up when the window regained focus.
    ///
    /// Carries the whole effective set for the same reason `OwnershipChanged`
    /// does — a consumer that missed one still converges — plus the list of
    /// keys that moved, so a consumer can ignore an event that does not
    /// concern it. Emitted only when the effective values actually differ,
    /// which is also what keeps an applier that reads it from looping.
    SettingsChanged(SettingsChangedPayload),
    /// The roster of agent sessions running on this machine changed (T220).
    /// Carries the whole roster and is emitted only when it actually differs
    /// from the last one, never once per poll (rule 8).
    AgentSessionsChanged(AgentRoster),
    /// New entries were appended to an **open** agent timeline (T230).
    ///
    /// A delta, not a snapshot: a timeline grows without bound, so resending it
    /// per tick would be the unbounded forwarding rule 8 forbids. Batched by
    /// the follower, never one event per record, and emitted only for a session
    /// whose timeline someone opened.
    AgentTimelineAppended(AgentTimelineAppendedPayload),
    /// A run Gritty **owns** changed state or activity (T300).
    ///
    /// Not `AgentSessionsChanged`, and not about the same thing: that one
    /// reports a roster of agents some other terminal started, this one reports
    /// one run Gritty started and can cancel. Emitted on a state or activity
    /// change only — never per event — and carrying the whole run, the way
    /// `WorkstreamChanged` does: a run is small, and a consumer that missed one
    /// still converges on the next.
    AgentRunChanged(AgentRunChangedPayload),
    /// New events were appended to a run's bounded ring (T300).
    ///
    /// A delta, not a snapshot, for `AgentTimelineAppended`'s reason: a run's
    /// event stream grows without bound, so resending it would be the
    /// unbounded forwarding rule 8 forbids. Batches are coalesced by the run
    /// manager, never one event per call, and a consumer that missed one calls
    /// `runs.events` to resynchronise.
    AgentRunEventAppended(AgentRunEventAppendedPayload),
    /// A bounded batch of matches from a running `search.query` (T080).
    ///
    /// A delta, and deliberately not `OperationProgress`: that event's
    /// `message` is a human string, not match data. Batches are throttled and
    /// the queue behind them is bounded (rule 8), and every batch carries
    /// `truncatedMatches` so a match that was dropped is a number the UI can
    /// show rather than a silent gap.
    SearchMatchesFound(SearchMatchesFoundPayload),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceOpenedPayload {
    pub workspace: Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceClosedPayload {
    pub workspace_id: WorkspaceId,
    /// Root the workspace had, for the UI message after it is gone.
    pub root: String,
    pub reason: WorkspaceClosedReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DocumentOpenedPayload {
    pub document: Document,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DocumentChangedPayload {
    pub document: Document,
    pub cause: DocumentChangeCause,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSavedPayload {
    pub document: Document,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DocumentExternalChangePayload {
    pub document: Document,
    pub previous_state: ExternalModificationState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DocumentClosedPayload {
    pub document_id: DocumentId,
    pub workspace_id: WorkspaceId,
    pub path: WorkspacePath,
    pub reason: DocumentClosedReason,
    /// Unsaved changes were dropped with the buffer. `true` only after an
    /// explicit discard or a workspace that went away.
    pub dirty: bool,
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
            Self::WorkspaceOpened(_) => "WorkspaceOpened",
            Self::WorkspaceClosed(_) => "WorkspaceClosed",
            Self::WorkspaceEntriesChanged(_) => "WorkspaceEntriesChanged",
            Self::DocumentOpened(_) => "DocumentOpened",
            Self::DocumentChanged(_) => "DocumentChanged",
            Self::DocumentSaved(_) => "DocumentSaved",
            Self::DocumentExternalChangeDetected(_) => "DocumentExternalChangeDetected",
            Self::DocumentClosed(_) => "DocumentClosed",
            Self::PersistenceStatusChanged(_) => "PersistenceStatusChanged",
            Self::TerminalOutput(_) => "TerminalOutput",
            Self::TerminalExited(_) => "TerminalExited",
            Self::PathsDropped(_) => "PathsDropped",
            Self::DropHoverChanged(_) => "DropHoverChanged",
            Self::WorkstreamCreated(_) => "WorkstreamCreated",
            Self::WorkstreamChanged(_) => "WorkstreamChanged",
            Self::WorkstreamRemoved(_) => "WorkstreamRemoved",
            Self::DecisionRequested(_) => "DecisionRequested",
            Self::DecisionResolved(_) => "DecisionResolved",
            Self::AgentSessionsChanged(_) => "AgentSessionsChanged",
            Self::AgentTimelineAppended(_) => "AgentTimelineAppended",
            Self::AgentRunChanged(_) => "AgentRunChanged",
            Self::AgentRunEventAppended(_) => "AgentRunEventAppended",
            Self::SearchMatchesFound(_) => "SearchMatchesFound",
            Self::OwnershipChanged(_) => "OwnershipChanged",
            Self::SettingsChanged(_) => "SettingsChanged",
        }
    }

    /// The operation an event is about: the five `Operation*` events, and
    /// `SearchMatchesFound`, which streams from one.
    pub fn operation_id(&self) -> Option<&OperationId> {
        match self {
            Self::OperationStarted(p) => Some(&p.operation.id),
            Self::OperationProgress(p) => Some(&p.operation_id),
            Self::OperationCompleted(p) => Some(&p.operation.id),
            Self::OperationFailed(p) => Some(&p.operation.id),
            Self::OperationCancelled(p) => Some(&p.operation.id),
            Self::SearchMatchesFound(p) => Some(&p.operation_id),
            Self::WorkspaceOpened(_)
            | Self::WorkspaceClosed(_)
            | Self::WorkspaceEntriesChanged(_)
            | Self::DocumentOpened(_)
            | Self::DocumentChanged(_)
            | Self::DocumentSaved(_)
            | Self::DocumentExternalChangeDetected(_)
            | Self::DocumentClosed(_)
            | Self::PersistenceStatusChanged(_)
            | Self::TerminalOutput(_)
            | Self::TerminalExited(_)
            | Self::PathsDropped(_)
            | Self::DropHoverChanged(_)
            | Self::WorkstreamCreated(_)
            | Self::WorkstreamChanged(_)
            | Self::WorkstreamRemoved(_)
            | Self::DecisionRequested(_)
            | Self::DecisionResolved(_)
            | Self::AgentSessionsChanged(_)
            | Self::AgentTimelineAppended(_)
            | Self::AgentRunChanged(_)
            | Self::AgentRunEventAppended(_)
            | Self::OwnershipChanged(_)
            | Self::SettingsChanged(_) => None,
        }
    }

    /// The document a `Document*` event is about.
    pub fn document_id(&self) -> Option<&DocumentId> {
        match self {
            Self::DocumentOpened(p) => Some(&p.document.id),
            Self::DocumentChanged(p) => Some(&p.document.id),
            Self::DocumentSaved(p) => Some(&p.document.id),
            Self::DocumentExternalChangeDetected(p) => Some(&p.document.id),
            Self::DocumentClosed(p) => Some(&p.document_id),
            _ => None,
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

/// Delivers every envelope to several sinks in order (the WebView and the
/// session tap, T060). Each sink sees every event even when another fails;
/// the first failure is reported to the bus.
pub struct FanOutSink {
    sinks: Vec<Arc<dyn EventSink>>,
}

impl FanOutSink {
    pub fn new(sinks: Vec<Arc<dyn EventSink>>) -> Arc<Self> {
        Arc::new(Self { sinks })
    }
}

impl EventSink for FanOutSink {
    fn emit(&self, envelope: &EventEnvelope) -> Result<(), CoreError> {
        let mut first_error = None;
        for sink in &self.sinks {
            if let Err(error) = sink.emit(envelope)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
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
            durable: false,
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
    fn workspace_events_round_trip_without_an_operation_id() {
        let workspace = Workspace {
            id: WorkspaceId::from_raw("ws-1"),
            root: "/ws".into(),
            name: "ws".into(),
            opened_at: Timestamp::from_millis(7),
        };
        let events = [
            CoreEvent::WorkspaceOpened(WorkspaceOpenedPayload { workspace }),
            CoreEvent::WorkspaceClosed(WorkspaceClosedPayload {
                workspace_id: WorkspaceId::from_raw("ws-1"),
                root: "/ws".into(),
                reason: WorkspaceClosedReason::RootMissing,
            }),
        ];
        for event in events {
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(json["name"], event.name());
            assert_eq!(event.operation_id(), None);
            let back: CoreEvent = serde_json::from_value(json).unwrap();
            assert_eq!(back, event);
        }
        let json = serde_json::to_value(CoreEvent::WorkspaceClosed(WorkspaceClosedPayload {
            workspace_id: WorkspaceId::from_raw("ws-1"),
            root: "/ws".into(),
            reason: WorkspaceClosedReason::User,
        }))
        .unwrap();
        assert_eq!(json["payload"]["reason"], "USER");
    }

    /// The two families T300 adds: one snapshot, one delta, both named on the
    /// wire and neither of them an operation.
    #[test]
    fn agent_run_events_round_trip_and_are_not_operations() {
        use crate::agent_run::wire::{
            AgentRun, AgentRunChangedPayload, AgentRunEvent, AgentRunEventAppendedPayload,
            AgentRunEventRecord, AgentRunProvider, AgentRunState,
        };
        use crate::protocol::ids::{AgentRunId, WorkstreamId};

        let run = AgentRun {
            id: AgentRunId::from_raw("run-1"),
            workstream_id: WorkstreamId::from_raw("wst-1"),
            workspace_id: WorkspaceId::from_raw("ws-1"),
            provider: AgentRunProvider::Fake,
            state: AgentRunState::Running,
            activity: None,
            started_at: Timestamp::from_millis(1),
            updated_at: Timestamp::from_millis(2),
            finished_at: None,
            cancellation_requested: false,
            correlation_id: CorrelationId::from_raw("corr-1"),
            error: None,
            pending: None,
        };
        let events = [
            CoreEvent::AgentRunChanged(AgentRunChangedPayload { run }),
            CoreEvent::AgentRunEventAppended(AgentRunEventAppendedPayload {
                run_id: AgentRunId::from_raw("run-1"),
                events: vec![AgentRunEventRecord::new(
                    1,
                    AgentRunId::from_raw("run-1"),
                    Timestamp::from_millis(3),
                    AgentRunEvent::RunStarted,
                )],
                dropped: 0,
            }),
        ];
        for event in events {
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(json["name"], event.name());
            assert_eq!(event.operation_id(), None);
            assert_eq!(event.document_id(), None);
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
