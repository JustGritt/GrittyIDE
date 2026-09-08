//! The run manager (T300-CORE, §§12-15): supervision, cancellation and the
//! bounded ring.
//!
//! # Why not the operation registry
//!
//! [`crate::operation::registry::OperationRegistry`] already supervises
//! cancellable work, and a run is *almost* one: it has an id, an owner, a
//! cancellation token, a correlation id and an ending. What does not fit is the
//! shape of the ending. `OperationStatus` has five states and no event stream;
//! [`AgentRunState`] has ten and a fourteen-variant vocabulary flowing beside
//! it. Squeezing one into the other would mean either flattening the state
//! machine this gate exists to freeze, or widening `OperationStatus` for a
//! single caller.
//!
//! So a run is supervised the way a terminal session is — its own registry,
//! its own lifetime — while reusing the operation model's *discipline*
//! verbatim, which is what hard rule 10 actually asks for:
//!
//! - one [`CancellationToken`] per run, a child of a root token, so shutdown
//!   cancels everything at once;
//! - the driver runs as **its own task**, so a panic inside it is contained
//!   here and ends the run `FAILED` rather than taking the process with it;
//! - a driver that returns without ending its run is an orphan, and an orphan
//!   is `FAILED` with an `Internal` error naming the gap — never left `RUNNING`;
//! - cancellation is cooperative *and* enforced: the token fires for a driver
//!   that watches it, and the supervising task aborts one that does not.
//!
//! # Who moves a run
//!
//! Only two things. The **driver** — the adapter's future, holding an
//! [`AgentRunContext`] — reports what the agent did and moves the run between
//! non-terminal states, and may end it [`AgentRunState::Finished`] or
//! [`AgentRunState::Failed`]. The **manager** ends it
//! [`AgentRunState::Cancelled`] or [`AgentRunState::Interrupted`], which is why
//! neither of those is reachable from the context: a run cannot cancel itself,
//! and a driver claiming to have been interrupted is claiming to know something
//! only the supervisor knows.
//!
//! Every transition goes through [`AgentRunState::check_transition`], so a
//! duplicated or out-of-order adapter step is refused with `Conflict` and
//! logged, changing nothing.
//!
//! # Never orphaned
//!
//! Three things end a run that nobody asked to end: its workstream is removed,
//! its workspace closes, or the app shuts down. All three fire the token and
//! all three guarantee a terminal state — including for a driver that ignores
//! cancellation entirely, which [`AgentRunManager::shutdown`] force-ends after
//! its grace period rather than leaving behind a row that claims to be running.
//!
//! The endings differ, and deliberately, because
//! [`AgentRunState::Interrupted`]'s own contract names two of these cases:
//! removing a workstream is a person asking for the work to stop
//! (`CANCELLED`), while a workspace closing or the app exiting is the ground
//! going out from under the run (`INTERRUPTED`). Both are "not orphaned", which
//! is what §15 requires; the state says which happened.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};

use super::ring::EventRing;
use super::state::AgentRunState;
use super::wire::{
    AgentRun, AgentRunActivity, AgentRunChangedPayload, AgentRunEvent, AgentRunEventLog,
    AgentRunProvider, MAX_REASON, MAX_RUN_EVENTS, clip,
};
use crate::event::{CoreEvent, EventBus, EventOrigin};
use crate::protocol::actor::Actor;
use crate::protocol::error::CoreError;
use crate::protocol::ids::{AgentRunId, CorrelationId, WorkspaceId, WorkstreamId};
use crate::protocol::time::Timestamp;
use crate::workspace::WorkspaceManager;

/// Runs allowed to be non-terminal in one workspace at a time.
///
/// The shape of `MAX_OPEN_TIMELINES`: a small named bound, enforced in the
/// core, refusing rather than queueing. The number is this task's judgement and
/// it is deliberately low. A run is an agent working *inside the user's
/// workspace*, and the priority order this project is built on puts
/// inspectability above orchestration power: four is already more than one
/// person can actually watch, and T410 exists because even four can need
/// attention at once.
///
/// The fake costs nothing, so this bound constrains nothing today — which is
/// exactly why it is set now rather than after a real provider makes the answer
/// expensive. **T310 must justify it against real process cost** (memory, token
/// spend, and how many worktrees T400 can hold) rather than inherit it.
pub const MAX_CONCURRENT_RUNS: usize = 4;

/// How many prompts may be waiting for one run before a caller is refused.
///
/// The shape of [`MAX_CONCURRENT_RUNS`] above: a small named bound, enforced in
/// the core, **refusing rather than queueing**. It lives here and not in
/// `wire.rs` because it is a supervision policy rather than a promise the
/// protocol makes — nothing on the wire reports how many prompts are pending,
/// which is exactly the difference that puts `MAX_RUN_EVENTS` on the other side
/// of that line.
///
/// Four, because a person types one prompt and waits for it. The queue exists
/// so a second thought typed mid-turn is not lost, not so a caller can fire a
/// hundred paid turns at a model nobody is watching. Worst case this bounds one
/// run's inbox at four times [`super::request::MAX_PROMPT_TEXT`].
pub const MAX_QUEUED_PROMPTS: usize = 4;

/// How many ended runs stay listable for rehydration, oldest pruned first.
///
/// A run is not persisted (ADR-021 decides whether it should be), so this is
/// the whole of a user's history within a session; `FINISHED_RETENTION` in the
/// operation registry keeps 200 much smaller records, and a run carries a ring
/// of up to [`MAX_RUN_EVENTS`] beside it.
pub const FINISHED_RETENTION: usize = 50;

/// How long a stop waits for one aborted driver before reporting the ending
/// anyway. A driver without await points cannot be stopped; it can only be
/// abandoned, and abandoning it silently is what would leave an orphan.
const CANCEL_JOIN_GRACE: Duration = Duration::from_millis(250);

/// Distinct unknown run ids whose events have been warned about. Bounded, since
/// the ids come from outside: a loop emitting for fresh unknown ids must not
/// grow this set without limit.
const MAX_UNKNOWN_WARNED: usize = 64;

