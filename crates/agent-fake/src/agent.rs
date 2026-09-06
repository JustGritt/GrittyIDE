//! The driver: a script, a logical clock, and a phase that only ever moves
//! because the caller advanced it.
//!
//! # Why there is no clock in here
//!
//! Nothing in this file reads `SystemTime`, `Instant` or a timer, and nothing
//! sleeps. [`FakeClock`] is a counter: it starts at a number the caller chose
//! and adds a fixed step per recorded action. Two runs of the same script
//! therefore produce byte-identical [`FakeObservation`]s, timestamps included,
//! which is what makes "assert the sequence twice" a real assertion rather than
//! an approximate one. A fake that is *almost* deterministic is worse than
//! none, because every test downstream inherits its flakiness.
//!
//! # Why there is an `Arc<Mutex<…>>` in a synchronous type
//!
//! Two reasons, and neither is concurrency for its own sake.
//!
//! 1. §11 requires that a run whose driving task is **dropped** still resolves
//!    to a terminal phase. A `Drop` impl cannot hand a value back to anyone, so
//!    the resolution has to land somewhere the supervisor still holds — which
//!    is [`FakeAgentHandle`].
//! 2. §10 requires that a pending permission or decision can be answered *out
//!    of band*, which in the running app means from a different task than the
//!    one advancing the script.
//!
//! The lock is uncontended in tests and held only for the length of one step.
//! It is also poison-tolerant: a panic in the driving task is exactly the case
//! this module has to survive, so a poisoned lock is recovered from rather than
//! propagated (`PoisonError::into_inner`).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::error::FakeError;
use crate::phase::FakePhase;
use crate::script::{FakeAction, FakeScript, FakeStep};

/// Why a run that ran out of script ended.
///
/// A `&'static str` rather than a formatted message so the record stays
/// reproducible; the core wraps it in `CoreError::internal` and the run ends
/// `FAILED`.
pub const ORPHANED_UNTERMINATED: &str = "the fake script ended without a finish or fail step, so the run was resolved rather than left running";

/// Why a run whose driver went away ended.
///
/// The panic-and-drop case (§11): the thing driving the run stopped existing,
/// and a run nobody is driving is an orphan. The core wraps it in
/// `CoreError::internal` and the run ends `FAILED` — deliberately not
/// `INTERRUPTED`, which is reserved for a supervisor deciding it can no longer
/// watch a run that is otherwise fine.
pub const ORPHANED_DROPPED: &str =
    "the fake agent's driver was dropped before the run reached a terminal step";

/// Where the fake's timestamps start: 1 January 2026, as epoch milliseconds.
///
/// A real-looking value rather than `0`, so a timestamp that leaks into a UI
/// during the manual pass does not render as 1970 and send someone hunting a
/// bug that is not there.
pub const DEFAULT_START_MILLIS: u64 = 1_767_225_600_000;

/// How far the logical clock moves per recorded action.
pub const DEFAULT_TICK_MILLIS: u64 = 250;

/// A counter that looks like a clock.
///
/// Not a clock. It advances only when an action is recorded, which is the whole
/// of why this crate has no timing-dependent behaviour to be flaky about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FakeClock {
    next_millis: u64,
    tick_millis: u64,
}

impl FakeClock {
    pub const fn new(start_millis: u64, tick_millis: u64) -> Self {
        Self {
            next_millis: start_millis,
            tick_millis,
        }
    }

    /// The next millisecond value, and then step forward. Saturating, so a
    /// clock started near `u64::MAX` stops rather than wrapping into the past.
    fn tick(&mut self) -> u64 {
        let now = self.next_millis;
        self.next_millis = self.next_millis.saturating_add(self.tick_millis);
        now
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new(DEFAULT_START_MILLIS, DEFAULT_TICK_MILLIS)
    }
}

/// One thing that happened, and the phase it left the run in.
///
/// The record a supervisor translates: [`FakeAction::run_events`] says which
/// `AgentRunEvent`s it becomes, [`FakePhase::run_state`] says which
/// `AgentRunState` the run is now in, and `at_millis` is the `Timestamp` for
/// both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeObservation {
    /// Strictly increasing from zero, per run. Carries no meaning beyond order.
    pub seq: u64,
    /// Epoch milliseconds from the logical clock — never wall-clock time.
    pub at_millis: u64,
    pub action: FakeAction,
    /// The phase **after** the action.
    pub phase: FakePhase,
}

