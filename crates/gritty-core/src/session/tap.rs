//! The event tap: turns core events into small intents for the writer task.
//!
//! It is one of the bus's sinks (`FanOutSink`), so it runs under the bus lock
//! and must be cheap: it copies ids and paths, never a document, and never
//! blocks. The channel is bounded; when it is full the intent is dropped and
//! an overflow flag tells the writer to resynchronize from the managers.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gritty_workspace::WorkspacePath;
use tokio::sync::{mpsc, oneshot};

use super::WindowGeometry;
use crate::document::{DocumentChangeCause, DocumentClosedReason};
use crate::event::{CoreEvent, EventEnvelope, EventSink};
use crate::operation::Operation;
use crate::protocol::error::CoreError;
use crate::protocol::ids::{DocumentId, WorkspaceId};
use crate::workspace::WorkspaceClosedReason;

/// What the writer needs to know about an event.
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    /// The buffer may have changed (opened, edited, reloaded, saved).
    DocumentTouched(DocumentId),
    /// A snapshot was put back into the buffer: from now on the row belongs
    /// to this process (it is removed once the document is clean or closed).
    DocumentRecovered(DocumentId),
    DocumentClosed {
        id: DocumentId,
        workspace_id: WorkspaceId,
        path: WorkspacePath,
        dirty: bool,
        reason: DocumentClosedReason,
    },
    WorkspaceOpened {
        id: WorkspaceId,
        root: String,
    },
    WorkspaceClosed {
        id: WorkspaceId,
        reason: WorkspaceClosedReason,
    },
    /// A durable operation moved; the snapshot is persisted as is.
    Operation(Box<Operation>),
}

impl Intent {
    pub fn from_event(event: &CoreEvent) -> Option<Self> {
        match event {
            CoreEvent::DocumentOpened(p) => Some(Self::DocumentTouched(p.document.id.clone())),
            CoreEvent::DocumentChanged(p) if p.cause == DocumentChangeCause::Recovered => {
                Some(Self::DocumentRecovered(p.document.id.clone()))
            }
            CoreEvent::DocumentChanged(p) => Some(Self::DocumentTouched(p.document.id.clone())),
            CoreEvent::DocumentSaved(p) => Some(Self::DocumentTouched(p.document.id.clone())),
            CoreEvent::DocumentClosed(p) => Some(Self::DocumentClosed {
                id: p.document_id.clone(),
                workspace_id: p.workspace_id.clone(),
                path: p.path.clone(),
                dirty: p.dirty,
                reason: p.reason,
            }),
            CoreEvent::WorkspaceOpened(p) => Some(Self::WorkspaceOpened {
                id: p.workspace.id.clone(),
                root: p.workspace.root.clone(),
            }),
            CoreEvent::WorkspaceClosed(p) => Some(Self::WorkspaceClosed {
                id: p.workspace_id.clone(),
                reason: p.reason,
            }),
            CoreEvent::OperationStarted(p) => durable(&p.operation),
            CoreEvent::OperationCompleted(p) => durable(&p.operation),
            CoreEvent::OperationFailed(p) => durable(&p.operation),
            CoreEvent::OperationCancelled(p) => durable(&p.operation),
            CoreEvent::OperationProgress(_)
            // What the watcher (T070) saw on disk is a fact about the disk,
            // rediscovered by looking at it again. Persisting it would let a
            // restart claim a directory changed while Gritty was closed.
            | CoreEvent::WorkspaceEntriesChanged(_)
            | CoreEvent::DocumentExternalChangeDetected(_)
            | CoreEvent::PersistenceStatusChanged(_)
            // Terminal buffers are an explicit T090 non-goal: nothing a shell
            // prints is persisted.
            | CoreEvent::TerminalOutput(_)
            | CoreEvent::TerminalExited(_)
            // A drop is a gesture, not state. What it leads to — a workspace
            // opened, a document opened — is persisted by those events; the
            // gesture itself must not survive a restart (T045).
            | CoreEvent::PathsDropped(_)
            | CoreEvent::DropHoverChanged(_)
            // Workstreams write themselves (T200): the command that changed
            // one has already committed before its event is emitted, so there
            // is nothing here for the session writer to persist afterwards.
            | CoreEvent::WorkstreamCreated(_)
            | CoreEvent::WorkstreamChanged(_)
            | CoreEvent::WorkstreamRemoved(_)
            // Ownership writes itself too (T325), in the same transaction that
            // decided the answer.
            | CoreEvent::OwnershipChanged(_)
            // Settings write themselves, to their own file (ADR-031). The
            // session database is deliberately not where they live: a settings
            // file you can repair in any editor when the application will not
            // start is the whole reason that decision went the way it did, and
            // persisting a second copy here would give the two somewhere to
            // disagree.
            | CoreEvent::SettingsChanged(_)
            // The agent roster (T220) is an observation of other processes,
            // rediscovered from scratch each launch. Persisting it would make
            // Gritty claim on startup that sessions are running which may have
            // ended while it was closed.
            | CoreEvent::AgentSessionsChanged(_)
            // A timeline (T230) is a bounded window onto a transcript the agent
            // owns and keeps on disk. Persisting it would duplicate the record
            // and, worse, put agent output — untrusted, routinely full of file
            // contents — into Gritty's database, which ADR-015 already refused
            // for terminal buffers on the same grounds.
            | CoreEvent::AgentTimelineAppended(_)
            // A run (T300) does not survive a restart, deliberately and for
            // now: it supervises something live, and a row claiming on startup
            // that a run is `RUNNING` when the process driving it died with the
            // app would be exactly the orphan the state machine exists to
            // prevent. ADR-021 decides what a real provider's runs persist,
            // before T310 gives one something worth losing.
            | CoreEvent::AgentRunChanged(_)
            | CoreEvent::AgentRunEventAppended(_)
            | CoreEvent::DecisionRequested(_)
            // A search result (T080) is a live answer about the tree as it is
            // right now. Persisting a batch would let a restart present stale
            // matches as current ones, and the operation model already refuses
            // to replay them (T080's "no result replay" non-goal).
            | CoreEvent::SearchMatchesFound(_)
            | CoreEvent::DecisionResolved(_) => None,
        }
    }
}