/// What a run needs before it exists. **Not a wire type**: `runs.startFake`'s
/// request shape is T300-CMD's, and this is what its adapter builds one into.
#[derive(Debug, Clone)]
pub struct AgentRunSpec {
    /// Must be the open workspace; [`AgentRunManager::start`] checks.
    pub workspace_id: WorkspaceId,
    /// The work this run is doing. **The caller guarantees it exists** — the
    /// manager holds no database, so `runs.startFake`'s "unknown workstream is
    /// `NotFound`" is checked in the command adapter, before it gets here.
    pub workstream_id: WorkstreamId,
    pub provider: AgentRunProvider,
    /// Who asked for the run. Attribution for the `AgentRunChanged` that
    /// announces it; everything the run itself reports is attributed to
    /// [`Actor::Agent`].
    pub actor: Actor,
    pub correlation_id: CorrelationId,
}

/// An event and where it is going. Collected under the registry lock and
/// emitted after it is released: the bus takes its own lock and calls sinks
/// under it, and a sink that re-entered this manager while it held its lock
/// would deadlock.
type Emission = (EventOrigin, CoreEvent);

/// How a driver's task ended, before the manager decides what that means.
enum Outcome {
    /// The future returned `Ok`.
    Returned,
    /// The future returned `Err`, or panicked.
    Failed(CoreError),
    /// The task was aborted, which only the manager does.
    Stopped,
}

struct Entry {
    snapshot: AgentRun,
    ring: EventRing,
    token: CancellationToken,
    /// The supervising task. `None` once it has been joined or taken for
    /// shutdown; a non-terminal entry with no runner is one nobody will finish,
    /// which is why every stop path force-ends it.
    runner: Option<JoinHandle<()>>,
    /// Where a manager-initiated stop must land. Set before the token fires, so
    /// the supervising task knows whether it is reporting a cancellation or an
    /// interruption without having to be told twice.
    ending: Option<AgentRunState>,
    /// Where [`AgentRunManager::prompt`] puts text for a driver that is already
    /// running, or `None` for a run whose driver takes no input.
    ///
    /// `None` is the shape that lets a refusal say *why* rather than say
    /// nothing: every `FAKE` run has one, because `runs.startFake` is a
    /// scripted fiction with nothing to say anything to.
    inbox: Option<mpsc::Sender<RunInput>>,
    /// The provider session this run owns, once it has one.
    ///
    /// Deliberately **not** on [`AgentRun`]: that is the frozen wire shape, and
    /// a provider's own session id is the *observed* lane's identity concept
    /// (`docs/architecture/agent-runs.md`, "The line, first"). Putting it there
    /// would invite exactly the join across the two lanes that the split
    /// exists to forbid. It is internal supervision state, and
    /// [`AgentRunManager::owned_provider_sessions`] is the only way out.
    provider_session: Option<OwnedProviderSession>,
}

/// One provider session a run owns: what Gritty told the provider to call
/// itself, and the process answering to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedProviderSession {
    /// The id Gritty minted and passed to the child.
    pub session_id: String,
    /// The child's pid, once the driver has reported one. `None` before the
    /// process exists.
    pub pid: Option<u32>,
}

/// What may be handed to a driver that is already running.
///
/// An enum rather than a bare `String` because the next thing to travel this
/// channel is already named: `T310-ANSWER`'s permission verdict goes to the
/// same driver over the same pipe, and a second channel for it would be a
/// second lifetime to get wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunInput {
    /// `runs.prompt`: what the user typed, in full.
    ///
    /// **Not** the clipped preview `PROMPT_STARTED` records. `MAX_TEXT` bounds
    /// untrusted adapter output for display; silently truncating a person's
    /// prompt before paying a model to answer it would be worse than refusing
    /// it, which is why `PromptRunRequest` refuses against its own larger bound
    /// instead.
    Prompt { text: String },
}

/// The manager's end of a run's inbox.
///
/// Made by the caller, before the run exists, because
/// [`AgentRunManager::start`] takes an already-built `FnOnce` closure — so a
/// receiver produced *inside* `start` could never reach a driver constructed
/// *outside* it. The driver owns the other half by capturing it, exactly as
/// `fake_driver::drive` captures its script.
pub struct RunInbox {
    sender: mpsc::Sender<RunInput>,
}

/// The driver's end of a run's inbox.
///
/// Appears nowhere on [`AgentRunContext`] on purpose: a context is `Clone` and
/// a receiver is not, and keeping the context purely outbound is what makes
/// "who moves a run" answerable by reading one type.
pub struct RunInboxReceiver {
    receiver: mpsc::Receiver<RunInput>,
}

impl RunInbox {
    /// A fresh inbox, bounded at [`MAX_QUEUED_PROMPTS`].
    pub fn pair() -> (Self, RunInboxReceiver) {
        let (sender, receiver) = mpsc::channel(MAX_QUEUED_PROMPTS);
        (Self { sender }, RunInboxReceiver { receiver })
    }

    /// The prompt `runs.start` was given, queued before the run exists.
    ///
    /// Infallible by construction: the channel is empty and its bound is at
    /// least one. So the initial prompt and every later one travel exactly the
    /// same path, and the driver has one place prompts come from rather than a
    /// special case for the first.
    pub fn seed(&self, text: String) {
        debug_assert!(MAX_QUEUED_PROMPTS >= 1);
        let _ = self.sender.try_send(RunInput::Prompt { text });
    }
}

impl RunInboxReceiver {
    /// The next thing to hand the agent, or `None` once the run is gone.
    ///
    /// Cancellation-safe, so a driver may select on it: if another branch wins,
    /// nothing was taken from the channel. That is what keeps
    /// [`MAX_QUEUED_PROMPTS`] the only bound — a driver that took a message it
    /// could not act on and held it in a local would have made a second,
    /// unbounded queue.
    pub async fn recv(&mut self) -> Option<RunInput> {
        self.receiver.recv().await
    }
}

impl Entry {
    fn origin(&self, actor: Actor) -> EventOrigin {
        EventOrigin {
            workspace_id: Some(self.snapshot.workspace_id.clone()),
            correlation_id: Some(self.snapshot.correlation_id.clone()),
            actor,
        }
    }

    /// The run itself is the actor for everything it reports.
    fn agent(&self) -> Actor {
        Actor::agent(self.snapshot.id.clone())
    }

