//! The writer task: one tokio task that turns intents into database writes.
//!
//! Every database call runs on the blocking pool, one at a time, in intent
//! order. Dirty-buffer snapshots are debounced per document (`debounce`) with
//! an upper bound while typing continues (`max_wait`), and taken from the
//! core's buffer at write time, never from an event payload. A failing write
//! keeps its job at the head of the queue and retries with exponential
//! backoff; the status flips to unhealthy once per failure streak and back
//! to healthy on the first success, each announced by
//! `PersistenceStatusChanged`.
//!
//! Rows are only ever deleted for documents this process *owns* (it wrote
//! their row, or recovered them from one) or after an explicit discard (a
//! dirty document closed with `force`, a workspace closed with `force`): a
//! snapshot left by a previous process is a recovery candidate until the user
//! answers, even if the same path is opened clean meanwhile. A workspace whose
//! root disappeared keeps its rows, too.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gritty_persistence::store::{operations, session, snapshots, window, workspace};
use gritty_persistence::{Database, PersistenceError, now_ms};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};

use super::tap::{Intent, WriterMessage};
use super::wire::{PersistenceStatusPayload, SessionNotice};
use super::{Status, WindowGeometry};
use crate::document::{DocumentClosedReason, DocumentManager};
use crate::event::{CoreEvent, EventBus, EventOrigin};
use crate::operation::{Operation, OperationStatus};
use crate::protocol::actor::Actor;
use crate::protocol::ids::{CorrelationId, DocumentId, WorkspaceId};
use crate::workspace::{WorkspaceClosedReason, WorkspaceManager};

/// Timings of the writer; tests shorten them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriterConfig {
    /// Quiet time after the last change before a snapshot is written.
    pub debounce: Duration,
    /// Longest a dirty buffer stays unwritten while changes keep coming.
    pub max_wait: Duration,
    pub window_debounce: Duration,
    pub backoff_base: Duration,
    pub backoff_cap: Duration,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_secs(1),
            max_wait: Duration::from_secs(2),
            window_debounce: Duration::from_millis(500),
            backoff_base: Duration::from_secs(1),
            backoff_cap: Duration::from_secs(30),
        }
    }
}

struct Pending {
    due: Instant,
    deadline: Instant,
}

struct Backoff {
    attempt: u32,
    until: Instant,
}

#[derive(Debug, Clone)]
enum Job {
    Snapshot(DocumentId),
    DeleteSnapshot {
        root: String,
        path: String,
    },
    WorkspaceOpened {
        root: String,
    },
    ClearWorkspaceRoot,
    Operation {
        operation: Box<Operation>,
        root: Option<String>,
    },
    Window(WindowGeometry),
}

pub(super) struct Writer {
    config: WriterConfig,
    handle: Handle,
    database: Arc<Database>,
    documents: Arc<DocumentManager>,
    workspaces: Arc<WorkspaceManager>,
    events: Arc<EventBus>,
    status: Arc<Status>,
    overflowed: Arc<AtomicBool>,
    roots: HashMap<WorkspaceId, String>,
    closed_reasons: HashMap<WorkspaceId, WorkspaceClosedReason>,
    pending: HashMap<DocumentId, Pending>,
    pending_window: Option<(WindowGeometry, Instant)>,
    /// Documents whose row this process may delete: written here, or recovered.
    owned: HashSet<DocumentId>,
    /// `currentVersion` last written per document; a matching version is skipped.
    last_written: HashMap<DocumentId, u64>,
    queue: VecDeque<Job>,
    backoff: Option<Backoff>,
}

enum Wake {
    Message(WriterMessage),
    Closed,
    Timer,
}