/// What one call to [`FakeAgent::advance`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeAdvance {
    /// A step ran, or the run resolved itself. Something changed.
    Moved(FakeObservation),
    /// The step was refused and nothing changed — a duplicated or out-of-order
    /// script. The step is consumed, so advancing again makes progress; the
    /// refusal is reported here rather than swallowed, and counted by
    /// [`FakeAgent::rejected`].
    Rejected { step: FakeStep, reason: FakeError },
    /// The run is stopped on a permission or a decision. **Nothing was
    /// consumed**: the script does not move again until someone answers out of
    /// band (§10).
    Blocked(FakePhase),
    /// The run is over. Every further call returns this same value.
    Finished(FakePhase),
}

impl FakeAdvance {
    /// The observation, when something happened.
    pub fn observation(&self) -> Option<&FakeObservation> {
        match self {
            Self::Moved(observation) => Some(observation),
            _ => None,
        }
    }

    pub const fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked(_))
    }

    pub const fn is_finished(&self) -> bool {
        matches!(self, Self::Finished(_))
    }
}

/// Everything about one scripted run, behind one lock.
#[derive(Debug)]
struct Inner {
    script: FakeScript,
    cursor: usize,
    phase: FakePhase,
    clock: FakeClock,
    next_seq: u64,
    rejected: u32,
    observations: Vec<FakeObservation>,
    /// The path of the last `proposeWrite` that has not been written yet.
    pending_write: Option<String>,
    /// The command of the last `command`/`test` that has not finished yet.
    pending_command: Option<String>,
}

impl Inner {
    fn new(script: FakeScript, clock: FakeClock) -> Self {
        Self {
            script,
            cursor: 0,
            phase: FakePhase::Preparing,
            clock,
            next_seq: 0,
            rejected: 0,
            observations: Vec::new(),
            pending_write: None,
            pending_command: None,
        }
    }

    fn record(&mut self, action: FakeAction, phase: FakePhase) -> FakeObservation {
        let seq = self.next_seq;
        self.next_seq += 1;
        let at_millis = self.clock.tick();
        self.phase = phase;
        let observation = FakeObservation {
            seq,
            at_millis,
            action,
            phase,
        };
        self.observations.push(observation.clone());
        observation
    }

    /// Check a step against the phase and the outstanding pairs, and say where
    /// it leaves the run. Nothing is mutated on the refusal paths.
    fn apply(&mut self, step: &FakeStep) -> Result<FakePhase, FakeError> {
        if self.phase.is_terminal() {
            return Err(FakeError::AlreadyEnded { phase: self.phase });
        }
        if !self.phase.accepts(step) {
            return Err(FakeError::OutOfOrder {
                step: step.kind(),
                phase: self.phase,
            });
        }
        match step {
            FakeStep::ProposeWrite { path } => self.pending_write = Some(path.clone()),
            FakeStep::Write { path } => {
                if self.pending_write.as_deref() != Some(path.as_str()) {
                    return Err(FakeError::Unpaired { step: step.kind() });
                }
                self.pending_write = None;
            }
            FakeStep::Command { command, .. } | FakeStep::Test { command } => {
                if self.pending_command.is_some() {
                    return Err(FakeError::Unpaired { step: step.kind() });
                }
                self.pending_command = Some(command.clone());
            }
            FakeStep::CommandDone { .. } => {
                let _finished = self
                    .pending_command
                    .take()
                    .ok_or(FakeError::Unpaired { step: step.kind() })?;
            }
            _ => {}
        }
        Ok(match step {
            FakeStep::Prompt { .. } => FakePhase::Working,
            FakeStep::AskPermission { .. } => FakePhase::AwaitingPermission,
            FakeStep::AskDecision { .. } => FakePhase::AwaitingDecision,
            FakeStep::Idle => FakePhase::Idle,
            FakeStep::Present => FakePhase::Presenting,
            FakeStep::Finish { .. } => FakePhase::Done,
            FakeStep::Fail { .. } => FakePhase::Broke,
            _ => self.phase,
        })
    }

