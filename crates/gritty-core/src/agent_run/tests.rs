//! Supervision and cancellation, exercised for real (T300-CORE, §§12-15).
//!
//! There is no `sleep` anywhere in an assertion path. A run is driven one step
//! at a time by a script the test holds, each step acknowledged before the next
//! is sent, and a run's ending is awaited on the **event stream** rather than
//! by waiting out a duration — which is what makes the acceptance criterion's
//! "run twice, identical" meaningful rather than lucky.
//!
//! The driver here is this module's own, not `gritty-agent-fake`'s: the crate
//! that will drive real scripts is T300-FAKE's and it must be free to have its
//! own step vocabulary. What is proved here is the supervisor's half — that a
//! run ends, exactly once, whatever its driver does or fails to do.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use gritty_persistence::testing::TempDir;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

use super::manager::{
    AgentRunContext, AgentRunManager, AgentRunSpec, MAX_CONCURRENT_RUNS, MAX_QUEUED_PROMPTS,
    OwnedProviderSession, RunInbox,
};
use super::state::AgentRunState;
use super::wire::{
    AgentRun, AgentRunActivity, AgentRunActivityKind, AgentRunEvent, AgentRunProvider, MAX_TEXT,
};
use crate::event::testing::RecordingSink;
use crate::event::{CoreEvent, EventBus, EventEnvelope};
use crate::operation::OperationRegistry;
use crate::protocol::actor::Actor;
use crate::protocol::error::{CoreError, CoreErrorCode};
use crate::protocol::ids::{AgentRunId, CorrelationId, WorkspaceId, WorkstreamId};
use crate::protocol::time::Timestamp;
use crate::workspace::{UnsavedPolicy, Workspace, WorkspaceManager};

/// One thing done to a run's context from inside its own driver task.
type Act = Box<dyn FnOnce(&AgentRunContext) -> Result<(), CoreError> + Send>;

/// What the driver's future is, once boxed so a closure can return it.
type Driven = std::pin::Pin<Box<dyn Future<Output = Result<(), CoreError>> + Send>>;

/// Everything the script can ask its run's driver to do.
enum Command {
    /// Do this to the context and say what happened.
    Act(Act, oneshot::Sender<Result<(), CoreError>>),
    /// Await something that never resolves. Only an abort ends this.
    Park,
    /// Die inside the driver's own task.
    Explode,
    /// Return `Ok(())` without ending the run — the orphan case.
    GiveUp,
}

/// The test's end of the driver.
struct Script {
    tx: mpsc::UnboundedSender<Command>,
}

impl Script {
    /// Run one step inside the driver's task and wait for its answer, so the
    /// next assertion happens strictly after this step took effect.
    async fn act(
        &self,
        act: impl FnOnce(&AgentRunContext) -> Result<(), CoreError> + Send + 'static,
    ) -> Result<(), CoreError> {
        let (ack, answered) = oneshot::channel();
        self.tx
            .send(Command::Act(Box::new(act), ack))
            .expect("the driver is still listening");
        timeout(Duration::from_secs(5), answered)
            .await
            .expect("the driver answers its script")
            .expect("the driver did not drop the acknowledgement")
    }

    async fn to(&self, state: AgentRunState) -> Result<(), CoreError> {
        self.act(move |ctx| ctx.transition(state).map(|_| ())).await
    }

    async fn advance(
        &self,
        state: AgentRunState,
        activity: AgentRunActivityKind,
    ) -> Result<(), CoreError> {
        self.act(move |ctx| {
            ctx.advance(
                state,
                Some(AgentRunActivity::new(activity, Timestamp::from_millis(1))),
            )
            .map(|_| ())
        })
        .await
    }

    async fn record(&self, event: AgentRunEvent) {
        self.act(move |ctx| {
            ctx.record(event);
            Ok(())
        })
        .await
        .expect("recording never fails");
    }

    async fn finish(&self, summary: &str) -> Result<(), CoreError> {
        let summary = summary.to_owned();
        self.act(move |ctx| ctx.finish(Some(summary)).map(|_| ()))
            .await
    }

    async fn fail(&self, message: &str) -> Result<(), CoreError> {
        let message = message.to_owned();
        self.act(move |ctx| ctx.fail(CoreError::internal(message)).map(|_| ()))
            .await
    }

    fn park(&self) {
        let _ = self.tx.send(Command::Park);
    }

    fn explode(&self) {
        let _ = self.tx.send(Command::Explode);
    }

    fn give_up(&self) {
        let _ = self.tx.send(Command::GiveUp);
    }
}

fn driver(
    mut rx: mpsc::UnboundedReceiver<Command>,
) -> impl FnOnce(AgentRunContext) -> Driven + Send + 'static {
    move |ctx| {
        Box::pin(async move {
            // Waiting for the next step is itself an await point, which is what
            // lets a cancel reach a driver that is doing nothing in particular.
            while let Some(command) = rx.recv().await {
                match command {
                    Command::Act(act, ack) => {
                        let _ = ack.send(act(&ctx));
                    }
                    Command::Park => std::future::pending::<()>().await,
                    Command::Explode => panic!("the driver exploded"),
                    Command::GiveUp => return Ok(()),
                }
            }
            Ok(())
        })
    }
}

struct Harness {
    manager: Arc<AgentRunManager>,
    workspaces: Arc<WorkspaceManager>,
    workspace: Workspace,
    sink: Arc<RecordingSink>,
    rx: mpsc::UnboundedReceiver<EventEnvelope>,
    _dir: TempDir,
}

fn start() -> Harness {
    let dir = TempDir::new("agent-run-ws");
    let (sink, rx) = RecordingSink::with_channel();
    let events = EventBus::new(sink.clone());
    let registry = OperationRegistry::new(Handle::current(), events.clone());
    let workspaces = WorkspaceManager::new(Handle::current(), events.clone(), registry);
    let workspace = workspaces
        .open(
            dir.path(),
            UnsavedPolicy::Refuse,
            Actor::User,
            CorrelationId::new(),
        )
        .expect("open the workspace");
    let manager = AgentRunManager::new(Handle::current(), events, workspaces.clone());
    Harness {
        manager,
        workspaces,
        workspace,
        sink,
        rx,
        _dir: dir,
    }
}

impl Harness {
    fn spec(&self) -> AgentRunSpec {
        self.spec_for(WorkstreamId::from_raw("ws-1"))
    }

    fn spec_for(&self, workstream_id: WorkstreamId) -> AgentRunSpec {
        AgentRunSpec {
            workspace_id: self.workspace.id.clone(),
            workstream_id,
            provider: AgentRunProvider::Fake,
            actor: Actor::User,
            correlation_id: CorrelationId::new(),
        }
    }

    /// Start a run with a script the test drives.
    fn scripted(&self) -> (AgentRun, Script) {
        self.scripted_for(self.spec())
    }

    fn scripted_for(&self, spec: AgentRunSpec) -> (AgentRun, Script) {
        let (tx, rx) = mpsc::unbounded_channel();
        let run = self
            .manager
            .start(spec, driver(rx))
            .expect("the run starts");
        (run, Script { tx })
    }