impl Writer {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        config: WriterConfig,
        handle: Handle,
        database: Arc<Database>,
        documents: Arc<DocumentManager>,
        workspaces: Arc<WorkspaceManager>,
        events: Arc<EventBus>,
        status: Arc<Status>,
        overflowed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            config,
            handle,
            database,
            documents,
            workspaces,
            events,
            status,
            overflowed,
            roots: HashMap::new(),
            closed_reasons: HashMap::new(),
            pending: HashMap::new(),
            pending_window: None,
            owned: HashSet::new(),
            last_written: HashMap::new(),
            queue: VecDeque::new(),
            backoff: None,
        }
    }

    pub(super) async fn run(mut self, mut rx: mpsc::Receiver<WriterMessage>) {
        loop {
            if self.overflowed.swap(false, Ordering::SeqCst) {
                self.resync();
            }
            let deadline = self.next_deadline();
            let wake = tokio::select! {
                message = rx.recv() => message.map_or(Wake::Closed, Wake::Message),
                () = wait_until(deadline) => Wake::Timer,
            };
            match wake {
                Wake::Message(message) => self.handle(message).await,
                Wake::Closed => {
                    self.flush_all().await;
                    tracing::debug!("session writer stopped");
                    return;
                }
                Wake::Timer => {}
            }
            self.collect_due(Instant::now());
            self.run_queue(false).await;
        }
    }

    async fn handle(&mut self, message: WriterMessage) {
        match message {
            WriterMessage::Intent(intent) => self.intent(intent),
            WriterMessage::Window(geometry) => {
                self.pending_window =
                    Some((geometry, Instant::now() + self.config.window_debounce));
            }
            WriterMessage::Resync => {
                // The rows are gone with the old file; nothing is owned any more.
                self.last_written.clear();
                self.owned.clear();
                self.resync();
            }
            WriterMessage::Flush(reply) => {
                self.flush_all().await;
                let _ = reply.send(());
            }
        }
    }

    fn intent(&mut self, intent: Intent) {
        let now = Instant::now();
        match intent {
            Intent::DocumentTouched(id) => self.schedule(id, now),
            Intent::DocumentRecovered(id) => {
                self.owned.insert(id.clone());
                self.schedule(id, now);
            }
            Intent::DocumentClosed {
                id,
                workspace_id,
                path,
                dirty,
                reason,
            } => {
                self.pending.remove(&id);
                self.last_written.remove(&id);
                let written = self.owned.remove(&id);
                let root_missing = reason == DocumentClosedReason::WorkspaceClosed
                    && self.closed_reasons.get(&workspace_id)
                        == Some(&WorkspaceClosedReason::RootMissing);
                let discard = dirty && !root_missing;
                if discard || (!dirty && written) {
                    if let Some(root) = self.root_for(&workspace_id) {
                        self.queue.push_back(Job::DeleteSnapshot {
                            root,
                            path: path.to_string(),
                        });
                    }
                } else if dirty {
                    tracing::info!(document = %id, %path, "keeping the recovery snapshot of a document whose root disappeared");
                }
            }
            Intent::WorkspaceOpened { id, root } => {
                self.roots.insert(id, root.clone());
                self.queue.push_back(Job::WorkspaceOpened { root });
            }
            Intent::WorkspaceClosed { id, reason } => {
                self.closed_reasons.insert(id, reason);
                if reason == WorkspaceClosedReason::User {
                    self.queue.push_back(Job::ClearWorkspaceRoot);
                }
            }
            Intent::Operation(operation) => {
                let root = operation
                    .workspace_id
                    .as_ref()
                    .and_then(|id| self.root_for(id));
                self.queue.push_back(Job::Operation { operation, root });
            }
        }
    }

    /// Debounce: a change moves `due`; `deadline` is fixed by the first change.
    fn schedule(&mut self, id: DocumentId, now: Instant) {
        let due = now + self.config.debounce;
        match self.pending.get_mut(&id) {
            Some(pending) => pending.due = due,
            None => {
                self.pending.insert(
                    id,
                    Pending {
                        due,
                        deadline: now + self.config.max_wait,
                    },
                );
            }
        }
    }

    /// The channel overflowed or the database was replaced: look at every
    /// open document again instead of trusting what was seen so far.
    fn resync(&mut self) {
        let now = Instant::now();
        let documents = self.documents.list();
        tracing::info!(
            documents = documents.len(),
            "session writer resynchronizing"
        );
        for document in documents {
            self.schedule(document.id, now);
        }
    }

    fn root_for(&mut self, workspace_id: &WorkspaceId) -> Option<String> {
        if let Some(root) = self.roots.get(workspace_id) {
            return Some(root.clone());
        }
        // Missed the `WorkspaceOpened` intent (overflow): ask the manager.
        let current = self.workspaces.current(&CorrelationId::new())?;
        if current.id != *workspace_id {
            return None;
        }
        self.roots
            .insert(workspace_id.clone(), current.root.clone());
        Some(current.root)
    }

    fn next_deadline(&self) -> Option<Instant> {
        let mut next: Option<Instant> = None;
        let mut consider = |candidate: Instant| {
            next = Some(next.map_or(candidate, |current| current.min(candidate)));
        };
        for pending in self.pending.values() {
            consider(pending.due.min(pending.deadline));
        }
        if let Some((_, due)) = &self.pending_window {
            consider(*due);
        }
        if !self.queue.is_empty() {
            match &self.backoff {
                Some(backoff) => consider(backoff.until),
                None => consider(Instant::now()),
            }
        }
        next
    }

    fn collect_due(&mut self, now: Instant) {
        let due: Vec<DocumentId> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.due <= now || pending.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.pending.remove(&id);
            self.queue.push_back(Job::Snapshot(id));
        }
        if self
            .pending_window
            .as_ref()
            .is_some_and(|(_, due)| *due <= now)
            && let Some((geometry, _)) = self.pending_window.take()
        {
            self.queue.push_back(Job::Window(geometry));
        }
    }

    async fn flush_all(&mut self) {
        let ids: Vec<DocumentId> = self.pending.drain().map(|(id, _)| id).collect();
        for id in ids {
            self.queue.push_back(Job::Snapshot(id));
        }
        if let Some((geometry, _)) = self.pending_window.take() {
            self.queue.push_back(Job::Window(geometry));
        }
        self.run_queue(true).await;
    }

    async fn run_queue(&mut self, force: bool) {
        while let Some(job) = self.queue.pop_front() {
            if !force
                && let Some(backoff) = &self.backoff
                && Instant::now() < backoff.until
            {
                self.queue.push_front(job);
                return;
            }
            match self.execute(&job).await {
                Ok(()) => {
                    if self.backoff.take().is_some() {
                        self.status.set_healthy();
                        tracing::info!("session database writes succeed again");
                        self.emit_status(None);
                    }
                }
                Err(error) => {
                    self.queue.push_front(job);
                    let attempt = self.backoff.as_ref().map_or(0, |b| b.attempt) + 1;
                    let delay = self
                        .config
                        .backoff_base
                        .saturating_mul(1u32 << (attempt - 1).min(16))
                        .min(self.config.backoff_cap);
                    self.backoff = Some(Backoff {
                        attempt,
                        until: Instant::now() + delay,
                    });
                    tracing::warn!(
                        %error,
                        attempt,
                        retry_in_ms = delay.as_millis() as u64,
                        queued = self.queue.len(),
                        "session database write failed; buffers stay in memory"
                    );
                    if attempt == 1 {
                        self.status.set_unhealthy(error.to_string());
                        self.emit_status(None);
                    }
                    return;
                }
            }
        }
    }

    async fn execute(&mut self, job: &Job) -> Result<(), PersistenceError> {
        let db = Arc::clone(&self.database);
        let now = now_ms();
        match job {
            Job::Snapshot(id) => {
                let Some(snapshot) = self.documents.snapshot(id) else {
                    return Ok(());
                };
                let Some(root) = self.root_for(&snapshot.workspace_id) else {
                    tracing::warn!(document = %id, "no root known for the document's workspace; snapshot skipped");
                    return Ok(());
                };
                if snapshot.dirty {
                    let Some(text) = snapshot.text else {
                        return Ok(());
                    };
                    let version = snapshot.current_version;
                    if self.last_written.get(id) == Some(&version) {
                        return Ok(());
                    }
                    let row = snapshots::SnapshotRow {
                        path: snapshot.path.to_string(),
                        text,
                        encoding: snapshot.encoding.to_string(),
                        saved_hash: snapshot.saved_hash.to_string(),
                        current_version: i64::try_from(version).unwrap_or(i64::MAX),
                        updated_at: now,
                    };
                    let bytes = row.text.len();
                    self.blocking(move || db.write(|tx| snapshots::upsert(tx, &root, &row)))
                        .await?;
                    self.owned.insert(id.clone());
                    self.last_written.insert(id.clone(), version);
                    tracing::debug!(document = %id, path = %snapshot.path, version, bytes, "recovery snapshot written");
                } else if self.owned.contains(id) {
                    let path = snapshot.path.to_string();
                    self.blocking(move || {
                        db.write(|tx| snapshots::delete(tx, &root, &path).map(|_| ()))
                    })
                    .await?;
                    self.owned.remove(id);
                    self.last_written.remove(id);
                    tracing::debug!(document = %id, path = %snapshot.path, "recovery snapshot removed (buffer clean)");
                }
                Ok(())
            }
            Job::DeleteSnapshot { root, path } => {
                let (root_key, path_key) = (root.clone(), path.clone());
                let removed = self
                    .blocking(move || db.write(|tx| snapshots::delete(tx, &root_key, &path_key)))
                    .await?;
                if removed {
                    tracing::debug!(%path, "recovery snapshot removed (document closed)");
                }
                Ok(())
            }
            Job::WorkspaceOpened { root } => {
                let root = root.clone();
                self.blocking(move || {
                    db.write(|tx| {
                        workspace::touch(tx, &root, now)?;
                        session::set_workspace_root(tx, Some(&root), now)
                    })
                })
                .await
            }
            Job::ClearWorkspaceRoot => {
                self.blocking(move || db.write(|tx| session::set_workspace_root(tx, None, now)))
                    .await
            }
            Job::Operation { operation, root } => {
                let row = operation_row(operation, root.clone(), now);
                self.blocking(move || db.write(|tx| operations::upsert(tx, &row)))
                    .await
            }
            Job::Window(geometry) => {
                let row = window::WindowRow {
                    label: geometry.label.clone(),
                    x: geometry.x.map(i64::from),
                    y: geometry.y.map(i64::from),
                    width: i64::from(geometry.width),
                    height: i64::from(geometry.height),
                    scale_factor: geometry.scale_factor,
                    maximized: geometry.maximized,
                    updated_at: now,
                };
                self.blocking(move || db.write(|tx| window::save(tx, &row)))
                    .await
            }
        }
    }

    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, PersistenceError> + Send + 'static,
    ) -> Result<T, PersistenceError> {
        self.handle
            .spawn_blocking(work)
            .await
            .map_err(|join| PersistenceError::Io {
                context: "session writer task".to_owned(),
                source: io::Error::other(join.to_string()),
            })?
    }

    fn emit_status(&self, notice: Option<SessionNotice>) {
        emit_status(&self.events, &self.status, notice);
    }
}

