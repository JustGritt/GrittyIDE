//! Operation status and the only transitions the registry accepts.
//!
//! ```text
//! CREATED ──► RUNNING ──► COMPLETED
//!    │           ├──────► FAILED
//!    │           └──────► CANCELLED
//!    └──────────────────► CANCELLED   (cancelled before the work started)
//! ```
//!
//! Terminal states never change again: a late or duplicate completion signal
//! is an [`InvalidTransition`], which the registry logs and drops. This is
//! what guarantees that a cancelled operation never later shows as completed.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OperationStatus {
    Created,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid operation transition {from:?} -> {to:?}")]
pub struct InvalidTransition {
    pub from: OperationStatus,
    pub to: OperationStatus,
}

impl OperationStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub const fn can_transition_to(self, to: Self) -> bool {
        matches!(
            (self, to),
            (Self::Created, Self::Running)
                | (Self::Created, Self::Cancelled)
                | (Self::Running, Self::Completed)
                | (Self::Running, Self::Failed)
                | (Self::Running, Self::Cancelled)
        )
    }

    pub fn transition(self, to: Self) -> Result<Self, InvalidTransition> {
        if self.can_transition_to(to) {
            Ok(to)
        } else {
            Err(InvalidTransition { from: self, to })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OperationStatus::*;
    use super::*;

    const ALL: [OperationStatus; 5] = [Created, Running, Completed, Failed, Cancelled];

    #[test]
    fn only_the_documented_transitions_are_valid() {
        let valid = [
            (Created, Running),
            (Created, Cancelled),
            (Running, Completed),
            (Running, Failed),
            (Running, Cancelled),
        ];
        for from in ALL {
            for to in ALL {
                let expected = valid.contains(&(from, to));
                assert_eq!(from.can_transition_to(to), expected, "{from:?} -> {to:?}");
                assert_eq!(from.transition(to).is_ok(), expected, "{from:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn terminal_states_never_change() {
        for from in [Completed, Failed, Cancelled] {
            assert!(from.is_terminal());
            for to in ALL {
                assert_eq!(from.transition(to), Err(InvalidTransition { from, to }));
            }
        }
        assert!(!Created.is_terminal());
        assert!(!Running.is_terminal());
    }

    #[test]
    fn serializes_screaming_snake_case() {
        assert_eq!(serde_json::to_string(&Cancelled).unwrap(), "\"CANCELLED\"");
        let back: OperationStatus = serde_json::from_str("\"RUNNING\"").unwrap();
        assert_eq!(back, Running);
    }
}