    fn advance(&mut self) -> FakeAdvance {
        if self.phase.is_terminal() {
            return FakeAdvance::Finished(self.phase);
        }
        if self.phase.is_awaiting_answer() {
            return FakeAdvance::Blocked(self.phase);
        }
        let next = self.script.steps().get(self.cursor).cloned();
        let Some(step) = next else {
            // §11, half one: a script that never says how it ends does not get
            // to leave the run in `RUNNING`.
            tracing::warn!(
                phase = %self.phase,
                steps = self.script.len(),
                "fake agent script ended with no terminal step"
            );
            return FakeAdvance::Moved(self.record(
                FakeAction::Orphaned {
                    reason: ORPHANED_UNTERMINATED,
                },
                FakePhase::Broke,
            ));
        };
        self.cursor += 1;
        match self.apply(&step) {
            Ok(phase) => FakeAdvance::Moved(self.record(FakeAction::Step(step), phase)),
            Err(reason) => {
                self.rejected += 1;
                // Kind and phase only. A payload is untrusted adapter text and
                // does not go in a log line (rule 13).
                tracing::warn!(
                    step = step.kind(),
                    phase = %self.phase,
                    "fake agent step rejected"
                );
                FakeAdvance::Rejected { step, reason }
            }
        }
    }

    fn answer(
        &mut self,
        expected: FakePhase,
        action: FakeAction,
    ) -> Result<FakeObservation, FakeError> {
        if self.phase.is_terminal() {
            return Err(FakeError::AlreadyEnded { phase: self.phase });
        }
        if self.phase != expected {
            return Err(FakeError::NothingToAnswer { phase: self.phase });
        }
        Ok(self.record(action, FakePhase::Working))
    }

    fn cancel(&mut self) -> Option<FakeObservation> {
        if self.phase.is_terminal() {
            return None;
        }
        Some(self.record(FakeAction::Cancelled, FakePhase::Stopped))
    }

    /// §11, half two. Called from `Drop`, where there is nobody to return a
    /// value to — which is why the record lives behind the shared handle.
    fn resolve_orphan(&mut self) -> bool {
        if self.phase.is_terminal() {
            return false;
        }
        tracing::warn!(
            phase = %self.phase,
            "fake agent driver dropped mid-run; resolving the run rather than orphaning it"
        );
        self.record(
            FakeAction::Orphaned {
                reason: ORPHANED_DROPPED,
            },
            FakePhase::Broke,
        );
        true
    }
}

/// A poisoned lock is the case this module exists to survive: the driving task
/// panicked, which is precisely when the orphan guarantee has to hold.
fn lock(inner: &Arc<Mutex<Inner>>) -> MutexGuard<'_, Inner> {
    inner.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The driver. Advancing it is the only thing that makes a scripted run move.
///
/// Dropping it before the run reached a terminal phase resolves the run to
/// [`FakePhase::Broke`] — see [`ORPHANED_DROPPED`]. That is the whole of §11's
/// second half, and it is why a supervisor should take a [`FakeAgentHandle`]
/// before handing the driver to whatever will advance it.
#[derive(Debug)]
pub struct FakeAgent {
    inner: Arc<Mutex<Inner>>,
}

impl FakeAgent {
    /// A run of `script`, on the default logical clock.
    pub fn new(script: FakeScript) -> Self {
        Self::with_clock(script, FakeClock::default())
    }