pub(super) fn emit_status(events: &EventBus, status: &Status, notice: Option<SessionNotice>) {
    events.emit(
        &EventOrigin {
            workspace_id: None,
            correlation_id: None,
            actor: Actor::System,
        },
        CoreEvent::PersistenceStatusChanged(PersistenceStatusPayload {
            status: status.snapshot(),
            notice,
        }),
    );
}

async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => std::future::pending::<()>().await,
    }
}

fn operation_row(
    operation: &Operation,
    root: Option<String>,
    now: i64,
) -> operations::OperationRow {
    let status = match operation.status {
        OperationStatus::Created => "CREATED",
        OperationStatus::Running => "RUNNING",
        OperationStatus::Completed => "COMPLETED",
        OperationStatus::Failed => "FAILED",
        OperationStatus::Cancelled => "CANCELLED",
        OperationStatus::Interrupted => operations::INTERRUPTED,
    };
    let millis = |timestamp: crate::protocol::time::Timestamp| {
        i64::try_from(timestamp.as_millis()).unwrap_or(i64::MAX)
    };
    operations::OperationRow {
        id: operation.id.to_string(),
        kind: operation.kind.as_str().to_owned(),
        workspace_root: root,
        status: status.to_owned(),
        correlation_id: operation.correlation_id.to_string(),
        created_at: millis(operation.created_at),
        started_at: operation.started_at.map(millis),
        finished_at: operation.finished_at.map(millis),
        updated_at: now,
    }
}