    /// Wait for the run to reach a terminal state, on the event stream rather
    /// than on the clock.
    async fn ended(&mut self, id: &AgentRunId) -> AgentRun {
        timeout(Duration::from_secs(5), async {
            loop {
                let envelope = self.rx.recv().await.expect("the event stream is open");
                if let CoreEvent::AgentRunChanged(payload) = &envelope.event
                    && payload.run.id == *id
                    && payload.run.state.is_terminal()
                {
                    return payload.run.clone();
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("run {id} never reached a terminal state"))
    }

    /// Every state this run was announced in, in order.
    fn states(&self, id: &AgentRunId) -> Vec<&'static str> {
        self.sink
            .recorded()
            .iter()
            .filter_map(|envelope| match &envelope.event {
                CoreEvent::AgentRunChanged(payload) if payload.run.id == *id => {
                    Some(payload.run.state.as_str())
                }
                _ => None,
            })
            .collect()
    }

    /// Every event this run reported, in order, flattened out of its batches.
    fn kinds(&self, id: &AgentRunId) -> Vec<&'static str> {
        self.sink
            .recorded()
            .iter()
            .filter_map(|envelope| match &envelope.event {
                CoreEvent::AgentRunEventAppended(payload) if payload.run_id == *id => {
                    Some(payload.events.iter().map(|record| record.event.as_str()))
                }
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn event_names(&self) -> Vec<&'static str> {
        self.sink.names()
    }
}

// ---------------------------------------------------------------------------
// Starting
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_started_run_exists_before_its_events_do() {
    let harness = start();
    let (run, _script) = harness.scripted();

    assert_eq!(run.state, AgentRunState::Starting);
    assert_eq!(run.provider, AgentRunProvider::Fake);
    assert!(!run.cancellation_requested);
    assert!(run.finished_at.is_none());
    assert!(run.activity.is_none());

    // A client drops a batch whose run it does not know, so the run has to be
    // announced first.
    assert_eq!(
        harness.event_names(),
        vec![
            "WorkspaceOpened",
            "AgentRunChanged",
            "AgentRunEventAppended"
        ]
    );
    assert_eq!(
        harness.kinds(&run.id),
        vec!["RUN_STARTED"],
        "the manager records the run's own beginning, exactly once"
    );
}

#[tokio::test]
async fn a_run_needs_an_open_workspace() {
    let harness = start();
    let mut spec = harness.spec();
    spec.workspace_id = WorkspaceId::from_raw("not-open");
    let (_tx, rx) = mpsc::unbounded_channel();

    let error = harness
        .manager
        .start(spec, driver(rx))
        .expect_err("a run has nowhere to happen without a workspace");
    assert_eq!(error.code, CoreErrorCode::NotFound);
    assert!(
        harness.manager.list(&harness.workspace.id).is_empty(),
        "no run was created"
    );
}

#[tokio::test]
async fn a_workspace_holds_no_more_than_max_concurrent_runs() {
    let harness = start();
    let mut scripts = Vec::new();
    for _ in 0..MAX_CONCURRENT_RUNS {
        scripts.push(harness.scripted());
    }

    let (_tx, rx) = mpsc::unbounded_channel();
    let error = harness
        .manager
        .start(harness.spec(), driver(rx))
        .expect_err("the bound refuses rather than queues");
    assert_eq!(error.code, CoreErrorCode::Conflict);
    assert!(
        error.message.contains(&MAX_CONCURRENT_RUNS.to_string()),
        "the refusal names the count: {}",
        error.message
    );
    assert_eq!(
        harness.manager.list(&harness.workspace.id).len(),
        MAX_CONCURRENT_RUNS,
        "the refused run was not created"
    );
}

#[tokio::test]
async fn an_ended_run_frees_its_slot() {
    let mut harness = start();
    let mut scripts = Vec::new();
    for _ in 0..MAX_CONCURRENT_RUNS {
        scripts.push(harness.scripted());
    }
    let (first, script) = &scripts[0];
    script.to(AgentRunState::Running).await.expect("running");
    script.finish("done").await.expect("finished");
    assert_eq!(
        harness.manager.get(&first.id).expect("still listed").state,
        AgentRunState::Finished
    );

    let (_replacement, _script) = harness.scripted();
    assert_eq!(
        harness.manager.list(&harness.workspace.id).len(),
        MAX_CONCURRENT_RUNS + 1,
        "the ended run is still listed, and it no longer counts as active"
    );
    let _ = harness.rx.try_recv();
}

#[tokio::test]
async fn runs_are_listed_per_workspace_oldest_first() {
    let harness = start();
    let (first, _a) = harness.scripted();
    let (second, _b) = harness.scripted();

    let listed = harness.manager.list(&harness.workspace.id);
    assert_eq!(
        listed.iter().map(|run| run.id.clone()).collect::<Vec<_>>(),
        vec![first.id, second.id]
    );
    assert!(
        harness
            .manager
            .list(&WorkspaceId::from_raw("elsewhere"))
            .is_empty(),
        "another workspace's runs are not this one's"
    );
}

// ---------------------------------------------------------------------------
// The acceptance criterion: a deterministic lifecycle, twice, identical
// ---------------------------------------------------------------------------

/// `STARTING → RUNNING → BLOCKED (permission) → RUNNING → NEEDS_DECISION →
/// RUNNING → FINISHED`, with the exact event sequence asserted, and no `sleep`
/// anywhere in the assertion path.
async fn scripted_lifecycle() -> (Vec<&'static str>, Vec<&'static str>) {
    let harness = start();
    let (run, script) = harness.scripted();

    script
        .record(AgentRunEvent::PromptStarted {
            text: "add a test".into(),
        })
        .await;
    script.to(AgentRunState::Running).await.expect("running");

    script
        .record(AgentRunEvent::PermissionRequested {
            tool: "Bash".into(),
            detail: Some("cargo test".into()),
        })
        .await;
    script
        .record(AgentRunEvent::RunBlocked {
            reason: "needs permission to run a command".into(),
        })
        .await;
    script
        .advance(
            AgentRunState::Blocked,
            AgentRunActivityKind::RequestingPermission,
        )
        .await
        .expect("blocked");

    script.record(AgentRunEvent::RunResumed).await;
    script
        .advance(AgentRunState::Running, AgentRunActivityKind::Executing)
        .await
        .expect("resumed");

    script
        .record(AgentRunEvent::DecisionRequested {
            question: "which crate should own this?".into(),
        })
        .await;
    script
        .advance(
            AgentRunState::NeedsDecision,
            AgentRunActivityKind::RequestingDecision,
        )
        .await
        .expect("needs a decision");

    script.record(AgentRunEvent::RunResumed).await;
    script
        .advance(AgentRunState::Running, AgentRunActivityKind::Editing)
        .await
        .expect("resumed again");

    script
        .record(AgentRunEvent::PromptFinished {
            text: Some("added".into()),
        })
        .await;
    script.finish("added one test").await.expect("finished");

    let ended = harness.manager.get(&run.id).expect("the run is listed");
    assert_eq!(ended.state, AgentRunState::Finished);
    assert!(ended.finished_at.is_some());
    assert!(
        ended.activity.is_none(),
        "nothing is happening once a run has ended"
    );
    assert!(ended.error.is_none());

    (harness.states(&run.id), harness.kinds(&run.id))
}

#[tokio::test]
async fn the_scripted_lifecycle_is_deterministic() {
    let (states, kinds) = scripted_lifecycle().await;
    assert_eq!(
        states,
        vec![
            "STARTING",
            "RUNNING",
            "BLOCKED",
            "RUNNING",
            "NEEDS_DECISION",
            "RUNNING",
            "FINISHED",
        ]
    );
    assert_eq!(
        kinds,
        vec![
            "RUN_STARTED",
            "PROMPT_STARTED",
            "PERMISSION_REQUESTED",
            "RUN_BLOCKED",
            "RUN_RESUMED",
            "DECISION_REQUESTED",
            "RUN_RESUMED",
            "PROMPT_FINISHED",
            "RUN_FINISHED",
        ]
    );

    let (again_states, again_kinds) = scripted_lifecycle().await;
    assert_eq!(again_states, states, "the same script, the same states");
    assert_eq!(again_kinds, kinds, "the same script, the same events");
}

// ---------------------------------------------------------------------------
// Cancellation
// ---------------------------------------------------------------------------

/// The acceptance criterion, state by state: a cancel lands wherever the run
/// happens to be, including on a run blocked on something it will never get.
#[tokio::test]
async fn a_run_is_cancellable_from_every_non_terminal_state() {
    for &state in AgentRunState::ALL {
        if state.is_terminal() {
            continue;
        }
        let mut harness = start();
        let (run, script) = harness.scripted();

        match state {
            AgentRunState::Starting => {}
            AgentRunState::Running => {
                script.to(AgentRunState::Running).await.expect("running");
            }
            other => {
                script.to(AgentRunState::Running).await.expect("running");
                script.to(other).await.expect("the state under test");
            }
        }
        assert_eq!(
            harness.manager.get(&run.id).expect("listed").state,
            state,
            "the run is in the state under test before it is cancelled"
        );

        let requested = harness
            .manager
            .cancel(&run.id, Actor::User)
            .expect("cancel is accepted");
        assert!(
            requested.cancellation_requested,
            "the request is visible at once, cooperatively"
        );

        let ended = harness.ended(&run.id).await;
        assert_eq!(
            ended.state,
            AgentRunState::Cancelled,
            "a run in {state:?} ends CANCELLED"
        );
        assert!(ended.finished_at.is_some());
        assert!(ended.error.is_none(), "a cancel is not a failure");
        assert!(
            !harness.kinds(&run.id).contains(&"RUN_FAILED"),
            "cancellation is reported by the state, not by an event"
        );
    }
}

/// A driver parked on something that never resolves is exactly what a real
/// agent blocked on a permission looks like. The token alone would not save it;
/// the supervisor aborting it does.
#[tokio::test]
async fn a_driver_parked_forever_is_still_cancelled() {
    let mut harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");
    script
        .to(AgentRunState::Blocked)
        .await
        .expect("blocked on a permission");
    script.park();

    harness
        .manager
        .cancel(&run.id, Actor::User)
        .expect("cancel is accepted");
    let ended = harness.ended(&run.id).await;
    assert_eq!(ended.state, AgentRunState::Cancelled);
}

#[tokio::test]
async fn cancelling_a_run_that_already_ended_changes_nothing() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");
    script.finish("done").await.expect("finished");
    let before = harness.manager.get(&run.id).expect("listed");
    let names_before = harness.event_names().len();

    for _ in 0..3 {
        let after = harness
            .manager
            .cancel(&run.id, Actor::User)
            .expect("idempotent");
        assert_eq!(after, before, "the snapshot comes back unchanged");
    }
    assert_eq!(
        harness.event_names().len(),
        names_before,
        "an idempotent no-op emits nothing"
    );
}

#[tokio::test]
async fn cancelling_an_unknown_run_is_not_found() {
    let harness = start();
    let error = harness
        .manager
        .cancel(&AgentRunId::from_raw("no-such-run"), Actor::User)
        .expect_err("unknown ids are not silently accepted");
    assert_eq!(error.code, CoreErrorCode::NotFound);
}

/// Once a stop was asked for, the only ending is the one the manager chose —
/// the operation registry's rule, and what stops a driver un-cancelling itself
/// in the window before its task is aborted.
#[tokio::test]
async fn a_stopping_run_cannot_be_moved_or_finished_by_its_driver() {
    let mut harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");

    // The cancel is asked for from *inside* the driver's own step, which is the
    // only way to observe the window between the request and the abort: the
    // driver's task cannot be aborted until it next awaits, so this is exactly
    // what a driver that carried on regardless would try to do.
    let manager = Arc::clone(&harness.manager);
    let id = run.id.clone();
    let refused = script
        .act(move |ctx| {
            manager
                .cancel(&id, Actor::User)
                .expect("cancel is accepted while the driver is mid-step");
            ctx.transition(AgentRunState::Review).map(|_| ())
        })
        .await
        .expect_err("a stopping run does not carry on");
    assert_eq!(refused.code, CoreErrorCode::Conflict);
    assert!(
        refused.message.contains("stopping"),
        "the refusal says why: {}",
        refused.message
    );

    let ended = harness.ended(&run.id).await;
    assert_eq!(ended.state, AgentRunState::Cancelled);
}

// ---------------------------------------------------------------------------
// Never orphaned
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_driver_that_returns_without_ending_its_run_fails_it() {
    let mut harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");
    script.give_up();

    let ended = harness.ended(&run.id).await;
    assert_eq!(
        ended.state,
        AgentRunState::Failed,
        "never left RUNNING forever"
    );
    let error = ended.error.expect("a failure says how");
    assert_eq!(error.code, CoreErrorCode::Internal);
    assert!(
        error.message.contains("without ending the run"),
        "the gap is named: {}",
        error.message
    );
    assert_eq!(
        harness.kinds(&run.id).last().copied(),
        Some("RUN_FAILED"),
        "the log closes with the ending"
    );
}

/// A script that is simply dropped is the same orphan seen from the other side:
/// the driver's `recv` returns `None` and it returns without ending anything.
#[tokio::test]
async fn a_run_whose_script_is_dropped_still_ends() {
    let mut harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");
    drop(script);

    let ended = harness.ended(&run.id).await;
    assert_eq!(ended.state, AgentRunState::Failed);
}

#[tokio::test]
async fn a_panicking_driver_is_contained_and_fails_its_run() {
    let mut harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");
    script.explode();

    let ended = harness.ended(&run.id).await;
    assert_eq!(ended.state, AgentRunState::Failed);
    let error = ended.error.expect("a failure says how");
    assert_eq!(error.code, CoreErrorCode::Internal);
    assert!(
        error.message.contains("panicked") && error.message.contains("exploded"),
        "the panic is reported, not swallowed: {}",
        error.message
    );
    // The process is still here, and so is the manager.
    let (_another, _script) = harness.scripted();
}

#[tokio::test]
async fn a_driver_may_fail_its_own_run() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");
    script.fail("the model refused").await.expect("failed");

    let ended = harness.manager.get(&run.id).expect("listed");
    assert_eq!(ended.state, AgentRunState::Failed);
    assert_eq!(
        ended.error.expect("an error").message,
        "the model refused",
        "the driver's own reason survives"
    );
    assert_eq!(harness.kinds(&run.id).last().copied(), Some("RUN_FAILED"));
}

#[tokio::test]
async fn removing_the_workstream_cancels_only_its_own_runs() {
    let mut harness = start();
    let doomed = WorkstreamId::from_raw("ws-doomed");
    let other = WorkstreamId::from_raw("ws-other");
    let (a, script_a) = harness.scripted_for(harness.spec_for(doomed.clone()));
    let (b, script_b) = harness.scripted_for(harness.spec_for(other));
    script_a.to(AgentRunState::Running).await.expect("running");
    script_b.to(AgentRunState::Running).await.expect("running");

    let stopped = harness.manager.cancel_for_workstream(&doomed);
    assert_eq!(stopped.len(), 1);

    let ended = harness.ended(&a.id).await;
    assert_eq!(
        ended.state,
        AgentRunState::Cancelled,
        "a person removed the work, so the run was cancelled"
    );
    assert_eq!(
        harness.manager.get(&b.id).expect("listed").state,
        AgentRunState::Running,
        "another workstream's run is untouched"
    );
}

#[tokio::test]
async fn closing_the_workspace_interrupts_its_runs() {
    let mut harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");

    // The reaction the composition root wires to `WorkspaceClosed`.
    harness
        .workspaces
        .close(
            &harness.workspace.id,
            UnsavedPolicy::Discard,
            Actor::User,
            CorrelationId::new(),
        )
        .expect("close the workspace");
    let stopped = harness
        .manager
        .interrupt_for_workspace(&harness.workspace.id);
    assert_eq!(stopped.len(), 1);

    let ended = harness.ended(&run.id).await;
    assert_eq!(
        ended.state,
        AgentRunState::Interrupted,
        "nobody asked the run to stop; the ground went out from under it"
    );
    assert!(ended.finished_at.is_some());
}

#[tokio::test]
async fn shutdown_leaves_no_run_running() {
    let harness = start();
    let (first, script_a) = harness.scripted();
    let (second, script_b) = harness.scripted();
    script_a.to(AgentRunState::Running).await.expect("running");
    script_b.to(AgentRunState::Running).await.expect("running");
    script_b.to(AgentRunState::Blocked).await.expect("blocked");
    script_b.park();

    harness.manager.shutdown(Duration::from_secs(2)).await;

    for id in [&first.id, &second.id] {
        let run = harness.manager.get(id).expect("still listed");
        assert_eq!(
            run.state,
            AgentRunState::Interrupted,
            "every run is terminal once shutdown returns"
        );
        assert!(run.finished_at.is_some());
    }
}

/// The force-kill case: the task that was supervising the run is gone, so
/// nothing will ever report its ending. Shutdown must still not leave a row
/// claiming to be running — the orphan the state machine exists to prevent.
#[tokio::test]
async fn shutdown_force_ends_a_run_whose_supervisor_died() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");
    script.park();

