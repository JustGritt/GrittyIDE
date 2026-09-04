//! The operation registry: creates operations, runs their bodies on the
//! shared tokio runtime, owns cancellation and emits `Operation*` events.
//!
//! Lifecycle guarantees (addendum §3.2):
//! - **cancel before start**: the operation becomes `CANCELLED` immediately and
//!   its body never runs;
//! - **cancel during execution**: the body's cancellation token fires, the body
//!   task is aborted at its next await point, and the operation ends `CANCELLED`;
//! - **events after cancellation** (progress) are dropped;
//! - **duplicate or late completion signals** are rejected by the state machine
//!   and logged, never emitted; once cancellation was requested, the only
//!   terminal state is `CANCELLED`;
//! - **body panics** are contained by running the body as its own task and end
//!   the operation `FAILED` with an `Internal` error;
//! - **shutdown** cancels every running operation through a root token.
//!
//! Renderer reload: the registry lives in the core, so `list()` rehydrates the
//! UI. Nothing is persisted (T060 decides what is).

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};
use ts_rs::TS;

use super::progress::Throttle;
use super::{Operation, OperationKind, OperationProgress, OperationStatus};
use crate::event::{
    CoreEvent, EventBus, EventOrigin, OperationCancelledPayload, OperationCompletedPayload,
    OperationFailedPayload, OperationProgressPayload, OperationStartedPayload,
};
use crate::protocol::actor::Actor;
use crate::protocol::error::CoreError;
use crate::protocol::ids::{CorrelationId, OperationId, WorkspaceId};
use crate::protocol::time::Timestamp;

/// How many finished operations `list()` keeps for rehydration.
pub const FINISHED_RETENTION: usize = 200;

/// How long the runner waits for an aborted body to stop before reporting
/// `CANCELLED` anyway (a body blocked without await points cannot be stopped).
const CANCEL_JOIN_GRACE: Duration = Duration::from_millis(250);

/// Argument of `operations.cancel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CancelOperationRequest {
    pub operation_id: OperationId,
}

/// Describes a new operation before it is registered.
#[derive(Debug, Clone)]
pub struct OperationSpec {
    pub kind: OperationKind,
    pub actor: Actor,
    pub workspace_id: Option<WorkspaceId>,
    pub correlation_id: CorrelationId,
    /// Persist the lifecycle so a process death is reconciled to `INTERRUPTED`
    /// at the next start (T060). Nothing sets it yet.
    pub durable: bool,
}

/// What a body returns: an optional JSON result, or the error that fails the operation.
pub type OperationResult = Result<Option<serde_json::Value>, CoreError>;

/// Handed to an operation body: identity, cancellation and progress reporting.
pub struct OperationContext {
    id: OperationId,
    token: CancellationToken,
    reporter: ProgressReporter,
}

impl OperationContext {
    pub fn id(&self) -> &OperationId {
        &self.id
    }

    /// Cooperative cancellation: bodies check this between units of work.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Resolves once cancellation was requested; use in `select!` with awaits.
    pub fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.token.cancelled()
    }

    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    pub fn reporter(&self) -> &ProgressReporter {
        &self.reporter
    }

    pub fn progress(&self, progress: OperationProgress) {
        self.reporter.report(progress);
    }

    pub fn report(&self, current: u64, total: Option<u64>, message: Option<String>) {
        self.progress(OperationProgress {
            current,
            total,
            message,
        });
    }
}

/// Cloneable, sendable progress handle. Reports after cancellation are dropped.
#[derive(Clone)]
pub struct ProgressReporter {
    registry: Arc<OperationRegistry>,
    id: OperationId,
    token: CancellationToken,
}

impl ProgressReporter {
    pub fn report(&self, progress: OperationProgress) {
        self.registry
            .report_progress(&self.id, &self.token, progress);
    }
}

enum Outcome {
    Completed(Option<serde_json::Value>),
    Failed(CoreError),
    Cancelled,
}

struct Entry {
    snapshot: Operation,
    token: CancellationToken,
    runner: Option<JoinHandle<()>>,
    throttle: Throttle,
}

impl Entry {
    fn origin(&self) -> EventOrigin {
        EventOrigin {
            workspace_id: self.snapshot.workspace_id.clone(),
            correlation_id: Some(self.snapshot.correlation_id.clone()),
            actor: self.snapshot.actor.clone(),
        }
    }
}

