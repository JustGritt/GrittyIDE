//! Why the fake refused something.
//!
//! These are refusals, not run failures. A rejected step changes nothing and
//! the run carries on — the same shape as the core's transition table, where a
//! move that is not in the table is a `Conflict` and leaves the state alone.
//! The one thing that *does* end a run is [`crate::FakeAdvance`] reaching the
//! end of a script with nothing terminal in it, and that is reported as an
//! action rather than as an error.
//!
//! The core turns these into a `CoreError` when it needs one. This crate does
//! not build a `CoreError` itself, because it does not depend on `gritty-core`
//! (see [`crate`]); the mapping each variant asks for is on the variant.

use crate::phase::FakePhase;

/// A refusal from the fake.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FakeError {
    /// The step cannot happen in that phase — a duplicated or out-of-order
    /// script. The core's spelling for this is `Conflict`.
    #[error("a fake agent cannot {step} while {phase}")]
    OutOfOrder {
        step: &'static str,
        phase: FakePhase,
    },
    /// A `write` with no `proposeWrite` before it, or a `commandDone` with no
    /// command outstanding. Also `Conflict`: the script contradicts itself.
    #[error("a fake agent cannot {step} because nothing was outstanding")]
    Unpaired { step: &'static str },
    /// An out-of-band answer arrived when the run was not asking anything.
    /// `Conflict`.
    #[error("nothing is waiting to be answered while {phase}")]
    NothingToAnswer { phase: FakePhase },
    /// The run is over. `Conflict`, and idempotent at the caller: a cancel of
    /// an already-ended run is a no-op rather than this.
    #[error("the run already ended {phase}")]
    AlreadyEnded { phase: FakePhase },
    /// `runs.startFake` was given a preset name that does not exist. The core's
    /// spelling for this is `InvalidInput`, on the `script` field.
    #[error("unknown fake script preset {name:?}")]
    UnknownPreset { name: String },
}

impl FakeError {
    /// The `CoreErrorCode` this refusal should become, spelled as
    /// `CoreErrorCode`'s serde form spells it.
    ///
    /// A specification for the core to apply, for the same reason
    /// [`crate::FakeAction::run_events`] is: this crate cannot name the type.
    pub const fn core_error_code(&self) -> &'static str {
        match self {
            Self::OutOfOrder { .. }
            | Self::Unpaired { .. }
            | Self::NothingToAnswer { .. }
            | Self::AlreadyEnded { .. } => "Conflict",
            Self::UnknownPreset { .. } => "InvalidInput",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_name_the_code_the_core_should_use() {
        assert_eq!(
            FakeError::OutOfOrder {
                step: "write",
                phase: FakePhase::Idle,
            }
            .core_error_code(),
            "Conflict"
        );
        assert_eq!(
            FakeError::UnknownPreset {
                name: "nope".to_owned()
            }
            .core_error_code(),
            "InvalidInput"
        );
    }

    /// The message names both halves, because a caller reading a rejection has
    /// no other way to see why the script was wrong.
    #[test]
    fn out_of_order_names_the_step_and_the_phase() {
        let error = FakeError::OutOfOrder {
            step: "write",
            phase: FakePhase::AwaitingPermission,
        };
        let message = error.to_string();
        assert!(message.contains("write"), "{message}");
        assert!(message.contains("awaitingPermission"), "{message}");
    }
}