    harness.manager.abandon_supervisor(&run.id);
    assert_eq!(
        harness.manager.get(&run.id).expect("listed").state,
        AgentRunState::Running,
        "nothing has ended it yet"
    );

    harness.manager.shutdown(Duration::from_millis(50)).await;
    let ended = harness.manager.get(&run.id).expect("listed");
    assert_eq!(ended.state, AgentRunState::Interrupted);
    assert!(ended.finished_at.is_some());
}

/// The same abandonment, met by `runs.cancel` instead of by shutdown.
#[tokio::test]
async fn cancelling_a_run_whose_supervisor_died_still_ends_it() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");
    script.park();
    harness.manager.abandon_supervisor(&run.id);

    let ended = harness
        .manager
        .cancel(&run.id, Actor::User)
        .expect("cancel is accepted");
    assert_eq!(ended.state, AgentRunState::Cancelled);
}

// ---------------------------------------------------------------------------
// Refusals: duplicated and out-of-order steps
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_duplicated_step_is_refused_and_changes_nothing() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");
    let before = harness.event_names().len();

    let refused = script
        .to(AgentRunState::Running)
        .await
        .expect_err("a duplicate is not a change");
    assert_eq!(refused.code, CoreErrorCode::Conflict);
    assert!(refused.message.contains("already"), "{}", refused.message);
    assert_eq!(
        harness.manager.get(&run.id).expect("listed").state,
        AgentRunState::Running
    );
    assert_eq!(
        harness.event_names().len(),
        before,
        "a refused step emits nothing"
    );
}