#[derive(Default)]
struct Inner {
    entries: HashMap<OperationId, Entry>,
    /// Finished ids in completion order, pruned to [`FINISHED_RETENTION`].
    finished: VecDeque<OperationId>,
}

pub struct OperationRegistry {
    inner: Mutex<Inner>,
    events: Arc<EventBus>,
    handle: Handle,
    /// Root token; every operation token is a child, so `shutdown` cancels all.
    shutdown: CancellationToken,
}

impl OperationRegistry {
    /// `handle` is the shared tokio runtime (Tauri's); the core never creates its own.
    pub fn new(handle: Handle, events: Arc<EventBus>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner::default()),
            events,
            handle,
            shutdown: CancellationToken::new(),
        })
    }

    /// Register an operation and schedule its body. Returns the `CREATED`
    /// snapshot; `OperationStarted` follows as soon as the body is scheduled.
    pub fn start<F, Fut>(self: &Arc<Self>, spec: OperationSpec, body: F) -> Operation
    where
        F: FnOnce(OperationContext) -> Fut + Send + 'static,
        Fut: Future<Output = OperationResult> + Send + 'static,
    {
        let id = OperationId::new();
        let token = self.shutdown.child_token();
        let snapshot = Operation {
            id: id.clone(),
            kind: spec.kind,
            actor: spec.actor,
            workspace_id: spec.workspace_id,
            status: OperationStatus::Created,
            created_at: Timestamp::now(),
            started_at: None,
            finished_at: None,
            cancellation_requested: false,
            correlation_id: spec.correlation_id,
            progress: None,
            error: None,
            durable: spec.durable,
        };
        self.lock().entries.insert(
            id.clone(),
            Entry {
                snapshot: snapshot.clone(),
                token: token.clone(),
                runner: None,
                throttle: Throttle::default(),
            },
        );
        tracing::debug!(operation = %id, kind = snapshot.kind.as_str(), "operation created");

        let runner = self
            .handle
            .spawn(Self::run(Arc::clone(self), id.clone(), token, body));
        if let Some(entry) = self.lock().entries.get_mut(&id) {
            entry.runner = Some(runner);
        }
        snapshot
    }

    async fn run<F, Fut>(this: Arc<Self>, id: OperationId, token: CancellationToken, body: F)
    where
        F: FnOnce(OperationContext) -> Fut + Send + 'static,
        Fut: Future<Output = OperationResult> + Send + 'static,
    {
        let Some(ctx) = this.begin(&id, &token) else {
            // Cancelled before it started: `cancel` already finished it.
            return;
        };
        // The body runs as its own task so a panic inside it is contained here.
        let mut body_task = this.handle.spawn(async move { body(ctx).await });
        let outcome = tokio::select! {
            biased;
            () = token.cancelled() => {
                body_task.abort();
                if tokio::time::timeout(CANCEL_JOIN_GRACE, &mut body_task).await.is_err() {
                    tracing::warn!(operation = %id, "operation body did not stop within the cancel grace period");
                }
                Outcome::Cancelled
            }
            joined = &mut body_task => match joined {
                Ok(Ok(result)) => Outcome::Completed(result),
                Ok(Err(error)) => Outcome::Failed(error),
                Err(join) if join.is_cancelled() => Outcome::Cancelled,
                Err(join) => Outcome::Failed(CoreError::internal(format!(
                    "operation body panicked: {}",
                    panic_message(join)
                ))),
            },
        };
        this.finish(&id, outcome);
    }

    /// `CREATED → RUNNING` and `OperationStarted`; `None` when already cancelled.
    fn begin(
        self: &Arc<Self>,
        id: &OperationId,
        token: &CancellationToken,
    ) -> Option<OperationContext> {
        let mut inner = self.lock();
        let entry = inner.entries.get_mut(id)?;
        if entry.snapshot.status != OperationStatus::Created {
            return None;
        }
        entry.snapshot.status = OperationStatus::Running;
        entry.snapshot.started_at = Some(Timestamp::now());
        tracing::info!(operation = %id, kind = entry.snapshot.kind.as_str(), "operation started");
        self.events.emit(
            &entry.origin(),
            CoreEvent::OperationStarted(OperationStartedPayload {
                operation: entry.snapshot.clone(),
            }),
        );
        Some(OperationContext {
            id: id.clone(),
            token: token.clone(),
            reporter: ProgressReporter {
                registry: Arc::clone(self),
                id: id.clone(),
                token: token.clone(),
            },
        })
    }

    /// Terminal transition. Rejects duplicates and anything after a terminal
    /// state; maps every outcome to `CANCELLED` once cancellation was requested.
    fn finish(&self, id: &OperationId, outcome: Outcome) {
        let mut inner = self.lock();
        let Some(entry) = inner.entries.get_mut(id) else {
            tracing::warn!(operation = %id, "ignoring completion of an unknown operation");
            return;
        };
        let (target, result, error) = match outcome {
            _ if entry.snapshot.cancellation_requested => (OperationStatus::Cancelled, None, None),
            Outcome::Completed(result) => (OperationStatus::Completed, result, None),
            Outcome::Failed(error) => (OperationStatus::Failed, None, Some(error)),
            Outcome::Cancelled => (OperationStatus::Cancelled, None, None),
        };
        let status = match entry.snapshot.status.transition(target) {
            Ok(status) => status,
            Err(invalid) => {
                tracing::warn!(operation = %id, %invalid, "ignoring stale operation completion");
                return;
            }
        };
        entry.snapshot.status = status;
        entry.snapshot.finished_at = Some(Timestamp::now());
        entry.snapshot.error = error.map(|e| e.or_correlation(&entry.snapshot.correlation_id));
        tracing::info!(operation = %id, ?status, "operation finished");
        let event = match status {
            OperationStatus::Completed => {
                CoreEvent::OperationCompleted(OperationCompletedPayload {
                    operation: entry.snapshot.clone(),
                    result,
                })
            }
            OperationStatus::Failed => CoreEvent::OperationFailed(OperationFailedPayload {
                operation: entry.snapshot.clone(),
                error: entry
                    .snapshot
                    .error
                    .clone()
                    .unwrap_or_else(|| CoreError::internal("operation failed")),
            }),
            _ => CoreEvent::OperationCancelled(OperationCancelledPayload {
                operation: entry.snapshot.clone(),
            }),
        };
        self.events.emit(&entry.origin(), event);
        inner.finished.push_back(id.clone());
        Self::prune(&mut inner);
    }

    fn prune(inner: &mut Inner) {
        while inner.finished.len() > FINISHED_RETENTION {
            if let Some(old) = inner.finished.pop_front() {
                inner.entries.remove(&old);
            }
        }
    }

    fn report_progress(
        &self,
        id: &OperationId,
        token: &CancellationToken,
        progress: OperationProgress,
    ) {
        if token.is_cancelled() {
            tracing::trace!(operation = %id, "dropping progress after cancellation");
            return;
        }
        let mut inner = self.lock();
        let Some(entry) = inner.entries.get_mut(id) else {
            return;
        };
        if entry.snapshot.status != OperationStatus::Running {
            return;
        }
        entry.snapshot.progress = Some(progress.clone());
        if entry.throttle.allow(Instant::now()) {
            self.events.emit(
                &entry.origin(),
                CoreEvent::OperationProgress(OperationProgressPayload {
                    operation_id: id.clone(),
                    progress,
                }),
            );
        }
    }

    /// Request cancellation. Idempotent; unknown ids are `NotFound`.
    pub fn cancel(&self, id: &OperationId) -> Result<Operation, CoreError> {
        let mut inner = self.lock();
        let Some(entry) = inner.entries.get_mut(id) else {
            return Err(CoreError::not_found(format!(
                "operation {id} does not exist"
            )));
        };
        match entry.snapshot.status {
            status if status.is_terminal() => Ok(entry.snapshot.clone()),
            OperationStatus::Created => {
                entry.snapshot.cancellation_requested = true;
                entry.snapshot.status = OperationStatus::Cancelled;
                entry.snapshot.finished_at = Some(Timestamp::now());
                entry.token.cancel();
                tracing::info!(operation = %id, "operation cancelled before it started");
                let snapshot = entry.snapshot.clone();
                self.events.emit(
                    &entry.origin(),
                    CoreEvent::OperationCancelled(OperationCancelledPayload {
                        operation: snapshot.clone(),
                    }),
                );
                inner.finished.push_back(id.clone());
                Self::prune(&mut inner);
                Ok(snapshot)
            }
            _ => {
                if !entry.snapshot.cancellation_requested {
                    entry.snapshot.cancellation_requested = true;
                    entry.token.cancel();
                    tracing::info!(operation = %id, "operation cancellation requested");
                }
                Ok(entry.snapshot.clone())
            }
        }
    }

    pub fn get(&self, id: &OperationId) -> Option<Operation> {
        self.lock()
            .entries
            .get(id)
            .map(|entry| entry.snapshot.clone())
    }

    /// Every known operation (running and recently finished) in creation order.
    pub fn list(&self) -> Vec<Operation> {
        let mut operations: Vec<Operation> = self
            .lock()
            .entries
            .values()
            .map(|entry| entry.snapshot.clone())
            .collect();
        operations.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        operations
    }

    /// Cancel every running operation and wait up to `grace` for their runners.
    pub async fn shutdown(&self, grace: Duration) {
        self.shutdown.cancel();
        let runners: Vec<(OperationId, JoinHandle<()>)> = {
            let mut inner = self.lock();
            inner
                .entries
                .iter_mut()
                .filter_map(|(id, entry)| entry.runner.take().map(|runner| (id.clone(), runner)))
                .collect()
        };
        let deadline = Instant::now() + grace;
        for (id, runner) in runners {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if tokio::time::timeout(remaining, runner).await.is_err() {
                tracing::warn!(operation = %id, "operation did not stop before shutdown deadline");
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn panic_message(join: tokio::task::JoinError) -> String {
    let payload = join.into_panic();
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use tokio::sync::{mpsc, oneshot};
    use tokio::time::timeout;

    use super::*;
    use crate::event::EventEnvelope;
    use crate::event::testing::RecordingSink;
    use crate::protocol::error::CoreErrorCode;

    struct Harness {
        registry: Arc<OperationRegistry>,
        sink: Arc<RecordingSink>,
        rx: mpsc::UnboundedReceiver<EventEnvelope>,
    }

    fn harness() -> Harness {
        let (sink, rx) = RecordingSink::with_channel();
        let events = EventBus::new(sink.clone());
        let registry = OperationRegistry::new(Handle::current(), events);
        Harness { registry, sink, rx }
    }

    fn spec() -> OperationSpec {
        OperationSpec {
            kind: OperationKind::new("test.op"),
            actor: Actor::User,
            workspace_id: None,
            correlation_id: CorrelationId::new(),
            durable: false,
        }
    }

    impl Harness {
        async fn next_named(&mut self, name: &str) -> EventEnvelope {
            timeout(Duration::from_secs(5), async {
                loop {
                    let envelope = self.rx.recv().await.expect("event stream open");
                    if envelope.event.name() == name {
                        return envelope;
                    }
                }
            })
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {name}"))
        }

        /// Let queued tasks run, then return every event emitted so far.
        async fn settle(&mut self) -> Vec<&'static str> {
            for _ in 0..20 {
                tokio::task::yield_now().await;
            }
            self.sink.names()
        }
    }

    #[tokio::test]
    async fn body_result_completes_the_operation() {
        let mut h = harness();
        let created = h.registry.start(spec(), |_ctx| async {
            Ok(Some(serde_json::json!({ "answer": 42 })))
        });
        assert_eq!(created.status, OperationStatus::Created);
        assert!(created.id.is_well_formed());

        let started = h.next_named("OperationStarted").await;
        assert_eq!(
            started.meta.correlation_id,
            Some(created.correlation_id.clone())
        );
        let done = h.next_named("OperationCompleted").await;
        let CoreEvent::OperationCompleted(payload) = done.event else {
            panic!("expected completion");
        };
        assert_eq!(payload.result, Some(serde_json::json!({ "answer": 42 })));
        assert_eq!(payload.operation.status, OperationStatus::Completed);
        assert!(payload.operation.finished_at.is_some());
        assert!(started.meta.seq < done.meta.seq);

        let listed = h.registry.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].status, OperationStatus::Completed);
    }

    #[tokio::test]
    async fn cancel_before_start_is_immediate_and_body_never_runs() {
        let mut h = harness();
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        let created = h.registry.start(spec(), move |_ctx| async move {
            flag.store(true, Ordering::SeqCst);
            Ok(None)
        });
        // The runner task has not been polled yet on this current-thread runtime.
        let cancelled = h.registry.cancel(&created.id).unwrap();
        assert_eq!(cancelled.status, OperationStatus::Cancelled);
        assert!(cancelled.cancellation_requested);
        assert!(cancelled.finished_at.is_some());
        assert_eq!(h.settle().await, vec!["OperationCancelled"]);
        assert!(!ran.load(Ordering::SeqCst));
        assert_eq!(
            h.registry.get(&created.id).unwrap().status,
            OperationStatus::Cancelled
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_during_execution_ends_cancelled_within_bound() {
        let mut h = harness();
        let created = h.registry.start(spec(), |ctx| async move {
            for step in 0..1_000u64 {
                tokio::time::sleep(Duration::from_millis(10)).await;
                ctx.report(step, Some(1_000), None);
            }
            Ok(Some(serde_json::json!("should never complete")))
        });
        h.next_named("OperationProgress").await;

        let snapshot = h.registry.cancel(&created.id).unwrap();
        assert!(snapshot.cancellation_requested);
        let cancelled = timeout(
            Duration::from_millis(500),
            h.next_named("OperationCancelled"),
        )
        .await
        .expect("cancelled within the bound");
        let CoreEvent::OperationCancelled(payload) = cancelled.event else {
            panic!("expected cancellation");
        };
        assert_eq!(payload.operation.status, OperationStatus::Cancelled);

        tokio::time::sleep(Duration::from_secs(1)).await;
        let names = h.settle().await;
        assert!(!names.contains(&"OperationCompleted"), "{names:?}");
        assert_eq!(names.last(), Some(&"OperationCancelled"));
        assert_eq!(
            h.registry.get(&created.id).unwrap().status,
            OperationStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn cancel_unknown_operation_is_not_found() {
        let h = harness();
        let error = h
            .registry
            .cancel(&OperationId::from_raw("nope"))
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::NotFound);
    }

    #[tokio::test]
    async fn cancel_is_idempotent_and_ignored_after_completion() {
        let mut h = harness();
        let (release, gate) = oneshot::channel::<()>();
        let created = h.registry.start(spec(), |_ctx| async move {
            let _ = gate.await;
            Ok(None)
        });
        h.next_named("OperationStarted").await;
        let first = h.registry.cancel(&created.id).unwrap();
        let second = h.registry.cancel(&created.id).unwrap();
        assert!(first.cancellation_requested && second.cancellation_requested);
        h.next_named("OperationCancelled").await;
        let third = h.registry.cancel(&created.id).unwrap();
        assert_eq!(third.status, OperationStatus::Cancelled);
        drop(release);
        let names = h.settle().await;
        assert_eq!(
            names.iter().filter(|n| **n == "OperationCancelled").count(),
            1
        );

        let done = h.registry.start(spec(), |_ctx| async { Ok(None) });
        h.next_named("OperationCompleted").await;
        let after = h.registry.cancel(&done.id).unwrap();
        assert_eq!(after.status, OperationStatus::Completed);
        assert!(!after.cancellation_requested);
        assert_eq!(
            h.settle()
                .await
                .iter()
                .filter(|n| **n == "OperationCancelled")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn duplicate_and_stale_completions_are_ignored() {
        let mut h = harness();
        let created = h.registry.start(spec(), |_ctx| async { Ok(None) });
        h.next_named("OperationCompleted").await;
        h.registry.finish(&created.id, Outcome::Completed(None));
        h.registry
            .finish(&created.id, Outcome::Failed(CoreError::internal("late")));
        let names = h.settle().await;
        assert_eq!(names, vec!["OperationStarted", "OperationCompleted"]);
        let snapshot = h.registry.get(&created.id).unwrap();
        assert_eq!(snapshot.status, OperationStatus::Completed);
        assert!(snapshot.error.is_none());
    }

    #[tokio::test]
    async fn completion_after_cancellation_request_is_reported_as_cancelled() {
        let mut h = harness();
        let (release, gate) = oneshot::channel::<()>();
        let created = h.registry.start(spec(), |_ctx| async move {
            let _ = gate.await;
            Ok(Some(serde_json::json!("done")))
        });
        h.next_named("OperationStarted").await;
        // Cancellation is requested but the runner has not observed it yet; a
        // completion signal racing in at this point must not win.
        h.registry.cancel(&created.id).unwrap();
        h.registry.finish(
            &created.id,
            Outcome::Completed(Some(serde_json::json!("done"))),
        );
        assert_eq!(
            h.registry.get(&created.id).unwrap().status,
            OperationStatus::Cancelled
        );
        drop(release);
        let names = h.settle().await;
        assert_eq!(names, vec!["OperationStarted", "OperationCancelled"]);
    }

    #[tokio::test]
    async fn progress_after_cancellation_is_dropped() {
        let mut h = harness();
        let (send_reporter, reporter) = oneshot::channel::<ProgressReporter>();
        let (_release, gate) = oneshot::channel::<()>();
        let created = h.registry.start(spec(), move |ctx| async move {
            let _ = send_reporter.send(ctx.reporter().clone());
            let _ = gate.await;
            Ok(None)
        });
        let reporter = reporter.await.unwrap();
        h.next_named("OperationStarted").await;
        reporter.report(OperationProgress {
            current: 1,
            total: None,
            message: None,
        });
        h.next_named("OperationProgress").await;
        h.registry.cancel(&created.id).unwrap();
        h.next_named("OperationCancelled").await;
        reporter.report(OperationProgress {
            current: 2,
            total: None,
            message: None,
        });
        let names = h.settle().await;
        assert_eq!(
            names.iter().filter(|n| **n == "OperationProgress").count(),
            1
        );
        assert_eq!(
            h.registry
                .get(&created.id)
                .unwrap()
                .progress
                .unwrap()
                .current,
            1
        );
    }

    #[tokio::test(start_paused = true)]
    async fn progress_events_are_throttled_but_snapshot_is_current() {
        let mut h = harness();
        let created = h.registry.start(spec(), |ctx| async move {
            for i in 0..1_000u64 {
                ctx.report(i, Some(1_000), None);
            }
            tokio::time::sleep(Duration::from_millis(60)).await;
            ctx.report(1_000, Some(1_000), Some("done".into()));
            Ok(None)
        });
        h.next_named("OperationCompleted").await;
        let names = h.sink.names();
        assert_eq!(
            names.iter().filter(|n| **n == "OperationProgress").count(),
            2,
            "{names:?}"
        );
        let snapshot = h.registry.get(&created.id).unwrap();
        assert_eq!(snapshot.progress.unwrap().current, 1_000);
    }

    #[tokio::test]
    async fn body_panic_fails_the_operation_with_internal_error() {
        let mut h = harness();
        let created = h.registry.start(spec(), |_ctx| async {
            panic!("kaboom");
        });
        let failed = h.next_named("OperationFailed").await;
        let CoreEvent::OperationFailed(payload) = failed.event else {
            panic!("expected failure");
        };
        assert_eq!(payload.error.code, CoreErrorCode::Internal);
        assert!(payload.error.message.contains("kaboom"));
        assert_eq!(payload.error.correlation_id, Some(created.correlation_id));
        assert_eq!(
            h.registry.get(&created.id).unwrap().status,
            OperationStatus::Failed
        );
    }

    #[tokio::test]
    async fn body_error_fails_the_operation_with_that_error() {
        let mut h = harness();
        let created = h.registry.start(spec(), |_ctx| async {
            Err(CoreError::not_found("nothing here"))
        });
        h.next_named("OperationFailed").await;
        let snapshot = h.registry.get(&created.id).unwrap();
        assert_eq!(snapshot.status, OperationStatus::Failed);
        assert_eq!(snapshot.error.unwrap().code, CoreErrorCode::NotFound);
    }

    #[tokio::test]
    async fn shutdown_cancels_running_operations() {
        let mut h = harness();
        let (_release, gate) = oneshot::channel::<()>();
        let created = h.registry.start(spec(), |_ctx| async move {
            let _ = gate.await;
            Ok(None)
        });
        h.next_named("OperationStarted").await;
        timeout(
            Duration::from_secs(2),
            h.registry.shutdown(Duration::from_secs(1)),
        )
        .await
        .expect("shutdown completes");
        assert_eq!(
            h.registry.get(&created.id).unwrap().status,
            OperationStatus::Cancelled
        );
        assert_eq!(
            h.sink.names(),
            vec!["OperationStarted", "OperationCancelled"]
        );
    }

    #[tokio::test]
    async fn list_is_ordered_and_bounded() {
        let mut h = harness();
        let mut ids = Vec::new();
        for _ in 0..(FINISHED_RETENTION + 5) {
            ids.push(h.registry.start(spec(), |_ctx| async { Ok(None) }).id);
        }
        for _ in 0..(FINISHED_RETENTION + 5) {
            h.next_named("OperationCompleted").await;
        }
        let listed = h.registry.list();
        assert_eq!(listed.len(), FINISHED_RETENTION);
        let listed_ids: Vec<_> = listed.iter().map(|op| op.id.clone()).collect();
        assert_eq!(listed_ids, ids[5..].to_vec());
        assert!(h.registry.get(&ids[0]).is_none());
    }
}
