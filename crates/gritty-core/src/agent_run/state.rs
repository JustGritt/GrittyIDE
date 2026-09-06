//! The agent-run state machine (Gate D, coordination plan §6).
//!
//! Ten states and an explicit transition table, read the way
//! [`crate::workstream::state::WorkstreamState`] is read: an adapter, the
//! supervisor (T300-CORE) or a UI consults this, none of them keeps its own
//! copy. Everything not in [`AgentRunState::allowed_transitions`] is a
//! `Conflict`, and a rejected transition changes nothing — which is how a
//! duplicated or out-of-order adapter step is refused rather than absorbed.
//!
//! This is **not** [`crate::agent_session`]'s `AgentActivity`. That one
//! describes a session Gritty observed and did not start, and is derived from
//! evidence; this one describes a run Gritty owns and drives, and is only ever
//! moved by a transition the core accepted.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::protocol::error::CoreError;

/// Where a run Gritty owns has got to.
///
/// The vocabulary is the coordination plan's, unchanged. `state` answers *what
/// is happening to the run*; [`super::wire::AgentRunActivity`] answers *what it
/// is doing right now*, and the two stay separate (plan §6) — `RUNNING` holds
/// still while the activity cycles through `EDITING`, `EXECUTING` and the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentRunState {
    /// Accepted, not yet working. The window in which an adapter gets itself
    /// ready and nothing of the user's work has begun.
    ///
    /// Deliberately distinct from [`Self::Running`] even though the fake
    /// leaves it immediately: phase 0 found that a terminal-backed agent is not
    /// usable the moment its process exists (`docs/plans/phase-0-spike.md`
    /// §D), so a real provider (T310) waits that window out *here* rather than
    /// claiming to be running. The state exists now so T310 does not invent it
    /// later and change this enum's shape after the gate froze it.
    Starting,
    /// Working. The only state in which an activity is expected.
    Running,
    /// Idle with its turn over, waiting for the user's next input. Nothing is
    /// wrong and nothing is outstanding; the run simply has nothing to do until
    /// someone says something.
    Waiting,
    /// Stopped on a question that has to be answered before it can go on.
    ///
    /// Raising the matching workstream `Decision` — the thing the user actually
    /// answers — is T335's, not this task's: a run may *reach* this state, and
    /// nothing here turns it into an answerable object.
    NeedsDecision,
    /// Stopped on something it cannot pass on its own: a permission prompt, or
    /// ownership of a path another workstream holds (plan §6's own example).
    Blocked,
    /// Finished working and waiting to be looked at. The work exists; whether
    /// it is accepted is the user's call.
    Review,
    /// Ended, having done what it was asked.
    Finished,
    /// Ended badly. [`super::wire::AgentRun::error`] says how.
    Failed,
    /// Ended because someone asked it to stop.
    Cancelled,
    /// Ended because what was driving it went away — the process died, the app
    /// is shutting down, the workspace closed. Separate from [`Self::Failed`]
    /// because "the agent broke" and "we stopped being able to watch it" are
    /// different answers, and from [`Self::Cancelled`] because nobody asked.
    Interrupted,
}