#[tokio::test]
async fn an_out_of_order_step_is_refused_by_the_state_machine() {
    let harness = start();
    let (run, script) = harness.scripted();

    let refused = script
        .to(AgentRunState::Review)
        .await
        .expect_err("a run that has not begun has nothing to review");
    assert_eq!(refused.code, CoreErrorCode::Conflict);
    assert_eq!(
        harness.manager.get(&run.id).expect("listed").state,
        AgentRunState::Starting
    );
}

/// A driver does not get to say it was cancelled or interrupted: those are the
/// supervisor's answers about the supervised thing.
#[tokio::test]
async fn a_driver_cannot_claim_a_terminal_state_that_is_the_managers() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");

    for state in [
        AgentRunState::Cancelled,
        AgentRunState::Interrupted,
        AgentRunState::Finished,
        AgentRunState::Failed,
    ] {
        let refused = script
            .to(state)
            .await
            .expect_err("a driver finishes or fails; it does not transition to an ending");
        assert_eq!(refused.code, CoreErrorCode::Conflict);
    }
    assert_eq!(
        harness.manager.get(&run.id).expect("listed").state,
        AgentRunState::Running
    );
}

// ---------------------------------------------------------------------------
// Events: bounding, coalescing, the ring, and the unknown run
// ---------------------------------------------------------------------------

#[tokio::test]
async fn events_are_coalesced_into_one_batch_per_step() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");

    script
        .act(|ctx| {
            ctx.record_all((0..50).map(|index| AgentRunEvent::PromptStarted {
                text: format!("chunk {index}"),
            }));
            Ok(())
        })
        .await
        .expect("recorded");

    let batches = harness
        .sink
        .recorded()
        .iter()
        .filter(|envelope| matches!(&envelope.event, CoreEvent::AgentRunEventAppended(payload) if payload.run_id == run.id))
        .count();
    assert_eq!(
        batches, 2,
        "one batch for RUN_STARTED and one for the fifty, never fifty-one events"
    );
    assert_eq!(harness.kinds(&run.id).len(), 51);
}

#[tokio::test]
async fn an_activity_reported_twice_emits_once() {
    let harness = start();
    let (_run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");
    let before = harness.event_names().len();

    for _ in 0..3 {
        script
            .act(|ctx| {
                ctx.set_activity(Some(AgentRunActivity::new(
                    AgentRunActivityKind::Thinking,
                    Timestamp::from_millis(7),
                )))
                .map(|_| ())
            })
            .await
            .expect("activity accepted");
    }
    assert_eq!(
        harness.event_names().len(),
        before + 1,
        "the same activity three times is one change"
    );
}

#[tokio::test]
async fn an_adapters_string_is_bounded_before_it_is_recorded() {
    let harness = start();
    let (run, script) = harness.scripted();
    script
        .record(AgentRunEvent::PromptStarted {
            text: "x".repeat(MAX_TEXT * 3),
        })
        .await;

    let log = harness.manager.events(&run.id).expect("the log");
    let AgentRunEvent::PromptStarted { text } = &log.events[1].event else {
        panic!("expected PROMPT_STARTED");
    };
    assert_eq!(text.chars().count(), MAX_TEXT);
}

#[tokio::test]
async fn the_ring_reports_what_it_dropped() {
    use super::wire::MAX_RUN_EVENTS;

    let harness = start();
    let (run, script) = harness.scripted();
    let overflow = 5;
    script
        .act(move |ctx| {
            for _ in 0..MAX_RUN_EVENTS + overflow {
                ctx.record(AgentRunEvent::RunResumed);
            }
            Ok(())
        })
        .await
        .expect("recorded");

    let log = harness.manager.events(&run.id).expect("the log");
    assert_eq!(log.events.len(), MAX_RUN_EVENTS);
    assert_eq!(
        log.dropped,
        overflow as u32 + 1,
        "RUN_STARTED plus the overflow, evicted from the oldest end and counted"
    );
    assert!(
        !log.events
            .iter()
            .any(|record| record.event.as_str() == "RUN_STARTED"),
        "the oldest record is the one that goes"
    );
}

#[tokio::test]
async fn rehydration_does_not_replay_what_it_just_answered() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.record(AgentRunEvent::RunResumed).await;
    let before = harness.event_names().len();

    let log = harness.manager.events(&run.id).expect("the log");
    assert_eq!(log.events.len(), 2, "RUN_STARTED and the resume");
    script
        .act(|ctx| {
            assert!(!ctx.flush(), "nothing is left undelivered");
            Ok(())
        })
        .await
        .expect("flushed");
    assert_eq!(harness.event_names().len(), before);
}

#[tokio::test]
async fn events_for_a_run_that_does_not_exist_are_dropped() {
    let harness = start();
    let before = harness.event_names().len();

    let unknown = AgentRunId::from_raw("no-such-run");
    for _ in 0..5 {
        harness.manager.record(&unknown, AgentRunEvent::RunResumed);
    }
    assert_eq!(
        harness.event_names().len(),
        before,
        "nothing is forwarded for a run nobody knows"
    );
    assert!(harness.manager.events(&unknown).is_err());
}

#[tokio::test]
async fn events_reported_after_the_run_ended_are_dropped() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.expect("running");
    script.finish("done").await.expect("finished");
    let after_ending = harness
        .manager
        .events(&run.id)
        .expect("the log")
        .events
        .len();

    script
        .act(|ctx| {
            ctx.record(AgentRunEvent::RunResumed);
            Ok(())
        })
        .await
        .expect("recording never fails, it just does nothing");
    assert_eq!(
        harness
            .manager
            .events(&run.id)
            .expect("the log")
            .events
            .len(),
        after_ending,
        "a run that ended does not grow a longer history"
    );
}

#[tokio::test]
async fn events_of_an_unknown_run_are_not_found() {
    let harness = start();
    let error = harness
        .manager
        .events(&AgentRunId::from_raw("no-such-run"))
        .expect_err("unknown ids are not silently empty");
    assert_eq!(error.code, CoreErrorCode::NotFound);
}