    fn changed(&self, actor: Actor) -> Emission {
        (
            self.origin(actor),
            CoreEvent::AgentRunChanged(AgentRunChangedPayload {
                run: self.snapshot.clone(),
            }),
        )
    }

    /// One coalesced batch, or nothing. Never one event per recorded event.
    fn batch(&mut self) -> Option<Emission> {
        let actor = self.agent();
        self.ring.take_batch(&self.snapshot.id).map(|payload| {
            (
                self.origin(actor),
                CoreEvent::AgentRunEventAppended(payload),
            )
        })
    }
}

#[derive(Default)]
struct Inner {
    entries: HashMap<AgentRunId, Entry>,
    /// Ended ids in ending order, pruned to [`FINISHED_RETENTION`].
    finished: VecDeque<AgentRunId>,
    /// Unknown ids already warned about, so a stale driver logs once and not
    /// once per event.
    warned: HashSet<AgentRunId>,
}

/// Owns every agent run in the process.
pub struct AgentRunManager {
    inner: Mutex<Inner>,
    events: Arc<EventBus>,
    workspaces: Arc<WorkspaceManager>,
    handle: Handle,
    /// Root token; every run's token is a child, so [`Self::shutdown`] cancels
    /// all of them at once.
    shutdown: CancellationToken,
}

