//! The phase a scripted run is in.
//!
//! This is the fake's **own** vocabulary, not `gritty_core::AgentRunState`,
//! and it cannot be that type: this crate has no `gritty-core` dependency, for
//! the cycle reason spelled out in [`crate`]. The difference is not cosmetic —
//! there are nine phases and ten states, because
//! `AgentRunState::Interrupted` is something only a supervisor can decide (the
//! workspace closed, the app is shutting down) and a script can never reach it
//! on its own.
//!
//! Every phase names the state it maps to, as a string, via
//! [`FakePhase::run_state`]. That is deliberately a `&'static str` and not an
//! imported enum: it lets the two lanes be diffed against each other, and it
//! keeps `AgentRunState::as_str` the authority rather than a second copy of the
//! transition table. **The core's table is the one that decides.** The
//! legality rules here are the fake's own, narrower guard so that a malformed
//! script is refused where it is written instead of one layer later.

use crate::script::FakeStep;

/// Where a scripted run has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FakePhase {
    /// Loaded, nothing run yet. Maps to `STARTING`.
    ///
    /// The fake needs no shell, so it leaves this the moment the first step is
    /// advanced — which is exactly the window a terminal-backed adapter (T310)
    /// will sit in instead.
    Preparing,
    /// Working through the script. Maps to `RUNNING`.
    Working,
    /// The turn is over and nothing is outstanding. Maps to `WAITING`.
    Idle,
    /// Stopped on a permission ask, until someone answers it out of band.
    /// Maps to `BLOCKED`.
    AwaitingPermission,
    /// Stopped on a question, until someone answers it out of band. Maps to
    /// `NEEDS_DECISION`.
    AwaitingDecision,
    /// Done working, waiting to be looked at. Maps to `REVIEW`.
    Presenting,
    /// Ended, having run its script to a `Finish`. Maps to `FINISHED`.
    Done,
    /// Ended badly: a scripted `Fail`, a script that ran out with no terminal
    /// step, or a driver that was dropped mid-run. Maps to `FAILED`.
    Broke,
    /// Ended because someone cancelled it. Maps to `CANCELLED`.
    Stopped,
}

impl FakePhase {
    /// Every phase, for exhaustive tests.
    pub const ALL: &'static [Self] = &[
        Self::Preparing,
        Self::Working,
        Self::Idle,
        Self::AwaitingPermission,
        Self::AwaitingDecision,
        Self::Presenting,
        Self::Done,
        Self::Broke,
        Self::Stopped,
    ];

    /// The three ways a scripted run ends. A fourth ending — `INTERRUPTED` —
    /// exists in the core and is not reachable from here on purpose: nothing a
    /// script does means "we stopped being able to watch it".
    pub const TERMINAL: &'static [Self] = &[Self::Done, Self::Broke, Self::Stopped];

    /// The fake's name for this phase, for tracing and for tests.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Working => "working",
            Self::Idle => "idle",
            Self::AwaitingPermission => "awaitingPermission",
            Self::AwaitingDecision => "awaitingDecision",
            Self::Presenting => "presenting",
            Self::Done => "done",
            Self::Broke => "broke",
            Self::Stopped => "stopped",
        }
    }

    /// The `AgentRunState` this phase means, spelled as
    /// `gritty_core::AgentRunState::as_str` spells it.
    ///
    /// The translation in `agent_run` (T300-CORE) is what actually performs the
    /// mapping; this exists so the two lanes can be checked against each other
    /// without either importing the other, and so a reviewer can see the whole
    /// correspondence in one place.
    pub const fn run_state(self) -> &'static str {
        match self {
            Self::Preparing => "STARTING",
            Self::Working => "RUNNING",
            Self::Idle => "WAITING",
            Self::AwaitingPermission => "BLOCKED",
            Self::AwaitingDecision => "NEEDS_DECISION",
            Self::Presenting => "REVIEW",
            Self::Done => "FINISHED",
            Self::Broke => "FAILED",
            Self::Stopped => "CANCELLED",
        }
    }

    /// Nothing further happens on its own, and nothing can move it again.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Broke | Self::Stopped)
    }

    /// Stopped on something only an out-of-band answer can clear. The script
    /// does not advance while this is true — which is the whole of §10.
    pub const fn is_awaiting_answer(self) -> bool {
        matches!(self, Self::AwaitingPermission | Self::AwaitingDecision)
    }

    /// Whether this phase will accept that step.
    ///
    /// The rules mirror `AgentRunState::allowed_transitions` and are narrower
    /// nowhere it matters:
    ///
    /// - Work steps happen only while [`Self::Working`], because that is the
    ///   only state in which an activity is expected.
    /// - A stopped run resumes before it does anything else, so a script step
    ///   that is not `Fail` is refused while awaiting an answer — the core's
    ///   "one stop at a time" rule, enforced a layer earlier.
    /// - `Fail` is legal from every non-terminal phase, because a run has to be
    ///   able to end badly from wherever it happens to be.
    /// - Nothing is legal once terminal.
    pub fn accepts(self, step: &FakeStep) -> bool {
        if self.is_terminal() {
            return false;
        }
        match step {
            FakeStep::Fail { .. } => true,
            FakeStep::Prompt { .. } => matches!(
                self,
                Self::Preparing | Self::Working | Self::Idle | Self::Presenting
            ),
            FakeStep::Think
            | FakeStep::Read { .. }
            | FakeStep::Search { .. }
            | FakeStep::ProposeWrite { .. }
            | FakeStep::Write { .. }
            | FakeStep::Command { .. }
            | FakeStep::Test { .. }
            | FakeStep::CommandDone { .. }
            | FakeStep::Reply { .. }
            | FakeStep::AskPermission { .. }
            | FakeStep::AskDecision { .. }
            | FakeStep::Idle => self == Self::Working,
            FakeStep::Present => matches!(self, Self::Working | Self::Idle),
            FakeStep::Finish { .. } => {
                matches!(self, Self::Working | Self::Idle | Self::Presenting)
            }
        }
    }
}