/// Every edge `runs.prompt` refuses, with the state controlled rather than
/// raced (T310-B §4).
///
/// The integration test through the real IPC can only assert that a prompt is
/// refused; which refusal it gets depends on how far a script has run. Here the
/// run is driven to exactly the state each case needs.
#[tokio::test]
async fn a_prompt_is_refused_at_every_edge() {
    let mut harness = start();
    let (run, script) = harness.scripted();

    // Unknown id: the same wording every other `runs.*` lookup miss uses.
    let unknown = harness
        .manager
        .prompt(
            &AgentRunId::from_raw("run-nope"),
            "hello".to_owned(),
            Actor::User,
        )
        .expect_err("an unknown run is not found");
    assert_eq!(unknown.code, CoreErrorCode::NotFound);

    // `STARTING`: there is no child to speak to yet, and the refusal says so
    // rather than queueing text that would be invisible if the run then failed.
    let starting = harness
        .manager
        .prompt(&run.id, "hello".to_owned(), Actor::User)
        .expect_err("a starting run is not accepting a prompt");
    assert_eq!(starting.code, CoreErrorCode::Conflict);
    assert!(
        starting.message.contains("STARTING"),
        "the refusal names the state: {}",
        starting.message
    );

    // `RUNNING`, so the state check passes and the *channel* check is what
    // answers: a fake has no inbox, and the refusal names the provider.
    script
        .act(|ctx| ctx.advance(AgentRunState::Running, None).map(|_| ()))
        .await
        .expect("the run reaches RUNNING");
    let no_channel = harness
        .manager
        .prompt(&run.id, "hello".to_owned(), Actor::User)
        .expect_err("a fake run has no prompt channel");
    assert_eq!(no_channel.code, CoreErrorCode::Conflict);
    assert!(
        no_channel.message.contains("FAKE"),
        "the refusal names the provider: {}",
        no_channel.message
    );

    // Terminal: the state again, and it is the last word.
    script
        .act(|ctx| ctx.finish(None).map(|_| ()))
        .await
        .expect("the run finishes");
    let ended = harness.ended(&run.id).await;
    let terminal = harness
        .manager
        .prompt(&run.id, "hello".to_owned(), Actor::User)
        .expect_err("an ended run cannot be prompted");
    assert_eq!(terminal.code, CoreErrorCode::Conflict);
    assert!(
        terminal.message.contains(ended.state.as_str()),
        "the refusal names the state: {}",
        terminal.message
    );
}

/// A prompt reaches a live driver, and the queue is bounded rather than
/// unbounded (T310-B §4).
///
/// The driver here never reads its inbox, which is exactly the case the bound
/// exists for: a caller that outruns the agent is refused, never allowed to
/// grow memory without limit.
#[tokio::test]
async fn a_prompt_queue_is_bounded_and_refuses_rather_than_grows() {
    let harness = start();
    let (inbox, receiver) = RunInbox::pair();
    let session = OwnedProviderSession {
        session_id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_owned(),
        pid: None,
    };
    // A driver that holds its inbox and never drains it, so the queue fills.
    let drive = move |ctx: AgentRunContext| {
        let mut receiver = receiver;
        Box::pin(async move {
            ctx.advance(AgentRunState::Running, None)?;
            // Keep the receiver alive without taking from it.
            ctx.cancelled().await;
            let _ = &mut receiver;
            Ok(())
        }) as Driven
    };
    let run = harness
        .manager
        .start_with_inbox(harness.spec(), inbox, session.clone(), drive)
        .expect("the run starts");

    // Wait for the driver to have moved it to RUNNING before prompting.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while harness.manager.get(&run.id).map(|run| run.state) != Some(AgentRunState::Running) {
        assert!(std::time::Instant::now() < deadline, "the driver never ran");
        tokio::task::yield_now().await;
    }

    // The manager knows which provider session it owns, which is what the
    // observed roster is later handed so it can exclude Gritty's own child.
    assert_eq!(harness.manager.owned_provider_sessions(), vec![session]);

    for index in 0..MAX_QUEUED_PROMPTS {
        harness
            .manager
            .prompt(&run.id, format!("prompt {index}"), Actor::User)
            .unwrap_or_else(|error| panic!("prompt {index} should be accepted: {error:?}"));
    }
    let full = harness
        .manager
        .prompt(&run.id, "one too many".to_owned(), Actor::User)
        .expect_err("the queue is bounded");
    assert_eq!(full.code, CoreErrorCode::Conflict);
    assert!(
        full.message.contains(&MAX_QUEUED_PROMPTS.to_string()),
        "the refusal names the bound: {}",
        full.message
    );

    // Accepting a prompt changes nothing about the run: it is queued, not seen.
    let after = harness.manager.get(&run.id).expect("the run still exists");
    assert_eq!(after.state, AgentRunState::Running);
    assert!(after.activity.is_none());
}

// ---------------------------------------------------------------------------
// Permission asks (T310-ANSWER)
// ---------------------------------------------------------------------------

use super::manager::{PendingVerdict, PermissionAskDraft};
use super::wire::{
    AgentRunPermissionVerb, AgentRunPermissionVerdict, MAX_ASK_TARGET, MAX_DETAIL, MAX_TOOL,
};

fn draft(target: &str) -> PermissionAskDraft {
    PermissionAskDraft {
        tool: "Bash".to_owned(),
        verb: AgentRunPermissionVerb::RunCommand,
        target: target.to_owned(),
        detail: None,
    }
}

fn asking_events() -> Vec<AgentRunEvent> {
    vec![
        AgentRunEvent::PermissionRequested {
            tool: "Bash".to_owned(),
            detail: None,
        },
        AgentRunEvent::RunBlocked {
            reason: "Bash needs permission".to_owned(),
        },
    ]
}

fn requesting() -> Option<AgentRunActivity> {
    Some(AgentRunActivity::new(
        AgentRunActivityKind::RequestingPermission,
        Timestamp::from_millis(1),
    ))
}

impl Script {
    /// Raise an ask from inside the driver and hand its verdict end back.
    async fn ask(&self, draft: PermissionAskDraft) -> Result<PendingVerdict, CoreError> {
        let (tx, rx) = oneshot::channel();
        self.act(move |ctx| {
            let pending = ctx.ask_permission(draft, asking_events(), requesting())?;
            let _ = tx.send(pending);
            Ok(())
        })
        .await?;
        Ok(rx.await.expect("the verdict end was handed back"))
    }
}

/// Resolves when `pending` does, or reports that it had not yet.
async fn resolved(pending: &mut PendingVerdict) -> Option<Option<AgentRunPermissionVerdict>> {
    timeout(Duration::from_millis(50), pending.wait())
        .await
        .ok()
}

