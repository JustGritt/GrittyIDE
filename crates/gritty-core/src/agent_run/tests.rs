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