    /// A run of `script`, on a clock the caller chose. Two agents built with
    /// the same script and the same clock produce identical observations.
    pub fn with_clock(script: FakeScript, clock: FakeClock) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::new(script, clock))),
        }
    }

    /// A view a supervisor keeps. It outlives this driver on purpose.
    pub fn handle(&self) -> FakeAgentHandle {
        FakeAgentHandle {
            inner: Arc::clone(&self.inner),
        }
    }

    pub fn phase(&self) -> FakePhase {
        lock(&self.inner).phase
    }

    pub fn is_finished(&self) -> bool {
        lock(&self.inner).phase.is_terminal()
    }

    pub fn script_name(&self) -> String {
        lock(&self.inner).script.name().to_owned()
    }

    /// Steps not yet reached. Zero does not mean the run ended: the next
    /// advance is what resolves an unterminated script.
    pub fn remaining(&self) -> usize {
        let inner = lock(&self.inner);
        inner.script.len().saturating_sub(inner.cursor)
    }

    /// How many steps were refused as duplicated or out of order. Reported,
    /// never hidden.
    pub fn rejected(&self) -> u32 {
        lock(&self.inner).rejected
    }

    /// Everything recorded so far, oldest first.
    pub fn observations(&self) -> Vec<FakeObservation> {
        lock(&self.inner).observations.clone()
    }

    /// Run one step.
    pub fn advance(&mut self) -> FakeAdvance {
        lock(&self.inner).advance()
    }

    /// Run steps until the run stops on an answer or ends.
    ///
    /// Refusals are counted and skipped rather than returned, so this always
    /// terminates: every call consumes a step or ends the run.
    pub fn advance_all(&mut self) -> Vec<FakeObservation> {
        let mut moved = Vec::new();
        loop {
            match self.advance() {
                FakeAdvance::Moved(observation) => moved.push(observation),
                FakeAdvance::Rejected { .. } => {}
                FakeAdvance::Blocked(_) | FakeAdvance::Finished(_) => return moved,
            }
        }
    }

    /// Answer a pending permission ask and resume (§10).
    ///
    /// Out of band from the wire protocol on purpose: this is not a `runs.*`
    /// command and must not become one here. Turning a `PERMISSION_REQUESTED`
    /// into something a user answers is T335's.
    pub fn answer_permission(&self, granted: bool) -> Result<FakeObservation, FakeError> {
        lock(&self.inner).answer(
            FakePhase::AwaitingPermission,
            FakeAction::PermissionAnswered { granted },
        )
    }

    /// Answer a pending question and resume (§10). The answer itself is not
    /// carried — see [`FakeAction::DecisionAnswered`].
    pub fn answer_decision(&self) -> Result<FakeObservation, FakeError> {
        lock(&self.inner).answer(FakePhase::AwaitingDecision, FakeAction::DecisionAnswered)
    }

    /// Stop the run. Idempotent: a run that already ended returns `None` and
    /// keeps the ending it had.
    pub fn cancel(&self) -> Option<FakeObservation> {
        lock(&self.inner).cancel()
    }

    /// Poison this agent's lock from inside a panic, to prove the orphan
    /// guarantee survives one.
    #[cfg(test)]
    fn poison(&self) {
        let _guard = self.inner.lock().expect("not poisoned yet");
        panic!("poisoning the fake agent's lock on purpose");
    }
}

impl Drop for FakeAgent {
    fn drop(&mut self) {
        lock(&self.inner).resolve_orphan();
    }
}

/// What a supervisor holds: it can read the record, answer out of band, and
/// cancel, but it cannot advance the script.
///
/// It survives the driver, which is what makes the drop case observable.
#[derive(Debug, Clone)]
pub struct FakeAgentHandle {
    inner: Arc<Mutex<Inner>>,
}

impl FakeAgentHandle {
    pub fn phase(&self) -> FakePhase {
        lock(&self.inner).phase
    }

    pub fn is_finished(&self) -> bool {
        lock(&self.inner).phase.is_terminal()
    }

    pub fn rejected(&self) -> u32 {
        lock(&self.inner).rejected
    }

    /// Everything recorded so far, oldest first.
    ///
    /// The record is bounded by the script: at most one observation per step,
    /// plus one per answer and at most one ending. A script is finite, so this
    /// is too — there is no ring here, and the bounded ring the protocol
    /// promises is the core's (`MAX_RUN_EVENTS`).
    pub fn observations(&self) -> Vec<FakeObservation> {
        lock(&self.inner).observations.clone()
    }

    /// Everything recorded at or after `seq` — what a supervisor batches.
    pub fn observations_since(&self, seq: u64) -> Vec<FakeObservation> {
        lock(&self.inner)
            .observations
            .iter()
            .filter(|observation| observation.seq >= seq)
            .cloned()
            .collect()
    }

    /// See [`FakeAgent::answer_permission`].
    pub fn answer_permission(&self, granted: bool) -> Result<FakeObservation, FakeError> {
        lock(&self.inner).answer(
            FakePhase::AwaitingPermission,
            FakeAction::PermissionAnswered { granted },
        )
    }