impl AgentRunState {
    /// Every state, for exhaustive tests and for a store's round trip.
    pub const ALL: &'static [Self] = &[
        Self::Starting,
        Self::Running,
        Self::Waiting,
        Self::NeedsDecision,
        Self::Blocked,
        Self::Review,
        Self::Finished,
        Self::Failed,
        Self::Cancelled,
        Self::Interrupted,
    ];

    /// The four ways a run ends.
    pub const TERMINAL: &'static [Self] = &[
        Self::Finished,
        Self::Failed,
        Self::Cancelled,
        Self::Interrupted,
    ];

    /// Where this state may go next.
    ///
    /// Two rules generate the whole table, and both are load-bearing:
    ///
    /// - **Every non-terminal state may reach [`Self::Failed`],
    ///   [`Self::Cancelled`] and [`Self::Interrupted`].** Cancellation has to
    ///   work from wherever the run happens to be (ADR-005), and a supervisor
    ///   that loses the thing it was driving has to be able to say so from any
    ///   state — otherwise an orphan sits in `RUNNING` forever, which is the
    ///   failure this state machine exists to prevent.
    /// - **Progress is otherwise narrow.** A stopped run ([`Self::Blocked`],
    ///   [`Self::NeedsDecision`]) resumes to [`Self::Running`] and nowhere
    ///   else: one stop at a time, so changing what a run is stopped on means
    ///   resuming first. `STARTING` reaches only `RUNNING`, because nothing
    ///   else can have happened yet.
    ///
    /// Unlike a workstream, a run's terminal states are genuinely terminal:
    /// there is no way back, because the thing that was executing is gone. A
    /// second attempt is a second run.
    pub const fn allowed_transitions(self) -> &'static [Self] {
        match self {
            Self::Starting => &[
                Self::Running,
                Self::Failed,
                Self::Cancelled,
                Self::Interrupted,
            ],
            Self::Running => &[
                Self::Waiting,
                Self::NeedsDecision,
                Self::Blocked,
                Self::Review,
                Self::Finished,
                Self::Failed,
                Self::Cancelled,
                Self::Interrupted,
            ],
            Self::Waiting => &[
                Self::Running,
                Self::Review,
                Self::Finished,
                Self::Failed,
                Self::Cancelled,
                Self::Interrupted,
            ],
            Self::NeedsDecision | Self::Blocked => &[
                Self::Running,
                Self::Failed,
                Self::Cancelled,
                Self::Interrupted,
            ],
            Self::Review => &[
                Self::Running,
                Self::Finished,
                Self::Failed,
                Self::Cancelled,
                Self::Interrupted,
            ],
            Self::Finished | Self::Failed | Self::Cancelled | Self::Interrupted => &[],
        }
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        self.allowed_transitions().contains(&next)
    }

    /// `Ok` when the move is legal.
    ///
    /// The error names both states because its caller — a supervisor reacting
    /// to an adapter step — has no other way to explain the refusal, and
    /// because a refused step is exactly what a duplicated or out-of-order
    /// adapter looks like from in here.
    pub fn check_transition(self, next: Self) -> Result<(), CoreError> {
        if self == next {
            return Err(CoreError::conflict(format!(
                "run is already {}",
                self.as_str()
            )));
        }
        if self.is_terminal() {
            return Err(CoreError::conflict(format!(
                "run already ended {}; it cannot become {}",
                self.as_str(),
                next.as_str()
            )));
        }
        if self.can_transition_to(next) {
            return Ok(());
        }
        Err(CoreError::conflict(format!(
            "a run cannot go from {} to {}",
            self.as_str(),
            next.as_str()
        )))
    }

    /// The run is over; nothing further happens on its own, and nothing can
    /// move it again.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Finished | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    /// The run is executing, so there is something for cancellation to reach
    /// and something for a supervisor to be holding.
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }

    /// The run is stopped and the user is the one holding it up — what T410
    /// will aggregate into the attention queue, and what the island counts.
    pub const fn needs_attention(self) -> bool {
        matches!(
            self,
            Self::Waiting | Self::NeedsDecision | Self::Blocked | Self::Review
        )
    }

    /// The stored and transmitted spelling, which is the serde one. Persistence
    /// is a later task (ADR-021), but a state that cannot name itself in one
    /// place is how a column and a wire value drift apart.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "STARTING",
            Self::Running => "RUNNING",
            Self::Waiting => "WAITING",
            Self::NeedsDecision => "NEEDS_DECISION",
            Self::Blocked => "BLOCKED",
            Self::Review => "REVIEW",
            Self::Finished => "FINISHED",
            Self::Failed => "FAILED",
            Self::Cancelled => "CANCELLED",
            Self::Interrupted => "INTERRUPTED",
        }
    }

    /// Parse a stored or transmitted state. A value Gritty cannot read is a bug
    /// or a hand-edited store, not something to guess at.
    pub fn parse(raw: &str) -> Result<Self, CoreError> {
        Self::ALL
            .iter()
            .copied()
            .find(|state| state.as_str() == raw)
            .ok_or_else(|| CoreError::internal(format!("unknown agent run state {raw:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::error::CoreErrorCode;

    /// The whole matrix, written out. Adding or removing a transition means
    /// editing this list deliberately — which is the point of a gate.
    const LEGAL: &[(AgentRunState, AgentRunState)] = &[
        (AgentRunState::Starting, AgentRunState::Running),
        (AgentRunState::Starting, AgentRunState::Failed),
        (AgentRunState::Starting, AgentRunState::Cancelled),
        (AgentRunState::Starting, AgentRunState::Interrupted),
        (AgentRunState::Running, AgentRunState::Waiting),
        (AgentRunState::Running, AgentRunState::NeedsDecision),
        (AgentRunState::Running, AgentRunState::Blocked),
        (AgentRunState::Running, AgentRunState::Review),
        (AgentRunState::Running, AgentRunState::Finished),
        (AgentRunState::Running, AgentRunState::Failed),
        (AgentRunState::Running, AgentRunState::Cancelled),
        (AgentRunState::Running, AgentRunState::Interrupted),
        (AgentRunState::Waiting, AgentRunState::Running),
        (AgentRunState::Waiting, AgentRunState::Review),
        (AgentRunState::Waiting, AgentRunState::Finished),
        (AgentRunState::Waiting, AgentRunState::Failed),
        (AgentRunState::Waiting, AgentRunState::Cancelled),
        (AgentRunState::Waiting, AgentRunState::Interrupted),
        (AgentRunState::NeedsDecision, AgentRunState::Running),
        (AgentRunState::NeedsDecision, AgentRunState::Failed),
        (AgentRunState::NeedsDecision, AgentRunState::Cancelled),
        (AgentRunState::NeedsDecision, AgentRunState::Interrupted),
        (AgentRunState::Blocked, AgentRunState::Running),
        (AgentRunState::Blocked, AgentRunState::Failed),
        (AgentRunState::Blocked, AgentRunState::Cancelled),
        (AgentRunState::Blocked, AgentRunState::Interrupted),
        (AgentRunState::Review, AgentRunState::Running),
        (AgentRunState::Review, AgentRunState::Finished),
        (AgentRunState::Review, AgentRunState::Failed),
        (AgentRunState::Review, AgentRunState::Cancelled),
        (AgentRunState::Review, AgentRunState::Interrupted),
    ];

    #[test]
    fn every_pair_is_legal_exactly_when_the_matrix_says_so() {
        for &from in AgentRunState::ALL {
            for &to in AgentRunState::ALL {
                let expected = LEGAL.contains(&(from, to));
                assert_eq!(
                    from.can_transition_to(to),
                    expected,
                    "{from:?} -> {to:?} should be {}",
                    if expected { "legal" } else { "illegal" }
                );
                assert_eq!(
                    from.check_transition(to).is_ok(),
                    expected,
                    "check_transition disagrees with can_transition_to for {from:?} -> {to:?}"
                );
            }
        }
    }

    /// The acceptance criterion, as a property rather than as one scripted run:
    /// a cancel has to land from wherever the run happens to be.
    #[test]
    fn every_non_terminal_state_can_be_cancelled_failed_and_interrupted() {
        for &state in AgentRunState::ALL {
            if state.is_terminal() {
                continue;
            }
            for &ending in &[
                AgentRunState::Cancelled,
                AgentRunState::Failed,
                AgentRunState::Interrupted,
            ] {
                assert!(
                    state.can_transition_to(ending),
                    "{state:?} must be able to end {ending:?}"
                );
            }
        }
    }

    #[test]
    fn terminal_states_are_dead_ends() {
        for &state in AgentRunState::TERMINAL {
            assert!(state.is_terminal());
            assert!(
                state.allowed_transitions().is_empty(),
                "{state:?} must not move again"
            );
            let error = state
                .check_transition(AgentRunState::Running)
                .expect_err("a finished run does not restart");
            assert_eq!(error.code, CoreErrorCode::Conflict);
            assert!(error.message.contains("already ended"), "{}", error.message);
        }
        for &state in AgentRunState::ALL {
            assert_eq!(
                state.is_terminal(),
                AgentRunState::TERMINAL.contains(&state),
                "{state:?}"
            );
        }
    }

    /// A duplicated adapter step is a transition to the state the run is
    /// already in, and it is refused rather than absorbed.
    #[test]
    fn a_state_cannot_transition_to_itself() {
        for &state in AgentRunState::ALL {
            let error = state
                .check_transition(state)
                .expect_err("self transition is not a change");
            assert_eq!(error.code, CoreErrorCode::Conflict);
            assert!(error.message.contains("already"), "{}", error.message);
        }
    }

    #[test]
    fn illegal_transitions_conflict_and_name_both_states() {
        let error = AgentRunState::Starting
            .check_transition(AgentRunState::Review)
            .expect_err("a run that has not begun has nothing to review");
        assert_eq!(error.code, CoreErrorCode::Conflict);
        assert!(error.message.contains("STARTING"), "{}", error.message);
        assert!(error.message.contains("REVIEW"), "{}", error.message);
    }

    /// The one-stop-at-a-time rule, stated as a test so that widening it is a
    /// deliberate edit.
    #[test]
    fn a_stopped_run_resumes_before_it_stops_on_something_else() {
        for &stopped in &[AgentRunState::Blocked, AgentRunState::NeedsDecision] {
            for &other in &[
                AgentRunState::Blocked,
                AgentRunState::NeedsDecision,
                AgentRunState::Waiting,
                AgentRunState::Review,
                AgentRunState::Finished,
            ] {
                assert!(
                    !stopped.can_transition_to(other),
                    "{stopped:?} must resume before reaching {other:?}"
                );
            }
            assert!(stopped.can_transition_to(AgentRunState::Running));
        }
    }

    #[test]
    fn active_and_attention_states_partition_the_non_terminal_ones() {
        for &state in AgentRunState::ALL {
            let counted = usize::from(state.is_active())
                + usize::from(state.needs_attention())
                + usize::from(state.is_terminal());
            assert_eq!(
                counted, 1,
                "{state:?} is in exactly one of the three groups"
            );
        }
        assert!(AgentRunState::Starting.is_active());
        assert!(AgentRunState::Blocked.needs_attention());
        assert!(!AgentRunState::Running.needs_attention());
    }

    #[test]
    fn states_round_trip_through_their_stored_form() {
        for &state in AgentRunState::ALL {
            assert_eq!(AgentRunState::parse(state.as_str()).unwrap(), state);
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::json!(state.as_str())
            );
        }
        let error = AgentRunState::parse("IN_PROGRESS").expect_err("unknown state");
        assert_eq!(error.code, CoreErrorCode::Internal);
    }
}