fn durable(operation: &Operation) -> Option<Intent> {
    operation
        .durable
        .then(|| Intent::Operation(Box::new(operation.clone())))
}

/// Everything the writer task receives.
#[derive(Debug)]
pub(super) enum WriterMessage {
    Intent(Intent),
    Window(WindowGeometry),
    /// Forget what was written (the database was replaced) and look at every
    /// open document again.
    Resync,
    /// Persist everything pending now and answer when done.
    Flush(oneshot::Sender<()>),
}

/// The receiving half handed to `SessionService::start`.
pub struct TapReceiver {
    pub(super) rx: mpsc::Receiver<WriterMessage>,
    pub(super) sender: mpsc::Sender<WriterMessage>,
    pub(super) overflowed: Arc<AtomicBool>,
}

pub struct SessionTap {
    tx: mpsc::Sender<WriterMessage>,
    overflowed: Arc<AtomicBool>,
}

impl SessionTap {
    /// A tap and its receiver; `capacity` bounds the intents in flight.
    pub fn new(capacity: usize) -> (Arc<Self>, TapReceiver) {
        let (tx, rx) = mpsc::channel(capacity);
        let overflowed = Arc::new(AtomicBool::new(false));
        let tap = Arc::new(Self {
            tx: tx.clone(),
            overflowed: Arc::clone(&overflowed),
        });
        (
            tap,
            TapReceiver {
                rx,
                sender: tx,
                overflowed,
            },
        )
    }
}

impl EventSink for SessionTap {
    fn emit(&self, envelope: &EventEnvelope) -> Result<(), CoreError> {
        let Some(intent) = Intent::from_event(&envelope.event) else {
            return Ok(());
        };
        match self.tx.try_send(WriterMessage::Intent(intent)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                if !self.overflowed.swap(true, Ordering::SeqCst) {
                    tracing::warn!(
                        seq = envelope.meta.seq,
                        "session intent channel is full; the writer will resynchronize"
                    );
                }
            }
            // No writer (persistence unavailable or shutting down): nothing to record.
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
        Ok(())
    }
}
