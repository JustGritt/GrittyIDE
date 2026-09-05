//! The workstream state machine (Gate C, addendum §4).
//!
//! Small on purpose. Seven states and an explicit transition table: an agent
//! adapter, a UI or a later orchestration layer reads this, it never defines
//! its own. Everything not in [`WorkstreamState::allowed_transitions`] is a
//! `Conflict`, and a rejected transition changes nothing.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::protocol::error::CoreError;

/// Where a workstream is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkstreamState {
    /// Being described. Not ready for anyone — you or an agent — to start.
    Draft,
    /// Described well enough to start.
    Ready,
    /// Someone is working on it.
    InProgress,
    /// Waiting: on an open decision, or on something outside Gritty.
    Blocked,
    /// The work is finished and waiting for your review.
    InReview,
    /// Reviewed and accepted.
    Done,
    /// Dropped. Kept rather than deleted, so the history still explains itself.
    Abandoned,
}

impl WorkstreamState {
    /// Where this state may go next.
    ///
    /// The two terminal states each keep one way back — `DONE → IN_PROGRESS`
    /// (you found something in review after all) and `ABANDONED → DRAFT`
    /// (picking it up again) — so nothing forces the user to recreate a
    /// workstream and lose its decisions. Everything else is forward motion.
    pub const fn allowed_transitions(self) -> &'static [Self] {
        match self {
            Self::Draft => &[Self::Ready, Self::Abandoned],
            Self::Ready => &[Self::InProgress, Self::Draft, Self::Abandoned],
            Self::InProgress => &[Self::Blocked, Self::InReview, Self::Ready, Self::Abandoned],
            Self::Blocked => &[Self::InProgress, Self::Abandoned],
            Self::InReview => &[Self::InProgress, Self::Done, Self::Abandoned],
            Self::Done => &[Self::InProgress],
            Self::Abandoned => &[Self::Draft],
        }
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        self.allowed_transitions().contains(&next)
    }

    /// `Ok` when the move is legal. The error names both states because the
    /// caller (a command adapter) has no other way to explain the refusal.
    pub fn check_transition(self, next: Self) -> Result<(), CoreError> {
        if self == next {
            return Err(CoreError::conflict(format!(
                "workstream is already {}",
                self.as_str()
            )));
        }
        if self.can_transition_to(next) {
            return Ok(());
        }
        Err(CoreError::conflict(format!(
            "a workstream cannot go from {} to {}",
            self.as_str(),
            next.as_str()
        )))
    }

    /// Nothing further happens on its own from here.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Abandoned)
    }

    /// Whether the user is the one holding this up. What the island counts as
    /// "needs you" and what T410 will aggregate.
    pub const fn needs_attention(self) -> bool {
        matches!(self, Self::Blocked | Self::InReview)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
            Self::Ready => "READY",
            Self::InProgress => "IN_PROGRESS",
            Self::Blocked => "BLOCKED",
            Self::InReview => "IN_REVIEW",
            Self::Done => "DONE",
            Self::Abandoned => "ABANDONED",
        }
    }

    /// Every state, for exhaustive tests and for the store's round-trip.
    pub const ALL: &'static [Self] = &[
        Self::Draft,
        Self::Ready,
        Self::InProgress,
        Self::Blocked,
        Self::InReview,
        Self::Done,
        Self::Abandoned,
    ];

    /// Parse a stored state. A row Gritty cannot read is a bug or a hand-edited
    /// database, not something to guess at.
    pub fn parse(raw: &str) -> Result<Self, CoreError> {
        Self::ALL
            .iter()
            .copied()
            .find(|state| state.as_str() == raw)
            .ok_or_else(|| CoreError::internal(format!("unknown workstream state {raw:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::error::CoreErrorCode;

    /// The whole matrix, written out. If a transition is added or removed this
    /// test must be edited deliberately — which is the point of a gate.
    const LEGAL: &[(WorkstreamState, WorkstreamState)] = &[
        (WorkstreamState::Draft, WorkstreamState::Ready),
        (WorkstreamState::Draft, WorkstreamState::Abandoned),
        (WorkstreamState::Ready, WorkstreamState::InProgress),
        (WorkstreamState::Ready, WorkstreamState::Draft),
        (WorkstreamState::Ready, WorkstreamState::Abandoned),
        (WorkstreamState::InProgress, WorkstreamState::Blocked),
        (WorkstreamState::InProgress, WorkstreamState::InReview),
        (WorkstreamState::InProgress, WorkstreamState::Ready),
        (WorkstreamState::InProgress, WorkstreamState::Abandoned),
        (WorkstreamState::Blocked, WorkstreamState::InProgress),
        (WorkstreamState::Blocked, WorkstreamState::Abandoned),
        (WorkstreamState::InReview, WorkstreamState::InProgress),
        (WorkstreamState::InReview, WorkstreamState::Done),
        (WorkstreamState::InReview, WorkstreamState::Abandoned),
        (WorkstreamState::Done, WorkstreamState::InProgress),
        (WorkstreamState::Abandoned, WorkstreamState::Draft),
    ];

    #[test]
    fn every_pair_is_legal_exactly_when_the_matrix_says_so() {
        for &from in WorkstreamState::ALL {
            for &to in WorkstreamState::ALL {
                let expected = LEGAL.contains(&(from, to));
                assert_eq!(
                    from.can_transition_to(to),
                    expected,
                    "{from:?} -> {to:?} should be {}",
                    if expected { "legal" } else { "illegal" }
                );
            }
        }
    }

    #[test]
    fn illegal_transitions_conflict_and_name_both_states() {
        let error = WorkstreamState::Draft
            .check_transition(WorkstreamState::Done)
            .expect_err("draft cannot finish");
        assert_eq!(error.code, CoreErrorCode::Conflict);
        assert!(error.message.contains("DRAFT"), "{}", error.message);
        assert!(error.message.contains("DONE"), "{}", error.message);
    }

    #[test]
    fn a_state_cannot_transition_to_itself() {
        for &state in WorkstreamState::ALL {
            let error = state
                .check_transition(state)
                .expect_err("self transition is not a change");
            assert_eq!(error.code, CoreErrorCode::Conflict);
            assert!(error.message.contains("already"), "{}", error.message);
        }
    }

    #[test]
    fn no_state_is_a_dead_end() {
        for &state in WorkstreamState::ALL {
            assert!(
                !state.allowed_transitions().is_empty(),
                "{state:?} has no way out"
            );
        }
    }

    #[test]
    fn terminal_and_attention_states() {
        assert!(WorkstreamState::Done.is_terminal());
        assert!(WorkstreamState::Abandoned.is_terminal());
        assert!(!WorkstreamState::Blocked.is_terminal());
        assert!(WorkstreamState::Blocked.needs_attention());
        assert!(WorkstreamState::InReview.needs_attention());
        assert!(!WorkstreamState::InProgress.needs_attention());
    }

    #[test]
    fn states_round_trip_through_their_stored_form() {
        for &state in WorkstreamState::ALL {
            assert_eq!(WorkstreamState::parse(state.as_str()).unwrap(), state);
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::json!(state.as_str())
            );
        }
        let error = WorkstreamState::parse("RUNNING").expect_err("unknown state");
        assert_eq!(error.code, CoreErrorCode::Internal);
    }
}