#[tokio::test]
async fn an_ask_blocks_the_run_and_sits_on_it_in_one_change() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.unwrap();
    let before = harness.states(&run.id).len();

    let _pending = script
        .ask(draft("cargo test"))
        .await
        .expect("the ask is accepted");

    let blocked = harness.manager.get(&run.id).expect("the run exists");
    assert_eq!(blocked.state, AgentRunState::Blocked);
    assert_eq!(
        blocked.activity.map(|activity| activity.kind),
        Some(AgentRunActivityKind::RequestingPermission)
    );
    let ask = blocked.pending.expect("the ask is on the run");
    assert_eq!(ask.tool, "Bash");
    assert_eq!(ask.verb, AgentRunPermissionVerb::RunCommand);
    assert_eq!(ask.target, "cargo test");
    assert!(
        uuid::Uuid::try_parse(&ask.ask_id).is_ok(),
        "the core mints the id"
    );

    // One `AgentRunChanged` for the state, the activity and the ask together,
    // and it carries the ask: no consumer sees a run blocked on nothing.
    let changes: Vec<AgentRun> = harness
        .sink
        .recorded()
        .iter()
        .filter_map(|envelope| match &envelope.event {
            CoreEvent::AgentRunChanged(payload) if payload.run.id == run.id => {
                Some(payload.run.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(changes.len(), before + 1);
    assert!(changes.last().unwrap().pending.is_some());
    // The events explaining the stop arrived before the change.
    let kinds = harness.kinds(&run.id);
    assert!(
        kinds.ends_with(&["PERMISSION_REQUESTED", "RUN_BLOCKED"]),
        "{kinds:?}"
    );
}

#[tokio::test]
async fn an_answer_reaches_the_driver_once_and_takes_the_ask_off_the_run() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.unwrap();
    let mut pending = script.ask(draft("cargo test")).await.unwrap();
    let ask_id = harness
        .manager
        .get(&run.id)
        .unwrap()
        .pending
        .unwrap()
        .ask_id;

    let answered = harness
        .manager
        .answer_permission(
            &run.id,
            &ask_id,
            AgentRunPermissionVerdict::AllowOnce,
            Actor::User,
        )
        .expect("the answer is accepted");
    assert!(
        answered.pending.is_none(),
        "the ask stops being answerable at once"
    );
    // The run leaves BLOCKED when the child acknowledges, not here.
    assert_eq!(answered.state, AgentRunState::Blocked);
    assert_eq!(
        resolved(&mut pending).await,
        Some(Some(AgentRunPermissionVerdict::AllowOnce))
    );

    // A second click is not a second verdict, and not an error either.
    let again = harness
        .manager
        .answer_permission(
            &run.id,
            &ask_id,
            AgentRunPermissionVerdict::Deny,
            Actor::User,
        )
        .expect("an answer to a gone ask is not an error");
    assert_eq!(again, harness.manager.get(&run.id).unwrap());
}

#[tokio::test]
async fn an_answer_addressed_to_another_ask_changes_nothing() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.unwrap();
    let mut pending = script.ask(draft("cargo test")).await.unwrap();

    let unchanged = harness
        .manager
        .answer_permission(
            &run.id,
            "not-the-ask",
            AgentRunPermissionVerdict::AllowOnce,
            Actor::User,
        )
        .expect("not an error");
    assert!(
        unchanged.pending.is_some(),
        "the real ask is still outstanding"
    );
    assert_eq!(resolved(&mut pending).await, None, "nothing was delivered");

    let unknown = harness
        .manager
        .answer_permission(
            &AgentRunId::from_raw("no-such-run"),
            "ask",
            AgentRunPermissionVerdict::AllowOnce,
            Actor::User,
        )
        .expect_err("an unknown run");
    assert_eq!(unknown.code, CoreErrorCode::NotFound);
}

#[tokio::test]
async fn a_second_ask_while_one_is_outstanding_is_refused_and_the_first_stands() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.unwrap();
    let _first = script.ask(draft("first")).await.unwrap();
    let first_id = harness
        .manager
        .get(&run.id)
        .unwrap()
        .pending
        .unwrap()
        .ask_id;
    let kinds_before = harness.kinds(&run.id);

    let error = script
        .ask(draft("second"))
        .await
        .expect_err("one ask at a time");
    assert_eq!(error.code, CoreErrorCode::Conflict);

    let after = harness.manager.get(&run.id).unwrap();
    let pending = after.pending.expect("the first ask is untouched");
    assert_eq!(pending.ask_id, first_id);
    assert_eq!(pending.target, "first");
    assert_eq!(
        harness.kinds(&run.id),
        kinds_before,
        "a refused ask records nothing: it did not stop the run"
    );
}

/// The rule the task exists to protect: an ask nobody answers is never an
/// allowance, however long it is left.
#[tokio::test(start_paused = true)]
async fn an_unanswered_ask_never_resolves_on_its_own() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.unwrap();
    let mut pending = script.ask(draft("rm -rf target")).await.unwrap();

    tokio::time::advance(Duration::from_secs(24 * 60 * 60)).await;
    assert_eq!(
        resolved(&mut pending).await,
        None,
        "a day later, still no verdict"
    );
    assert!(harness.manager.get(&run.id).unwrap().pending.is_some());
}

#[tokio::test]
async fn cancelling_a_blocked_run_withdraws_its_ask_as_no_answer() {
    let mut harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.unwrap();
    let mut pending = script.ask(draft("cargo test")).await.unwrap();
    let ask_id = harness
        .manager
        .get(&run.id)
        .unwrap()
        .pending
        .unwrap()
        .ask_id;

    let stopping = harness.manager.cancel(&run.id, Actor::User).unwrap();
    assert!(
        stopping.pending.is_none(),
        "the stop is the answer; a prompt must not offer a choice that is gone"
    );
    // `None` is what the driver denies on. It is never an allowance.
    assert_eq!(resolved(&mut pending).await, Some(None));
    let late = harness
        .manager
        .answer_permission(
            &run.id,
            &ask_id,
            AgentRunPermissionVerdict::AllowOnce,
            Actor::User,
        )
        .expect("a withdrawn ask is not an error");
    assert!(late.pending.is_none());

    let ended = harness.ended(&run.id).await;
    assert_eq!(ended.state, AgentRunState::Cancelled);
    assert!(ended.pending.is_none());
}

#[tokio::test]
async fn a_run_that_ends_while_asking_withdraws_the_ask() {
    let mut harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.unwrap();
    let mut pending = script.ask(draft("cargo test")).await.unwrap();

    script.fail("the child died").await.unwrap();
    let ended = harness.ended(&run.id).await;
    assert_eq!(ended.state, AgentRunState::Failed);
    assert!(
        ended.pending.is_none(),
        "an ask about a gone process is withdrawn"
    );
    assert_eq!(resolved(&mut pending).await, Some(None));
}

#[tokio::test]
async fn resuming_without_an_answer_clears_the_ask() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.unwrap();
    let mut pending = script.ask(draft("cargo test")).await.unwrap();

    script.to(AgentRunState::Running).await.unwrap();
    assert!(harness.manager.get(&run.id).unwrap().pending.is_none());
    assert_eq!(resolved(&mut pending).await, Some(None));
}

#[tokio::test]
async fn a_run_that_cannot_block_cannot_ask() {
    let harness = start();
    let (run, script) = harness.scripted();
    // STARTING reaches only RUNNING.
    let early = script.ask(draft("x")).await.expect_err("not from STARTING");
    assert_eq!(early.code, CoreErrorCode::Conflict);
    script.to(AgentRunState::Running).await.unwrap();
    script.to(AgentRunState::Waiting).await.unwrap();
    let waiting = script.ask(draft("x")).await.expect_err("not from WAITING");
    assert_eq!(waiting.code, CoreErrorCode::Conflict);
    assert!(harness.manager.get(&run.id).unwrap().pending.is_none());
    assert!(
        !harness.kinds(&run.id).contains(&"RUN_BLOCKED"),
        "a refused ask records no block"
    );

    script.to(AgentRunState::Running).await.unwrap();
    // A parked driver cannot act once a stop lands (it is aborted), so the
    // stopping edge is asked of the manager directly — it is only reachable
    // from a driver in a race, and a race is not something to test by hoping.
    script.park();
    harness.manager.cancel(&run.id, Actor::User).unwrap();
    let stopping = harness
        .manager
        .raise_permission(&run.id, draft("x"), asking_events(), requesting())
        .expect_err("not while stopping");
    assert_eq!(stopping.code, CoreErrorCode::Conflict);
    assert!(
        stopping.message.contains("stopping"),
        "{}",
        stopping.message
    );
}

#[tokio::test]
async fn an_answer_that_cannot_be_delivered_is_refused_and_the_ask_is_gone() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.unwrap();
    let pending = script.ask(draft("cargo test")).await.unwrap();
    let ask_id = harness
        .manager
        .get(&run.id)
        .unwrap()
        .pending
        .unwrap()
        .ask_id;
    // The driver's end is gone: what a driver that died mid-ask leaves.
    drop(pending);

    let error = harness
        .manager
        .answer_permission(
            &run.id,
            &ask_id,
            AgentRunPermissionVerdict::AllowOnce,
            Actor::User,
        )
        .expect_err("the caller must not believe it was delivered");
    assert_eq!(error.code, CoreErrorCode::Conflict);
    assert!(harness.manager.get(&run.id).unwrap().pending.is_none());
}