    /// See [`FakeAgent::answer_decision`].
    pub fn answer_decision(&self) -> Result<FakeObservation, FakeError> {
        lock(&self.inner).answer(FakePhase::AwaitingDecision, FakeAction::DecisionAnswered)
    }

    /// See [`FakeAgent::cancel`].
    pub fn cancel(&self) -> Option<FakeObservation> {
        lock(&self.inner).cancel()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::FakeScriptPreset;

    fn agent(preset: FakeScriptPreset) -> FakeAgent {
        FakeAgent::new(FakeScript::preset(preset))
    }

    /// A supervisor drives the fake from one task and answers it from another,
    /// so both halves have to cross a thread boundary.
    #[test]
    fn the_driver_and_its_handle_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<FakeAgent>();
        assert_send_sync::<FakeAgentHandle>();
    }

    #[test]
    fn the_clock_is_a_counter_and_never_a_clock() {
        let mut clock = FakeClock::new(10, 5);
        assert_eq!(clock.tick(), 10);
        assert_eq!(clock.tick(), 15);
        assert_eq!(clock.tick(), 20);
        let mut ceiling = FakeClock::new(u64::MAX, 5);
        assert_eq!(ceiling.tick(), u64::MAX);
        assert_eq!(ceiling.tick(), u64::MAX, "saturates rather than wrapping");
    }

    #[test]
    fn a_fresh_run_is_preparing_with_its_whole_script_ahead_of_it() {
        let agent = agent(FakeScriptPreset::Quick);
        assert_eq!(agent.phase(), FakePhase::Preparing);
        assert_eq!(agent.script_name(), "quick");
        assert_eq!(agent.remaining(), 3);
        assert_eq!(agent.rejected(), 0);
        assert!(agent.observations().is_empty());
        assert!(!agent.is_finished());
    }

    #[test]
    fn a_blocked_run_consumes_nothing_until_it_is_answered() {
        let mut agent = agent(FakeScriptPreset::FullLifecycle);
        agent.advance_all();
        assert_eq!(agent.phase(), FakePhase::AwaitingPermission);
        let before = agent.remaining();
        assert!(agent.advance().is_blocked());
        assert!(agent.advance().is_blocked());
        assert_eq!(agent.remaining(), before, "the script did not move");
        agent.answer_permission(true).unwrap();
        assert_eq!(agent.phase(), FakePhase::Working);
    }

    #[test]
    fn answering_when_nothing_was_asked_is_refused() {
        let mut agent = agent(FakeScriptPreset::Quick);
        assert_eq!(
            agent.answer_permission(true).unwrap_err(),
            FakeError::NothingToAnswer {
                phase: FakePhase::Preparing
            }
        );
        agent.advance_all();
        assert_eq!(agent.phase(), FakePhase::Done);
        assert_eq!(
            agent.answer_decision().unwrap_err(),
            FakeError::AlreadyEnded {
                phase: FakePhase::Done
            }
        );
    }

    /// Answering the *other* kind of stop is refused too: one stop at a time.
    #[test]
    fn the_wrong_answer_does_not_clear_a_stop() {
        let mut agent = agent(FakeScriptPreset::FullLifecycle);
        agent.advance_all();
        assert_eq!(agent.phase(), FakePhase::AwaitingPermission);
        assert_eq!(
            agent.answer_decision().unwrap_err(),
            FakeError::NothingToAnswer {
                phase: FakePhase::AwaitingPermission
            }
        );
        assert_eq!(agent.phase(), FakePhase::AwaitingPermission);
    }

    #[test]
    fn an_out_of_order_step_is_rejected_counted_and_changes_nothing() {
        let script = FakeScript::new(
            "out-of-order",
            vec![
                FakeStep::prompt("go"),
                // A write nobody proposed.
                FakeStep::write("src/lib.rs"),
                FakeStep::finish("done"),
            ],
        );
        let mut agent = FakeAgent::new(script);
        assert!(matches!(agent.advance(), FakeAdvance::Moved(_)));
        let refusal = agent.advance();
        let FakeAdvance::Rejected { step, reason } = refusal else {
            panic!("an unpaired write must be refused, got {refusal:?}");
        };
        assert_eq!(step.kind(), "write");
        assert_eq!(reason, FakeError::Unpaired { step: "write" });
        assert_eq!(agent.phase(), FakePhase::Working, "nothing changed");
        assert_eq!(agent.rejected(), 1);
        assert_eq!(agent.observations().len(), 1, "no observation recorded");
        agent.advance_all();
        assert_eq!(agent.phase(), FakePhase::Done, "the run carried on");
    }

