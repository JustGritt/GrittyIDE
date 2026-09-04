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
            | CoreEvent::DropHoverChanged(_) => None,
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