#[tokio::test]
async fn an_asks_strings_are_bounded_on_the_run() {
    let harness = start();
    let (run, script) = harness.scripted();
    script.to(AgentRunState::Running).await.unwrap();
    let long = "x\n".repeat(20_000);
    let _pending = script
        .ask(PermissionAskDraft {
            tool: long.clone(),
            verb: AgentRunPermissionVerb::Other,
            target: long.clone(),
            detail: Some(format!("{long}!")),
        })
        .await
        .unwrap();
    let ask = harness.manager.get(&run.id).unwrap().pending.unwrap();
    assert_eq!(ask.tool.chars().count(), MAX_TOOL);
    assert_eq!(ask.target.chars().count(), MAX_ASK_TARGET);
    assert_eq!(ask.detail.unwrap().chars().count(), MAX_DETAIL);
}

// ---------------------------------------------------------------------------
// The Claude driver's permission loop, against a real child (T310-ANSWER)
//
// A `/bin/sh` stub stands in for the CLI: it asks, reads whatever Gritty
// writes back, logs it, and performs its side effect **only** if the answer
// allowed it. Every assertion below is about that side effect or that log —
// never about the absence of a log line in Gritty's own tracing (rule 12).
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod claude_permissions {
    use std::path::PathBuf;

    use super::*;
    use crate::agent_run::claude_driver::{self, ClaudeDriverOptions, ClaudeLaunch};

    const SESSION: &str = "3f8a1c2e-9b40-4d17-8e55-6a2b7c4d9f10";

    const INIT: &str = r#"{"type":"system","subtype":"init","session_id":"s","tools":[]}"#;
    const REPLAY: &str =
        r#"{"type":"user","isReplay":true,"message":{"role":"user","content":"go"}}"#;
    const ASK: &str = r#"{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"touch side-effect"},"description":"touch side-effect"}}"#;
    const RESULT: &str =
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done"}"#;
    /// An ordinary `tool_use`: a line whose handling is observable (it records
    /// `COMMAND_STARTED`), used to prove everything before it was handled.
    const MARKER: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_m","name":"Bash","input":{"command":"marker"}}]}}"#;

    struct Stub {
        dir: TempDir,
        program: PathBuf,
    }

    impl Stub {
        /// `body` runs after `init` and the first prompt's replay. `$LOG`,
        /// `$EFFECT` and `$PIDFILE` name files in the stub's own directory.
        fn new(name: &str, body: &str) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let dir = TempDir::new(name);
            let program = dir.path().join("claude-stub");
            let script = format!(
                "#!/bin/sh\nLOG='{log}'\nEFFECT='{effect}'\necho $$ > '{pid}'\n\
                 printf '%s\\n' '{INIT}'\nIFS= read -r prompt\nprintf '%s\\n' '{REPLAY}'\n{body}\n",
                log = dir.path().join("answers.log").display(),
                effect = dir.path().join("side-effect").display(),
                pid = dir.path().join("stub.pid").display(),
            );
            std::fs::write(&program, script).expect("write the stub");
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
                .expect("make it executable");
            Self { dir, program }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }

        fn effect_happened(&self) -> bool {
            self.path("side-effect").exists()
        }

        fn answers(&self) -> String {
            std::fs::read_to_string(self.path("answers.log")).unwrap_or_default()
        }

        fn pid(&self) -> i32 {
            std::fs::read_to_string(self.path("stub.pid"))
                .expect("the stub wrote its pid")
                .trim()
                .parse()
                .expect("a pid")
        }
    }

    /// Ask once, act on the answer, echo it, end the turn, then idle.
    const ASK_AND_ACT: &str = r#"