impl std::fmt::Display for FakePhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_phases_accept_nothing() {
        for &phase in FakePhase::TERMINAL {
            assert!(phase.is_terminal(), "{phase}");
            for step in crate::script::every_step_shape() {
                assert!(!phase.accepts(&step), "{phase} must refuse {}", step.kind());
            }
        }
        for &phase in FakePhase::ALL {
            assert_eq!(
                phase.is_terminal(),
                FakePhase::TERMINAL.contains(&phase),
                "{phase}"
            );
        }
    }

    /// A run has to be able to end badly from wherever it is — the property
    /// that makes the orphan guarantee unconditional rather than
    /// true-for-most-phases.
    #[test]
    fn every_non_terminal_phase_accepts_a_failure() {
        for &phase in FakePhase::ALL {
            if phase.is_terminal() {
                continue;
            }
            assert!(
                phase.accepts(&FakeStep::fail("stop")),
                "{phase} must be able to fail"
            );
        }
    }

    /// The core's "one stop at a time" rule, enforced a layer earlier: while a
    /// run is waiting to be answered, the script does not get to do anything
    /// except fail.
    #[test]
    fn a_stopped_phase_refuses_every_step_but_a_failure() {
        for &phase in &[FakePhase::AwaitingPermission, FakePhase::AwaitingDecision] {
            assert!(phase.is_awaiting_answer(), "{phase}");
            for step in crate::script::every_step_shape() {
                let expected = matches!(step, FakeStep::Fail { .. });
                assert_eq!(phase.accepts(&step), expected, "{phase} / {}", step.kind());
            }
        }
    }

    /// A run that has not begun has nothing to review and nothing to finish —
    /// `AgentRunState::Starting` reaches only `Running` and the endings.
    #[test]
    fn preparing_only_accepts_a_prompt_or_a_failure() {
        for step in crate::script::every_step_shape() {
            let expected = matches!(step, FakeStep::Prompt { .. } | FakeStep::Fail { .. });
            assert_eq!(
                FakePhase::Preparing.accepts(&step),
                expected,
                "{}",
                step.kind()
            );
        }
    }

    /// Each phase names exactly one core state, and no two share one. If this
    /// ever fails, the two lanes have drifted and the translation in
    /// `agent_run` cannot be total.
    #[test]
    fn phases_map_onto_distinct_core_states() {
        let mut seen = Vec::new();
        for &phase in FakePhase::ALL {
            let state = phase.run_state();
            assert!(!seen.contains(&state), "{state} claimed twice");
            seen.push(state);
        }
        assert_eq!(seen.len(), 9);
        // The tenth state exists and is deliberately unreachable from here.
        assert!(!seen.contains(&"INTERRUPTED"));
    }

    #[test]
    fn phase_names_round_trip_through_their_display_form() {
        for &phase in FakePhase::ALL {
            assert_eq!(phase.to_string(), phase.as_str());
        }
    }
}
