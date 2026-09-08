//! Why the adapter refused, or could not go on.
//!
//! The core turns these into a `CoreError` when it needs one. This crate does
//! not build one itself, because it does not depend on `gritty-core` (see
//! [`crate`]); the code each variant asks for is published on the variant, the
//! same arrangement `gritty_agent_fake::FakeError::core_error_code` uses.

use std::time::Duration;

/// A refusal from the Claude Code adapter.
#[derive(Debug, thiserror::Error)]
pub enum ClaudeError {
    /// The binary is not on `PATH`, is not executable, or the spawn failed for
    /// some other operating-system reason.
    ///
    /// The task's failure table is explicit that this ends the run `FAILED` and
    /// never `INTERRUPTED`: nothing was ever being supervised.
    #[error("the agent binary {program} could not be started: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    /// The child spawned but never announced itself with `system`/`init`
    /// inside the bounded wait.
    ///
    /// This is the case [`crate::ClaudeObservation::Ready`] exists for: a run
    /// stays `STARTING` until the session is actually usable, and a child that
    /// never gets there fails rather than sitting in `STARTING` forever.
    #[error("the agent did not report a ready session within {}s", wait.as_secs())]
    NotReady { wait: Duration },
    /// Something was written to a child whose stdin is already closed — a
    /// prompt for a run that has ended, or an answer to an ask that died with
    /// the process.
    #[error("the agent is no longer accepting input: {reason}")]
    Closed { reason: &'static str },
    /// The pipe to the child broke while it was being written to.
    #[error("writing to the agent failed: {source}")]
    Write {
        #[source]
        source: std::io::Error,
    },
    /// `--session-id` takes a UUID and Gritty mints it. A value that is not one
    /// is a caller bug, caught before a process exists rather than after.
    #[error("a session id must be a UUID, not {value:?}")]
    InvalidSessionId { value: String },
    /// A working directory that is not a directory, or cannot be read.
    ///
    /// Checked before the spawn so the failure names the directory instead of
    /// arriving as an opaque `ENOENT` from `posix_spawn`, which cannot say
    /// whether the *binary* or the *cwd* was missing.
    #[error("the working directory {path} is not usable: {reason}")]
    WorkingDirectory { path: String, reason: &'static str },
}

impl ClaudeError {
    /// The `CoreErrorCode` this refusal should become, spelled as
    /// `CoreErrorCode`'s serde form spells it.
    ///
    /// A specification for the core to apply, for the same reason
    /// [`crate::ClaudeObservation::run_events`] is: this crate cannot name the
    /// type. `Io` for everything the operating system decided, `Conflict` for
    /// a write to a run that is over, `InvalidInput` for an argument that was
    /// never going to work.
    pub const fn core_error_code(&self) -> &'static str {
        match self {
            Self::Spawn { .. } | Self::Write { .. } => "Io",
            Self::NotReady { .. } => "Timeout",
            Self::Closed { .. } => "Conflict",
            Self::InvalidSessionId { .. } | Self::WorkingDirectory { .. } => "InvalidInput",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_name_the_code_the_core_should_use() {
        assert_eq!(
            ClaudeError::Spawn {
                program: "claude".to_owned(),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            }
            .core_error_code(),
            "Io"
        );
        assert_eq!(
            ClaudeError::NotReady {
                wait: Duration::from_secs(30)
            }
            .core_error_code(),
            "Timeout"
        );
        assert_eq!(
            ClaudeError::Closed { reason: "ended" }.core_error_code(),
            "Conflict"
        );
        assert_eq!(
            ClaudeError::InvalidSessionId {
                value: "nope".to_owned()
            }
            .core_error_code(),
            "InvalidInput"
        );
    }

    /// A spawn failure has to name the binary: "claude is not installed" and
    /// "claude crashed" are different problems and the user fixes them
    /// differently.
    #[test]
    fn a_spawn_failure_names_the_binary() {
        let error = ClaudeError::Spawn {
            program: "/usr/local/bin/claude".to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        };
        let message = error.to_string();
        assert!(message.contains("/usr/local/bin/claude"), "{message}");
    }
}