printf '%s\n' "$ASK"
IFS= read -r answer
printf '%s\n' "$answer" >> "$LOG"
case "$answer" in *'"behavior":"allow"'*) touch "$EFFECT" ;; esac
printf '%s\n' "$answer"
printf '%s\n' "$RESULT"
while IFS= read -r line; do :; done
"#;

    fn with_lines(body: &str) -> String {
        format!("ASK='{ASK}'\nRESULT='{RESULT}'\nMARKER='{MARKER}'\n{body}",)
    }

    impl Harness {
        fn claude(&self, stub: &Stub) -> AgentRun {
            let (inbox, receiver) = RunInbox::pair();
            inbox.seed("go".to_owned());
            let launch = ClaudeLaunch::new(SESSION, self.workspace.root.clone())
                .expect("a valid session id")
                .with_program(&stub.program);
            let spec = AgentRunSpec {
                provider: AgentRunProvider::ClaudeCode,
                ..self.spec()
            };
            self.manager
                .start_with_inbox(
                    spec,
                    inbox,
                    OwnedProviderSession {
                        session_id: SESSION.to_owned(),
                        pid: None,
                    },
                    claude_driver::drive(launch, receiver, ClaudeDriverOptions::default()),
                )
                .expect("the run starts")
        }

        /// The first announcement of this run that satisfies `accept`.
        async fn until(&mut self, id: &AgentRunId, accept: impl Fn(&AgentRun) -> bool) -> AgentRun {
            timeout(Duration::from_secs(20), async {
                loop {
                    let envelope = self.rx.recv().await.expect("the event stream is open");
                    if let CoreEvent::AgentRunChanged(payload) = &envelope.event
                        && payload.run.id == *id
                        && accept(&payload.run)
                    {
                        return payload.run.clone();
                    }
                }
            })
            .await
            .unwrap_or_else(|_| panic!("run {id} never reached the expected state"))
        }

        /// Wait until the run has recorded `kind`, on the event stream.
        async fn until_event(&mut self, id: &AgentRunId, kind: &str) {
            timeout(Duration::from_secs(20), async {
                loop {
                    let envelope = self.rx.recv().await.expect("the event stream is open");
                    if let CoreEvent::AgentRunEventAppended(payload) = &envelope.event
                        && payload.run_id == *id
                        && payload
                            .events
                            .iter()
                            .any(|record| record.event.as_str() == kind)
                    {
                        return;
                    }
                }
            })
            .await
            .unwrap_or_else(|_| panic!("run {id} never recorded {kind}"));
        }
    }

    async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while !condition() {
            assert!(tokio::time::Instant::now() < deadline, "{what}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn alive(pid: i32) -> bool {
        std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|status| status.success())
    }

    async fn blocked(harness: &mut Harness, run: &AgentRun) -> String {
        let blocked = harness.until(&run.id, |run| run.pending.is_some()).await;
        assert_eq!(blocked.state, AgentRunState::Blocked);
        let ask = blocked.pending.expect("an ask");
        assert_eq!(ask.target, "touch side-effect");
        assert_eq!(ask.verb, AgentRunPermissionVerb::RunCommand);
        ask.ask_id
    }

    #[tokio::test]
    async fn an_allowed_ask_lets_the_tool_run_and_the_run_carries_on() {
        let mut harness = start();
        let stub = Stub::new("allow", &with_lines(ASK_AND_ACT));
        let run = harness.claude(&stub);
        let ask_id = blocked(&mut harness, &run).await;
        assert!(
            !stub.effect_happened(),
            "nothing runs while the ask is outstanding"
        );

        harness
            .manager
            .answer_permission(
                &run.id,
                &ask_id,
                AgentRunPermissionVerdict::AllowOnce,
                Actor::User,
            )
            .expect("answered");
        let waiting = harness
            .until(&run.id, |run| run.state == AgentRunState::Waiting)
            .await;
        assert!(waiting.pending.is_none());
        assert!(
            stub.effect_happened(),
            "the side effect is what an allowance means"
        );
        assert!(
            stub.answers()
                .contains(r#""response":{"behavior":"allow"}"#),
            "the allowance carried no input back: {}",
            stub.answers()
        );
        let kinds = harness.kinds(&run.id);
        assert!(kinds.contains(&"RUN_RESUMED"), "{kinds:?}");
        harness.manager.cancel(&run.id, Actor::User).unwrap();
        harness.ended(&run.id).await;
    }

    #[tokio::test]
    async fn a_denied_ask_does_not_run_and_the_run_carries_on() {
        let mut harness = start();
        let stub = Stub::new("deny", &with_lines(ASK_AND_ACT));
        let run = harness.claude(&stub);
        let ask_id = blocked(&mut harness, &run).await;

        harness
            .manager
            .answer_permission(
                &run.id,
                &ask_id,
                AgentRunPermissionVerdict::Deny,
                Actor::User,
            )
            .expect("answered");
        let waiting = harness
            .until(&run.id, |run| {
                run.state.is_terminal() || run.state == AgentRunState::Waiting
            })
            .await;
        assert_eq!(
            waiting.state,
            AgentRunState::Waiting,
            "a denial is not a failure"
        );
        assert!(!stub.effect_happened(), "a denied tool did not run");
        assert!(
            stub.answers()
                .contains(gritty_agent_claude::USER_DENIED_REASON),
            "the model is told the user said no: {}",
            stub.answers()
        );
        harness.manager.cancel(&run.id, Actor::User).unwrap();
        harness.ended(&run.id).await;
    }

    #[tokio::test]
    async fn cancelling_a_blocked_run_denies_then_kills_and_leaves_no_child() {
        let mut harness = start();
        // Deaf to SIGTERM, so the kill is the SIGKILL after the grace period —
        // and so there is time for the stub to log the denial it was sent.
        let stub = Stub::new(
            "cancel",
            &with_lines(&format!("trap '' TERM\n{ASK_AND_ACT}")),
        );
        let run = harness.claude(&stub);
        blocked(&mut harness, &run).await;
        let pid = stub.pid();
        assert!(alive(pid));

        harness.manager.cancel(&run.id, Actor::User).unwrap();
        let ended = harness.ended(&run.id).await;
        assert_eq!(ended.state, AgentRunState::Cancelled);
        assert!(ended.pending.is_none());

        eventually("the child was never told no", || {
            stub.answers().contains(r#""behavior":"deny""#)
        })
        .await;
        eventually("the child survived the cancel", || !alive(pid)).await;
        assert!(!stub.effect_happened(), "the tool never ran");
    }

    #[tokio::test]
    async fn shutdown_with_an_ask_outstanding_denies_and_interrupts() {
        let mut harness = start();
        let stub = Stub::new(
            "shutdown",
            &with_lines(&format!("trap '' TERM\n{ASK_AND_ACT}")),
        );
        let run = harness.claude(&stub);
        blocked(&mut harness, &run).await;
        let pid = stub.pid();

        harness.manager.shutdown(Duration::from_secs(5)).await;
        let ended = harness
            .manager
            .get(&run.id)
            .expect("the run is still listed");
        assert_eq!(ended.state, AgentRunState::Interrupted);
        assert!(ended.pending.is_none());
        eventually("the child was never told no", || {
            stub.answers().contains(r#""behavior":"deny""#)
        })
        .await;
        eventually("the child survived shutdown", || !alive(pid)).await;
        assert!(!stub.effect_happened());
    }

    #[tokio::test]
    async fn a_child_that_dies_while_asking_withdraws_the_ask() {
        let mut harness = start();
        let stub = Stub::new(
            "die",
            &with_lines("printf '%s\\n' \"$ASK\"\nIFS= read -r answer\nexit 3"),
        );
        let run = harness.claude(&stub);
        let ask_id = blocked(&mut harness, &run).await;
        // Let the child go: it reads nothing it acts on and exits.
        harness
            .manager
            .answer_permission(
                &run.id,
                &ask_id,
                AgentRunPermissionVerdict::Deny,
                Actor::User,
            )
            .ok();
        let ended = harness.ended(&run.id).await;
        assert_eq!(ended.state, AgentRunState::Failed);
        assert!(
            ended.pending.is_none(),
            "no ask outlives the process it was about"
        );
    }

    #[tokio::test]
    async fn a_child_that_dies_before_anyone_answers_withdraws_the_ask() {
        let mut harness = start();
        let stub = Stub::new(
            "die-unanswered",
            &with_lines("printf '%s\\n' \"$ASK\"\nexit 3"),
        );
        let run = harness.claude(&stub);
        let ended = harness.ended(&run.id).await;
        assert_eq!(ended.state, AgentRunState::Failed);
        assert!(ended.pending.is_none());
    }

    /// A second ask while the first is outstanding is the child breaking the
    /// protocol: it is denied on the spot, its echo does not resume the run,
    /// and the first ask is still the one on screen.
    #[tokio::test]
    async fn a_second_ask_is_denied_and_its_echo_does_not_unblock_the_run() {
        let mut harness = start();
        let second = ASK.replace("req-1", "req-2");
        let body = format!(
            r#"
SECOND='{second}'
printf '%s\n' "$ASK"
printf '%s\n' "$SECOND"
IFS= read -r answer
printf '%s\n' "$answer" >> "$LOG"
printf '%s\n' "$answer"
printf '%s\n' "$MARKER"
IFS= read -r answer
printf '%s\n' "$answer" >> "$LOG"
case "$answer" in *'"behavior":"allow"'*) touch "$EFFECT" ;; esac
printf '%s\n' "$answer"
printf '%s\n' "$RESULT"
while IFS= read -r line; do :; done
"#
        );
        let stub = Stub::new("second", &with_lines(&body));
        let run = harness.claude(&stub);
        let ask_id = blocked(&mut harness, &run).await;

        // The marker comes after the echo of the second ask's denial, so once
        // it is recorded that echo has been handled.
        harness.until_event(&run.id, "COMMAND_STARTED").await;
        let still = harness.manager.get(&run.id).unwrap();
        assert_eq!(
            still.state,
            AgentRunState::Blocked,
            "the echo did not resume it"
        );
        assert_eq!(
            still.pending.as_ref().map(|ask| ask.ask_id.as_str()),
            Some(ask_id.as_str())
        );
        let first_answer = stub.answers();
        assert!(first_answer.contains("req-2"), "{first_answer}");
        assert!(
            first_answer.contains(gritty_agent_claude::ONE_ASK_AT_A_TIME_REASON),
            "{first_answer}"
        );
        assert!(!harness.kinds(&run.id).contains(&"RUN_RESUMED"));

        // The first ask is still answerable, and answering it works.
        harness
            .manager
            .answer_permission(
                &run.id,
                &ask_id,
                AgentRunPermissionVerdict::AllowOnce,
                Actor::User,
            )
            .expect("answered");
        harness
            .until(&run.id, |run| run.state == AgentRunState::Waiting)
            .await;
        assert!(stub.effect_happened());
        harness.manager.cancel(&run.id, Actor::User).unwrap();
        harness.ended(&run.id).await;
    }

    /// Malformed and unsupported requests are refused without ever putting a
    /// question in front of the user.
    #[tokio::test]
    async fn malformed_and_unsupported_requests_are_refused_unasked() {
        let mut harness = start();
        let body = r#"
printf '%s\n' '{"type":"control_request","request_id":"bad-1","request":{"subtype":"can_use_tool"}}'
IFS= read -r answer
printf '%s\n' "$answer" >> "$LOG"
printf '%s\n' "$answer"
printf '%s\n' '{"type":"control_request","request_id":"odd-1","request":{"subtype":"mcp_message"}}'
IFS= read -r answer
printf '%s\n' "$answer" >> "$LOG"
printf '%s\n' "$MARKER"
printf '%s\n' "$RESULT"
while IFS= read -r line; do :; done
"#;
        let stub = Stub::new("malformed", &with_lines(body));
        let run = harness.claude(&stub);
        harness.until_event(&run.id, "COMMAND_STARTED").await;

        let answers = stub.answers();
        let mut lines = answers.lines();
        let malformed = lines.next().expect("the malformed ask was answered");
        assert!(
            malformed.contains("bad-1") && malformed.contains(r#""behavior":"deny""#),
            "{malformed}"
        );
        assert!(malformed.contains(gritty_agent_claude::MALFORMED_ASK_REASON));
        let unsupported = lines.next().expect("the unsupported request was answered");
        assert!(
            unsupported.contains("odd-1") && unsupported.contains(r#""subtype":"error""#),
            "{unsupported}"
        );
        assert!(
            !harness.states(&run.id).contains(&"BLOCKED"),
            "nobody was asked: {:?}",
            harness.states(&run.id)
        );
        assert!(!harness.kinds(&run.id).contains(&"RUN_RESUMED"));
        harness.manager.cancel(&run.id, Actor::User).unwrap();
        harness.ended(&run.id).await;
    }
}