impl AgentRunManager {
    /// `handle` is the shared tokio runtime (Tauri's); the core never creates
    /// its own.
    pub fn new(
        handle: Handle,
        events: Arc<EventBus>,
        workspaces: Arc<WorkspaceManager>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner::default()),
            events,
            workspaces,
            handle,
            shutdown: CancellationToken::new(),
        })
    }

    // -- starting -----------------------------------------------------------

    /// Register a run and schedule its driver.
    ///
    /// The run is created `STARTING` and `RUN_STARTED` is its first record —
    /// the manager's, not the adapter's, because "this run began" is a fact the
    /// supervisor knows and the protocol promises exactly once. Moving on to
    /// `RUNNING` is the driver's, because only it knows when the thing it
    /// drives is actually ready (`STARTING` exists for T310's shell-readiness
    /// window).
    ///
    /// `drive` is the adapter's future. It receives an [`AgentRunContext`] and
    /// returns `Ok(())` once it has ended its run; returning without ending it
    /// is an orphan and the manager fails the run rather than leaving it.
    ///
    /// Fails `NotFound` when the workspace is not open, and `Conflict` naming
    /// the count when the workspace already has [`MAX_CONCURRENT_RUNS`] runs.
    /// Neither creates a run.
    pub fn start<F, Fut>(
        self: &Arc<Self>,
        spec: AgentRunSpec,
        drive: F,
    ) -> Result<AgentRun, CoreError>
    where
        F: FnOnce(AgentRunContext) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), CoreError>> + Send + 'static,
    {
        self.start_inner(spec, None, None, drive)
    }

    /// [`Self::start`], for a driver that can be spoken to (T310-B).
    ///
    /// Everything `start` does, plus the manager's end of the run's inbox and
    /// the provider session the run owns. The driver's end of that inbox was
    /// moved into `drive` before this was called — the only arrangement
    /// `FnOnce` allows, and the reason the inbox is not on
    /// [`AgentRunContext`].
    ///
    /// `start` is left exactly as it was rather than widened to take an
    /// `Option`: `runs.startFake`, `fake_driver` and every existing test are
    /// written against that signature, and "the fake keeps working with no
    /// edit to `agent-fake`" is honoured more strongly by not editing
    /// `fake_driver.rs` either.
    pub fn start_with_inbox<F, Fut>(
        self: &Arc<Self>,
        spec: AgentRunSpec,
        inbox: RunInbox,
        session: OwnedProviderSession,
        drive: F,
    ) -> Result<AgentRun, CoreError>
    where
        F: FnOnce(AgentRunContext) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), CoreError>> + Send + 'static,
    {
        self.start_inner(spec, Some(inbox), Some(session), drive)
    }

    fn start_inner<F, Fut>(
        self: &Arc<Self>,
        spec: AgentRunSpec,
        inbox: Option<RunInbox>,
        session: Option<OwnedProviderSession>,
        drive: F,
    ) -> Result<AgentRun, CoreError>
    where
        F: FnOnce(AgentRunContext) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), CoreError>> + Send + 'static,
    {
        // A run against a workspace that is not open has nowhere to happen, and
        // this is also where a root that vanished is noticed.
        self.workspaces
            .require(&spec.workspace_id, &spec.correlation_id)?;

        let id = AgentRunId::new();
        let token = self.shutdown.child_token();
        let now = Timestamp::now();
        let mut emissions = Vec::new();
        {
            let mut inner = self.lock();
            let active = inner
                .entries
                .values()
                .filter(|entry| {
                    entry.snapshot.workspace_id == spec.workspace_id
                        && !entry.snapshot.state.is_terminal()
                })
                .count();
            if active >= MAX_CONCURRENT_RUNS {
                return Err(CoreError::conflict(format!(
                    "this workspace already has {active} agent runs, the most it may have at once is {MAX_CONCURRENT_RUNS}"
                ))
                .or_correlation(&spec.correlation_id));
            }

            let mut entry = Entry {
                snapshot: AgentRun {
                    id: id.clone(),
                    workstream_id: spec.workstream_id.clone(),
                    workspace_id: spec.workspace_id.clone(),
                    provider: spec.provider,
                    state: AgentRunState::Starting,
                    activity: None,
                    started_at: now,
                    updated_at: now,
                    finished_at: None,
                    cancellation_requested: false,
                    correlation_id: spec.correlation_id.clone(),
                    error: None,
                },
                ring: EventRing::new(),
                token: token.clone(),
                runner: None,
                ending: None,
                inbox: inbox.map(|inbox| inbox.sender),
                provider_session: session,
            };
            entry.ring.record(&id, now, AgentRunEvent::RunStarted);
            // The run has to exist for a consumer before its events mean
            // anything: a client drops a batch whose `runId` it does not know.
            emissions.push(entry.changed(spec.actor.clone()));
            if let Some(batch) = entry.batch() {
                emissions.push(batch);
            }
            inner.entries.insert(id.clone(), entry);
        }
        tracing::info!(
            run = %id,
            provider = spec.provider.as_str(),
            state = AgentRunState::Starting.as_str(),
            "agent run started"
        );
        self.emit_all(emissions);

        let runner = self
            .handle
            .spawn(Self::supervise(Arc::clone(self), id.clone(), token, drive));
        let snapshot = {
            let mut inner = self.lock();
            match inner.entries.get_mut(&id) {
                Some(entry) => {
                    entry.runner = Some(runner);
                    entry.snapshot.clone()
                }
                // Only reachable if a stop raced the spawn and already pruned
                // the entry; the caller is told what it was, and the supervisor
                // will find nothing to move.
                None => return Err(CoreError::internal("the run ended before it was scheduled")),
            }
        };
        Ok(snapshot)
    }

    /// The supervising task: runs the driver as its own task so a panic is
    /// contained, and turns however it ended into a terminal state.
    async fn supervise<F, Fut>(this: Arc<Self>, id: AgentRunId, token: CancellationToken, drive: F)
    where
        F: FnOnce(AgentRunContext) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), CoreError>> + Send + 'static,
    {
        let context = AgentRunContext {
            manager: Arc::clone(&this),
            run_id: id.clone(),
            token: token.clone(),
        };
        let mut driver = this.handle.spawn(async move { drive(context).await });
        let outcome = tokio::select! {
            biased;
            () = token.cancelled() => {
                // Cooperative *and* enforced: a driver watching the token stops
                // itself, and one that is not is aborted at its next await
                // point. Either way the run ends.
                driver.abort();
                if tokio::time::timeout(CANCEL_JOIN_GRACE, &mut driver).await.is_err() {
                    tracing::warn!(run = %id, "agent run driver did not stop within the cancel grace period");
                }
                Outcome::Stopped
            }
            joined = &mut driver => match joined {
                Ok(Ok(())) => Outcome::Returned,
                Ok(Err(error)) => Outcome::Failed(error),
                Err(join) if join.is_cancelled() => Outcome::Stopped,
                Err(join) => Outcome::Failed(CoreError::internal(format!(
                    "agent run driver panicked: {}",
                    panic_message(join)
                ))),
            },
        };
        this.settle(&id, outcome);
    }

    /// Turn a finished driver into a terminal run, unless the driver already
    /// ended it. This is the orphan guarantee: every path through here leaves
    /// the run terminal.
    fn settle(&self, id: &AgentRunId, outcome: Outcome) {
        let mut emissions = Vec::new();
        {
            let mut inner = self.lock();
            let (ending, error) = {
                let Some(entry) = inner.entries.get_mut(id) else {
                    tracing::debug!(run = %id, "the run a finished driver belonged to is gone");
                    return;
                };
                entry.runner = None;
                if entry.snapshot.state.is_terminal() {
                    // The driver ended its own run, which is the ordinary case.
                    return;
                }
                match (entry.ending, outcome) {
                    // A manager-initiated stop decides the ending, whatever the
                    // driver's task did on its way out.
                    (Some(ending), _) => (ending, None),
                    (None, Outcome::Failed(error)) => (AgentRunState::Failed, Some(error)),
                    (None, Outcome::Returned) => (
                        AgentRunState::Failed,
                        Some(CoreError::internal(
                            "the run's driver returned without ending the run: no finish, failure or cancellation was reported",
                        )),
                    ),
                    // Aborted with no ending recorded: something cancelled the
                    // task without going through this manager.
                    (None, Outcome::Stopped) => (AgentRunState::Cancelled, None),
                }
            };
            Self::end(&mut inner, id, ending, error, &mut emissions);
        }
        self.emit_all(emissions);
    }

    // -- the driver's side --------------------------------------------------

    /// Record one event: bounded, appended to the ring, coalesced into the next
    /// batch. Events for an unknown or ended run are dropped and logged once.
    ///
    /// `pub(super)` rather than public: an adapter reaches this through its
    /// [`AgentRunContext`], which knows which run it is. Nothing outside this
    /// module gets to name a run id and assert what it reported.
    pub(super) fn record(&self, id: &AgentRunId, event: AgentRunEvent) {
        let mut emissions = Vec::new();
        {
            let mut inner = self.lock();
            let Some(entry) = inner.entries.get_mut(id) else {
                Self::warn_unknown(&mut inner, id);
                return;
            };
            if entry.snapshot.state.is_terminal() {
                tracing::debug!(
                    run = %id,
                    kind = event.as_str(),
                    state = entry.snapshot.state.as_str(),
                    "dropping an event reported after the run ended"
                );
                return;
            }
            entry.ring.record(id, Timestamp::now(), event);
            // A driver that never flushes still cannot make the pending list
            // unbounded, and the batch that leaves is one event, not a hundred.
            if entry.ring.pending() >= MAX_RUN_EVENTS
                && let Some(batch) = entry.batch()
            {
                emissions.push(batch);
            }
        }
        self.emit_all(emissions);
    }

    /// Deliver whatever has been recorded since the last batch, as one event.
    /// Returns whether anything went out.
    fn flush(&self, id: &AgentRunId) -> bool {
        let mut emissions = Vec::new();
        {
            let mut inner = self.lock();
            if let Some(entry) = inner.entries.get_mut(id)
                && let Some(batch) = entry.batch()
            {
                emissions.push(batch);
            }
        }
        let flushed = !emissions.is_empty();
        self.emit_all(emissions);
        flushed
    }

    /// Move the run, set its activity, or both, in one `AgentRunChanged`.
    ///
    /// `state` is checked against [`AgentRunState::allowed_transitions`], so a
    /// duplicated or out-of-order adapter step is `Conflict` and changes
    /// nothing. Terminal states are refused here: [`Self::finish`] and
    /// [`Self::fail`] are the driver's two endings, and `CANCELLED` /
    /// `INTERRUPTED` belong to the manager.
    fn advance(
        &self,
        id: &AgentRunId,
        state: Option<AgentRunState>,
        activity: Option<Option<AgentRunActivity>>,
    ) -> Result<AgentRun, CoreError> {
        let mut emissions = Vec::new();
        let snapshot = {
            let mut inner = self.lock();
            let Some(entry) = inner.entries.get_mut(id) else {
                return Err(CoreError::not_found(format!(
                    "agent run {id} does not exist"
                )));
            };
            if let Some(next) = state {
                if next.is_terminal() {
                    return Err(CoreError::conflict(format!(
                        "a run's driver does not move it to {}; it finishes, fails, or is cancelled by Gritty",
                        next.as_str()
                    )));
                }
                if entry.snapshot.cancellation_requested {
                    // Once a stop was asked for, the only ending is the one the
                    // manager chose. A driver still reporting progress is not
                    // an error, but it does not get to un-cancel itself.
                    return Err(CoreError::conflict(format!(
                        "run {id} is stopping; it cannot become {}",
                        next.as_str()
                    )));
                }
                if let Err(refused) = entry.snapshot.state.check_transition(next) {
                    tracing::warn!(
                        run = %id,
                        from = entry.snapshot.state.as_str(),
                        to = next.as_str(),
                        "refused an out-of-order agent run transition"
                    );
                    return Err(refused);
                }
            }
            let bounded = activity.map(|value| value.map(AgentRunActivity::bounded));
            let unchanged = state.is_none_or(|next| next == entry.snapshot.state)
                && bounded
                    .as_ref()
                    .is_none_or(|value| *value == entry.snapshot.activity);
            if unchanged {
                // Nothing to say. An adapter re-reporting the same activity
                // three times a second is three events the renderer never needs
                // (rule 8).
                return Ok(entry.snapshot.clone());
            }
            if let Some(next) = state {
                entry.snapshot.state = next;
            }
            if let Some(value) = bounded {
                entry.snapshot.activity = value;
            }
            entry.snapshot.updated_at = Timestamp::now();
            tracing::debug!(
                run = %id,
                state = entry.snapshot.state.as_str(),
                "agent run changed"
            );
            // Events first, so the batch that explains the change arrives
            // before the change itself.
            if let Some(batch) = entry.batch() {
                emissions.push(batch);
            }
            let actor = entry.agent();
            emissions.push(entry.changed(actor));
            entry.snapshot.clone()
        };
        self.emit_all(emissions);
        Ok(snapshot)
    }

    /// End the run `FINISHED`, recording `RUN_FINISHED` with the adapter's
    /// summary first so the log closes with the ending it describes.
    fn finish(&self, id: &AgentRunId, summary: Option<String>) -> Result<AgentRun, CoreError> {
        self.close(
            id,
            AgentRunState::Finished,
            Some(AgentRunEvent::RunFinished { summary }),
            None,
        )
    }

    /// End the run `FAILED`. [`Self::end`] records the `RUN_FAILED` that closes
    /// the log, so a failure the *manager* decided on — an orphan, a panic —
    /// closes it the same way a driver's own does.
    fn fail(&self, id: &AgentRunId, error: CoreError) -> Result<AgentRun, CoreError> {
        self.close(id, AgentRunState::Failed, None, Some(error))
    }

    fn close(
        &self,
        id: &AgentRunId,
        ending: AgentRunState,
        event: Option<AgentRunEvent>,
        error: Option<CoreError>,
    ) -> Result<AgentRun, CoreError> {
        let mut emissions = Vec::new();
        let snapshot = {
            let mut inner = self.lock();
            let (ending, error) = {
                let Some(entry) = inner.entries.get_mut(id) else {
                    return Err(CoreError::not_found(format!(
                        "agent run {id} does not exist"
                    )));
                };
                if let Err(refused) = entry.snapshot.state.check_transition(ending) {
                    tracing::warn!(
                        run = %id,
                        from = entry.snapshot.state.as_str(),
                        to = ending.as_str(),
                        "refused an out-of-order agent run ending"
                    );
                    return Err(refused);
                }
                // A stop already decided how this run ends; the driver's own
                // ending is too late, exactly as a completing operation's is
                // once cancellation was requested.
                match entry.ending {
                    Some(chosen) => (chosen, None),
                    None => {
                        if let Some(event) = event {
                            entry.ring.record(id, Timestamp::now(), event);
                        }
                        (ending, error)
                    }
                }
            };
            Self::end(&mut inner, id, ending, error, &mut emissions);
            inner
                .entries
                .get(id)
                .map(|entry| entry.snapshot.clone())
                .ok_or_else(|| {
                    CoreError::internal(format!("agent run {id} vanished while ending"))
                })?
        };
        self.emit_all(emissions);
        Ok(snapshot)
    }

    /// The one place a run becomes terminal. Records nothing and refuses
    /// nothing: callers have already decided and checked.
    fn end(
        inner: &mut Inner,
        id: &AgentRunId,
        ending: AgentRunState,
        error: Option<CoreError>,
        emissions: &mut Vec<Emission>,
    ) {
        let Some(entry) = inner.entries.get_mut(id) else {
            return;
        };
        let now = Timestamp::now();
        // A failure closes its log with `RUN_FAILED`, whoever decided on it: a
        // driver that failed itself, a driver that returned without ending its
        // run, or one that panicked. A consumer reading only the event stream
        // must not have to infer the ending from the absence of one.
        if ending == AgentRunState::Failed
            && let Some(error) = error.clone()
        {
            entry.ring.record(
                id,
                now,
                AgentRunEvent::RunFailed {
                    error: error.or_correlation(&entry.snapshot.correlation_id),
                },
            );
        }
        entry.snapshot.state = ending;
        entry.snapshot.finished_at = Some(now);
        entry.snapshot.updated_at = now;
        // Nothing is happening any more, and an activity left behind reads as
        // if something still were.
        entry.snapshot.activity = None;
        let correlation = entry.snapshot.correlation_id.clone();
        entry.snapshot.error = error.map(|error| {
            // A failure message is routinely built *from* what an adapter said,
            // so it is bounded like every other string that crosses the
            // protocol — `AgentRunEvent::bounded` does the same for the
            // `RUN_FAILED` record beside it.
            let error = error.or_correlation(&correlation);
            CoreError {
                message: clip(&error.message, MAX_REASON),
                ..error
            }
        });
        entry.runner = None;
        tracing::info!(
            run = %id,
            state = ending.as_str(),
            provider = entry.snapshot.provider.as_str(),
            duration_ms = now.as_millis().saturating_sub(entry.snapshot.started_at.as_millis()),
            "agent run ended"
        );
        if let Some(batch) = entry.batch() {
            emissions.push(batch);
        }
        let actor = entry.agent();
        emissions.push(entry.changed(actor));
        inner.finished.push_back(id.clone());
        Self::prune(inner);
    }

    fn prune(inner: &mut Inner) {
        while inner.finished.len() > FINISHED_RETENTION {
            if let Some(old) = inner.finished.pop_front() {
                inner.entries.remove(&old);
            }
        }
    }

    /// An event for a run nobody knows about: dropped, never forwarded, and
    /// logged once rather than once per event.
    fn warn_unknown(inner: &mut Inner, id: &AgentRunId) {
        if inner.warned.len() >= MAX_UNKNOWN_WARNED {
            // The ids come from outside; the set that remembers them does not
            // get to grow without limit.
            inner.warned.clear();
        }
        if inner.warned.insert(id.clone()) {
            tracing::warn!(run = %id, "dropping an event for a run that does not exist");
        }
    }

    // -- the command surface ------------------------------------------------

    /// `runs.cancel`: cooperative, idempotent, and enforced.
    ///
    /// - an unknown id is `NotFound`;
    /// - a run that already ended is a no-op returning its snapshot unchanged;
    /// - a running or blocked run has its token fired and ends `CANCELLED`,
    ///   which is why the snapshot returned here still shows the state the run
    ///   was in with `cancellationRequested` set. The `AgentRunChanged` that
    ///   follows carries the ending.
    ///
    /// Cancellation reaches a run in **any** non-terminal state, including one
    /// blocked on a permission it will never get: the supervising task aborts
    /// the driver, so a driver parked on an await that never resolves is ended
    /// rather than waited for.
    pub fn cancel(&self, id: &AgentRunId, actor: Actor) -> Result<AgentRun, CoreError> {
        let mut emissions = Vec::new();
        let snapshot = {
            let mut inner = self.lock();
            if !inner.entries.contains_key(id) {
                return Err(CoreError::not_found(format!(
                    "agent run {id} does not exist"
                )));
            }
            match Self::request_stop(
                &mut inner,
                id,
                AgentRunState::Cancelled,
                &actor,
                &mut emissions,
            ) {
                Some(snapshot) => snapshot,
                None => inner.entries[id].snapshot.clone(),
            }
        };
        self.emit_all(emissions);
        Ok(snapshot)
    }

    /// Ask one run to stop, landing in `ending`.
    ///
    /// Returns the snapshot when the run was live, `None` when it had already
    /// ended (in which case nothing changed — the idempotent no-op).
    fn request_stop(
        inner: &mut Inner,
        id: &AgentRunId,
        ending: AgentRunState,
        actor: &Actor,
        emissions: &mut Vec<Emission>,
    ) -> Option<AgentRun> {
        let (abandoned, chosen) = {
            let entry = inner.entries.get_mut(id)?;
            if entry.snapshot.state.is_terminal() {
                return None;
            }
            if entry.ending.is_none() {
                entry.ending = Some(ending);
                entry.snapshot.cancellation_requested = true;
                entry.snapshot.updated_at = Timestamp::now();
                entry.token.cancel();
                tracing::info!(run = %id, ending = ending.as_str(), "agent run stop requested");
                emissions.push(entry.changed(actor.clone()));
            }
            (entry.runner.is_none(), entry.ending.unwrap_or(ending))
        };
        if abandoned {
            // Nobody is left to notice the token: end it here rather than leave
            // a row that claims to be running.
            Self::end(inner, id, chosen, None, emissions);
        }
        inner.entries.get(id).map(|entry| entry.snapshot.clone())
    }

    pub fn get(&self, id: &AgentRunId) -> Option<AgentRun> {
        self.lock()
            .entries
            .get(id)
            .map(|entry| entry.snapshot.clone())
    }

    /// `runs.prompt`: hand text to a driver that is already running.
    ///
    /// **Refuses rather than queues at every edge, and every refusal names what
    /// is in the way** — the state, the provider, or the bound. Nothing here is
    /// optimistic: accepting a prompt records no event and changes no state, so
    /// a caller that got `Ok` has learned the text was *queued*, not that the
    /// agent has seen it. `PROMPT_STARTED` is the run's own confirmation and it
    /// comes from the driver, when the CLI replays the message back.
    ///
    /// The state check and the send are one critical section: without that a
    /// run could end between the two and text would be queued for a corpse.
    /// `try_send` calls no user code and at most schedules a wake, so holding
    /// the lock across it cannot re-enter this manager — and there is nothing
    /// to emit, so the emit-after-unlock discipline is not in play.
    pub fn prompt(
        &self,
        id: &AgentRunId,
        text: String,
        actor: Actor,
    ) -> Result<AgentRun, CoreError> {
        let mut inner = self.lock();
        let Some(entry) = inner.entries.get_mut(id) else {
            return Err(CoreError::not_found(format!(
                "agent run {id} does not exist"
            )));
        };
        let state = entry.snapshot.state;
        if state.is_terminal() {
            return Err(CoreError::conflict(format!(
                "run already ended {}; it cannot be prompted",
                state.as_str()
            )));
        }
        if entry.snapshot.cancellation_requested {
            return Err(CoreError::conflict(format!(
                "run {id} is stopping; it cannot be prompted"
            )));
        }
        if !state.accepts_input() {
            return Err(CoreError::conflict(format!(
                "run is {}; it is not accepting a prompt",
                state.as_str()
            )));
        }
        let Some(inbox) = entry.inbox.as_ref() else {
            return Err(CoreError::conflict(format!(
                "a {} run has no prompt channel",
                entry.snapshot.provider.as_str()
            )));
        };
        match inbox.try_send(RunInput::Prompt { text }) {
            Ok(()) => {
                // The id, the actor and a length — never the text (see the
                // module's tracing rule).
                tracing::info!(run = %id, ?actor, "a prompt was queued for a run");
                Ok(entry.snapshot.clone())
            }
            Err(mpsc::error::TrySendError::Full(_)) => Err(CoreError::conflict(format!(
                "this run already has {MAX_QUEUED_PROMPTS} prompts waiting; wait for it to answer"
            ))),
            // The receiver is gone: the driver's future returned or panicked
            // and `settle` has not taken this lock yet. Refuse and let it
            // decide the ending — racing it from here would be two paths
            // ending one run.
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!(run = %id, "a prompt arrived for a run whose driver has stopped");
                Err(CoreError::conflict(format!(
                    "run {id} is no longer accepting input: its driver has stopped"
                )))
            }
        }
    }

    /// Every provider session this manager minted and is still answerable for.
    ///
    /// The composition root turns this into the exclusion set agent discovery
    /// is given, so Gritty's own child is not also reported as a session
    /// somebody else owns (T310-B §8). It returns plain data and names nothing
    /// from `agent_session` or `gritty_agents`, which is what keeps the two
    /// lanes from importing each other.
    ///
    /// **Ended-but-unpruned runs are included on purpose.** Their child may
    /// still be dying and its transcript certainly still exists, and both are
    /// things Gritty owned rather than things it observed. Over-excluding for a
    /// while is the safe direction; under-excluding is what draws the same
    /// agent twice. Bounded by `FINISHED_RETENTION` + [`MAX_CONCURRENT_RUNS`]
    /// by construction, and forgotten only by `prune` — never persisted, since
    /// a set that outlived the process would be a permanent blacklist
    /// (ADR-021 §5).
    pub fn owned_provider_sessions(&self) -> Vec<OwnedProviderSession> {
        self.lock()
            .entries
            .values()
            .filter_map(|entry| entry.provider_session.clone())
            .collect()
    }

    /// `runs.list`: every known run of one workspace, oldest first.
    pub fn list(&self, workspace_id: &WorkspaceId) -> Vec<AgentRun> {
        let mut runs: Vec<AgentRun> = self
            .lock()
            .entries
            .values()
            .filter(|entry| entry.snapshot.workspace_id == *workspace_id)
            .map(|entry| entry.snapshot.clone())
            .collect();
        runs.sort_by(|a, b| (a.started_at, &a.id).cmp(&(b.started_at, &b.id)));
        runs
    }

    /// `runs.events`: the whole ring, for rehydration after a reload.
    ///
    /// The undelivered batch is forgotten, because the caller has those records
    /// in this response; a client that also listens converges rather than
    /// seeing them twice.
    pub fn events(&self, id: &AgentRunId) -> Result<AgentRunEventLog, CoreError> {
        let mut inner = self.lock();
        let entry = inner
            .entries
            .get_mut(id)
            .ok_or_else(|| CoreError::not_found(format!("agent run {id} does not exist")))?;
        let log = entry.ring.snapshot(id);
        entry.ring.forget_pending();
        Ok(log)
    }

    // -- never orphaned -----------------------------------------------------

    /// React to `WorkstreamRemoved`: the work a run was doing is gone, so the
    /// run is cancelled rather than left pointing at nothing.
    ///
    /// An explicit reaction, called by the composition root, because there is
    /// no foreign key to cascade: a run is not persisted and a workstream is.
    pub fn cancel_for_workstream(&self, workstream_id: &WorkstreamId) -> Vec<AgentRun> {
        self.stop_matching(
            |run| run.workstream_id == *workstream_id,
            AgentRunState::Cancelled,
            "its workstream was removed",
        )
    }

    /// React to `WorkspaceClosed`: the ground the run was standing on is gone,
    /// which is `INTERRUPTED` rather than `CANCELLED` — nobody asked the run to
    /// stop.
    pub fn interrupt_for_workspace(&self, workspace_id: &WorkspaceId) -> Vec<AgentRun> {
        self.stop_matching(
            |run| run.workspace_id == *workspace_id,
            AgentRunState::Interrupted,
            "its workspace closed",
        )
    }

    fn stop_matching(
        &self,
        matches: impl Fn(&AgentRun) -> bool,
        ending: AgentRunState,
        reason: &'static str,
    ) -> Vec<AgentRun> {
        let mut emissions = Vec::new();
        let stopped = {
            let mut inner = self.lock();
            let ids: Vec<AgentRunId> = inner
                .entries
                .values()
                .filter(|entry| !entry.snapshot.state.is_terminal() && matches(&entry.snapshot))
                .map(|entry| entry.snapshot.id.clone())
                .collect();
            ids.iter()
                .filter_map(|id| {
                    tracing::info!(run = %id, ending = ending.as_str(), reason, "stopping agent run");
                    Self::request_stop(&mut inner, id, ending, &Actor::System, &mut emissions)
                })
                .collect()
        };
        self.emit_all(emissions);
        stopped
    }

    /// Cancel every live run and wait up to `grace` for their supervisors.
    ///
    /// The guarantee terminals already make ("`Core::shutdown` kills every
    /// session before the session writer flushes") applied to runs: when this
    /// returns, **no run is non-terminal**. A driver that ignored its token
    /// long enough to miss the grace period is abandoned and its run is ended
    /// `INTERRUPTED` here, because a row that still claims to be running is the
    /// orphan the whole state machine exists to prevent.
    pub async fn shutdown(&self, grace: Duration) {
        let mut emissions = Vec::new();
        let runners: Vec<(AgentRunId, Option<JoinHandle<()>>)> = {
            let mut inner = self.lock();
            let ids: Vec<AgentRunId> = inner
                .entries
                .values()
                .filter(|entry| !entry.snapshot.state.is_terminal())
                .map(|entry| entry.snapshot.id.clone())
                .collect();
            for id in &ids {
                if let Some(entry) = inner.entries.get_mut(id)
                    && entry.ending.is_none()
                {
                    entry.ending = Some(AgentRunState::Interrupted);
                    entry.snapshot.cancellation_requested = true;
                }
            }
            ids.into_iter()
                .map(|id| {
                    let runner = inner
                        .entries
                        .get_mut(&id)
                        .and_then(|entry| entry.runner.take());
                    (id, runner)
                })
                .collect()
        };
        // One root token cancels every child at once.
        self.shutdown.cancel();

        let deadline = Instant::now() + grace;
        for (id, runner) in runners {
            let Some(runner) = runner else {
                continue;
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            if tokio::time::timeout(remaining, runner).await.is_err() {
                tracing::warn!(run = %id, "agent run did not stop before the shutdown deadline");
            }
        }

        {
            let mut inner = self.lock();
            let stragglers: Vec<AgentRunId> = inner
                .entries
                .values()
                .filter(|entry| !entry.snapshot.state.is_terminal())
                .map(|entry| entry.snapshot.id.clone())
                .collect();
            for id in stragglers {
                tracing::warn!(run = %id, "force-ending an agent run whose driver never stopped");
                Self::end(
                    &mut inner,
                    &id,
                    AgentRunState::Interrupted,
                    None,
                    &mut emissions,
                );
            }
        }
        self.emit_all(emissions);
    }

    // -- plumbing -----------------------------------------------------------

    /// Kill a run's supervising task and forget it, so nothing will ever report
    /// that run's ending.
    ///
    /// This is the force-kill the recovery guarantee has to survive (rule 12):
    /// it reproduces a supervisor that died with the run still live, which
    /// cannot be provoked from outside and which every stop path has to handle
    /// rather than leaving a row that claims to be running.
    #[cfg(test)]
    pub(super) fn abandon_supervisor(&self, id: &AgentRunId) {
        if let Some(entry) = self.lock().entries.get_mut(id)
            && let Some(runner) = entry.runner.take()
        {
            runner.abort();
        }
    }

    fn emit_all(&self, emissions: Vec<Emission>) {
        for (origin, event) in emissions {
            self.events.emit(&origin, event);
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The driver's handle on its run: identity, cancellation, events and state.
///
/// Cloneable and `Send`, so an adapter may hand it to whatever is doing the
/// work. It reaches exactly what an adapter is allowed to reach — which is why
/// there is no `cancel` on it and no way to reach `CANCELLED` or `INTERRUPTED`:
/// those are the supervisor's answers, not the supervised thing's.
///
/// Every string that arrives through here is bounded by the core before it is
/// recorded ([`AgentRunEvent::bounded`]), so an adapter neither has to bound
/// nor is trusted to have done it (rule 13).
#[derive(Clone)]
pub struct AgentRunContext {
    manager: Arc<AgentRunManager>,
    run_id: AgentRunId,
    token: CancellationToken,
}

impl AgentRunContext {
    pub fn run_id(&self) -> &AgentRunId {
        &self.run_id
    }

    /// The run as it stands, or `None` once it has been pruned.
    pub fn snapshot(&self) -> Option<AgentRun> {
        self.manager.get(&self.run_id)
    }

    /// Report the child's pid, once the driver has spawned one (T310-B §8).
    ///
    /// Outbound like everything else here, and it exists for one reason: the
    /// exclusion set keyed on a provider session id has a hole, because
    /// `discover_processes` has to *recover* that id from a command line `ps`
    /// may have truncated. A pid is exact and needs no parsing, so the two keys
    /// together close what either alone leaves open.
    ///
    /// A run with no provider session — every `FAKE` run — ignores this.
    pub fn set_child_pid(&self, pid: u32) {
        let mut inner = self.manager.lock();
        if let Some(entry) = inner.entries.get_mut(&self.run_id)
            && let Some(session) = entry.provider_session.as_mut()
        {
            session.pid = Some(pid);
        }
    }

    /// Cooperative cancellation: a driver checks this between units of work.
    /// Not checking it is survivable — the supervisor aborts the driver — but
    /// checking it is how a driver stops cleanly.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Resolves once a stop was requested; use it in `select!` beside an await.
    pub fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.token.cancelled()
    }

    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Report one event. Bounded, appended to the run's ring, and coalesced
    /// into the next batch rather than emitted on its own.
    pub fn record(&self, event: AgentRunEvent) {
        self.manager.record(&self.run_id, event);
    }

    /// Report several events at once, then deliver them as one batch. The call
    /// an adapter makes at the end of a step (rule 8).
    pub fn record_all(&self, events: impl IntoIterator<Item = AgentRunEvent>) {
        for event in events {
            self.manager.record(&self.run_id, event);
        }
        self.flush();
    }

    /// Deliver what has been recorded so far as one `AgentRunEventAppended`,
    /// returning whether anything went out.
    ///
    /// A state or activity change flushes on its own, so a driver only needs
    /// this at a step boundary that changed neither.
    pub fn flush(&self) -> bool {
        self.manager.flush(&self.run_id)
    }

    /// Move the run to a non-terminal state, leaving its activity alone.
    ///
    /// `Conflict` when the transition is not in
    /// [`AgentRunState::allowed_transitions`] — which is what a duplicated or
    /// out-of-order step looks like from here — and when a stop has already
    /// been requested.
    pub fn transition(&self, next: AgentRunState) -> Result<AgentRun, CoreError> {
        self.manager.advance(&self.run_id, Some(next), None)
    }

    /// Replace what the run is doing. `None` says it is doing nothing in
    /// particular. Reporting the same activity twice emits nothing.
    pub fn set_activity(&self, activity: Option<AgentRunActivity>) -> Result<AgentRun, CoreError> {
        self.manager.advance(&self.run_id, None, Some(activity))
    }

    /// State and activity together, in one `AgentRunChanged` — the common case
    /// (`BLOCKED` + `REQUESTING_PERMISSION`), and one event rather than two.
    ///
    /// `next` must be a real transition: an adapter whose *activity* changed
    /// while the run stayed `RUNNING` calls [`Self::set_activity`], because a
    /// state that did not change is a duplicated step and is refused as one.
    pub fn advance(
        &self,
        next: AgentRunState,
        activity: Option<AgentRunActivity>,
    ) -> Result<AgentRun, CoreError> {
        self.manager
            .advance(&self.run_id, Some(next), Some(activity))
    }

    /// End the run, having done what it was asked. Records `RUN_FINISHED` with
    /// the summary first.
    pub fn finish(&self, summary: Option<String>) -> Result<AgentRun, CoreError> {
        self.manager.finish(&self.run_id, summary)
    }

    /// End the run badly. Records `RUN_FAILED` with the error first.
    pub fn fail(&self, error: CoreError) -> Result<AgentRun, CoreError> {
        self.manager.fail(&self.run_id, error)
    }
}

/// The panic payload as a string. Same shape as the operation registry's, kept
/// local rather than made public there: this is eight lines of `downcast_ref`,
/// and widening another module's surface for it would be the larger change.
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