    #[test]
    fn a_duplicated_step_is_rejected_by_the_phase_rules() {
        let script = FakeScript::new(
            "duplicate-finish",
            vec![
                FakeStep::prompt("go"),
                FakeStep::finish("first"),
                FakeStep::finish("second"),
            ],
        );
        let mut agent = FakeAgent::new(script);
        agent.advance_all();
        assert_eq!(agent.phase(), FakePhase::Done);
        // A terminal run refuses everything, and says so the same way twice.
        assert_eq!(
            agent.advance(),
            FakeAdvance::Finished(FakePhase::Done),
            "a finished run does not consume the leftover step"
        );
        assert_eq!(agent.advance(), FakeAdvance::Finished(FakePhase::Done));
        assert_eq!(agent.rejected(), 0);
    }

    #[test]
    fn a_command_that_never_started_cannot_finish() {
        let script = FakeScript::new(
            "unpaired-command",
            vec![
                FakeStep::prompt("go"),
                FakeStep::CommandDone {
                    ok: true,
                    exit_code: Some(0),
                },
                FakeStep::command("ls"),
                FakeStep::command("ls"),
                FakeStep::finish("done"),
            ],
        );
        let mut agent = FakeAgent::new(script);
        agent.advance_all();
        assert_eq!(
            agent.rejected(),
            2,
            "the orphan result and the second start"
        );
        assert_eq!(agent.phase(), FakePhase::Done);
    }

    #[test]
    fn cancelling_is_idempotent_and_keeps_the_first_ending() {
        let mut agent = agent(FakeScriptPreset::Quick);
        agent.advance();
        let cancelled = agent.cancel().expect("a running run cancels");
        assert_eq!(cancelled.phase, FakePhase::Stopped);
        assert_eq!(cancelled.action, FakeAction::Cancelled);
        assert!(agent.cancel().is_none(), "a second cancel is a no-op");
        assert_eq!(agent.phase(), FakePhase::Stopped);
        assert_eq!(agent.observations().len(), 2);
    }

    /// A cancel has to land wherever the run happens to be — the same property
    /// the core's transition table asserts, exercised through the driver.
    #[test]
    fn a_run_can_be_cancelled_from_every_non_terminal_phase() {
        for &phase in FakePhase::ALL {
            if phase.is_terminal() {
                continue;
            }
            let mut agent = FakeAgent::new(script_reaching(phase));
            drive_to(&mut agent, phase);
            assert_eq!(agent.phase(), phase, "failed to reach {phase}");
            let cancelled = agent.cancel().unwrap_or_else(|| panic!("{phase}"));
            assert_eq!(cancelled.phase, FakePhase::Stopped);
            assert_eq!(agent.phase(), FakePhase::Stopped);
        }
    }

    /// A terminal run is unaffected by a cancel, whichever ending it had.
    #[test]
    fn a_terminal_run_is_unaffected_by_a_cancel() {
        for &preset in &[FakeScriptPreset::Quick, FakeScriptPreset::Failing] {
            let mut agent = agent(preset);
            agent.advance_all();
            let ending = agent.phase();
            assert!(ending.is_terminal(), "{preset}");
            let before = agent.observations();
            assert!(agent.cancel().is_none());
            assert_eq!(agent.phase(), ending);
            assert_eq!(agent.observations(), before);
        }
    }

    /// §11, half one: a script with nothing terminal in it resolves anyway.
    #[test]
    fn a_script_with_no_terminal_step_resolves_to_broke() {
        let mut agent = agent(FakeScriptPreset::Unterminated);
        let moved = agent.advance_all();
        assert_eq!(agent.phase(), FakePhase::Broke);
        let last = moved.last().expect("something happened");
        assert_eq!(
            last.action,
            FakeAction::Orphaned {
                reason: ORPHANED_UNTERMINATED
            }
        );
        assert_eq!(last.action.run_events(), &["RUN_FAILED"]);
        assert_eq!(agent.advance(), FakeAdvance::Finished(FakePhase::Broke));
    }

    /// §11, half two: the driver goes away and the run does not stay running.
    #[test]
    fn dropping_the_driver_resolves_the_run_through_the_handle() {
        let mut agent = agent(FakeScriptPreset::FullLifecycle);
        let handle = agent.handle();
        agent.advance();
        assert_eq!(handle.phase(), FakePhase::Working);
        drop(agent);
        assert_eq!(handle.phase(), FakePhase::Broke);
        let last = handle.observations().pop().expect("an ending was recorded");
        assert_eq!(
            last.action,
            FakeAction::Orphaned {
                reason: ORPHANED_DROPPED
            }
        );
    }

    #[test]
    fn dropping_a_finished_driver_records_nothing_further() {
        let mut agent = agent(FakeScriptPreset::Quick);
        let handle = agent.handle();
        agent.advance_all();
        let before = handle.observations();
        drop(agent);
        assert_eq!(handle.phase(), FakePhase::Done);
        assert_eq!(handle.observations(), before);
    }

    /// §11, half two again, this time through a real panic: the driving task
    /// falls over, the driver unwinds, and the run still ends.
    #[test]
    fn a_panicking_driver_still_resolves_the_run() {
        let agent = agent(FakeScriptPreset::FullLifecycle);
        let handle = agent.handle();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let mut agent = agent;
            agent.advance();
            panic!("the driving task fell over");
        }));
        std::panic::set_hook(previous);
        assert!(result.is_err(), "the panic must have happened");
        assert_eq!(handle.phase(), FakePhase::Broke);
        assert_eq!(
            handle.observations().pop().map(|o| o.action),
            Some(FakeAction::Orphaned {
                reason: ORPHANED_DROPPED
            })
        );
    }

    /// The nastier version: the panic happens while the lock is held, so the
    /// lock is poisoned and `Drop` still has to work.
    #[test]
    fn a_poisoned_lock_does_not_stop_the_run_resolving() {
        let agent = agent(FakeScriptPreset::FullLifecycle);
        let handle = agent.handle();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let mut agent = agent;
            agent.advance();
            agent.poison();
        }));
        std::panic::set_hook(previous);
        assert!(result.is_err());
        assert_eq!(handle.phase(), FakePhase::Broke);
        assert_eq!(handle.rejected(), 0);
    }

    #[test]
    fn observations_since_returns_the_tail_a_supervisor_batches() {
        let mut agent = agent(FakeScriptPreset::Quick);
        let handle = agent.handle();
        agent.advance_all();
        let all = handle.observations();
        assert_eq!(handle.observations_since(0), all);
        assert_eq!(handle.observations_since(1), all[1..].to_vec());
        assert!(handle.observations_since(99).is_empty());
        for (index, observation) in all.iter().enumerate() {
            assert_eq!(observation.seq, index as u64);
        }
    }

    /// A script that reaches `phase` and stops there.
    fn script_reaching(phase: FakePhase) -> FakeScript {
        let mut steps = vec![FakeStep::prompt("go")];
        match phase {
            FakePhase::Preparing => steps.clear(),
            FakePhase::Working => {}
            FakePhase::Idle => steps.push(FakeStep::Idle),
            FakePhase::AwaitingPermission => {
                steps.push(FakeStep::ask_permission("Edit", "why"));
            }
            FakePhase::AwaitingDecision => steps.push(FakeStep::ask_decision("which?")),
            FakePhase::Presenting => steps.push(FakeStep::Present),
            FakePhase::Done | FakePhase::Broke | FakePhase::Stopped => {
                unreachable!("terminal phases are not driven to")
            }
        }
        FakeScript::new(format!("reach-{phase}"), steps)
    }

    /// Advance exactly as far as the script for `phase` goes — never past the
    /// end, so the orphan resolution does not fire.
    fn drive_to(agent: &mut FakeAgent, phase: FakePhase) {
        if phase == FakePhase::Preparing {
            return;
        }
        while agent.remaining() > 0 {
            agent.advance();
        }
    }
}
