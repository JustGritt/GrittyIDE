//! What a write can fail with, and how much of that this crate is willing to
//! claim.
//!
//! ADR-029 says a write "has no output format to parse — explicit pathspecs
//! after `--`, and an exit code". That is true of the *success* path and an
//! over-claim on the failure path: `git` exits 1 for nearly everything, so an
//! exit code alone cannot tell a refusing `pre-commit` hook from an unmerged
//! path from a held lock.
//!
//! Rather than pattern-match `git`'s prose — which is localised, version-
//! dependent, and exactly the parser ADR-023 refused to maintain — this crate
//! classifies only what it can determine **structurally**:
//!
//! | Outcome | How it is known |
//! |---|---|
//! | [`GitWriteError::Locked`] | `index.lock` exists in the git directory |
//! | [`GitWriteError::Unavailable`] | the probe found no usable `git` |
//! | [`GitWriteError::Cancelled`] | this crate killed the process group |
//! | [`GitWriteError::Refused`] | non-zero exit; `git`'s own stderr travels with it |
//!
//! `Refused` carries the child's stderr **to the caller** so a hook's own words
//! reach the user who has to act on them. It is never logged: it is output from
//! a program the repository named, and rule 13 makes that untrusted input.

use std::process::ExitStatus;

/// Why a write did not happen.
#[derive(Debug, thiserror::Error)]
pub enum GitWriteError {
    /// No usable `git`. Carries the probe's reason so the UI can say *why* the
    /// write affordances are disabled rather than merely that they are
    /// (ADR-029 decision 5).
    #[error("git is unavailable: {reason}")]
    Unavailable { reason: String },

    /// Another process holds `index.lock`. Retryable, and named rather than
    /// retried: a silent retry loop against a lock someone else's `git` is
    /// holding is how a hang gets built.
    #[error("another git process is holding the index lock")]
    Locked,

    /// `git` exited non-zero. `stderr` is the child's own, verbatim.
    #[error("git {subcommand} failed")]
    Refused {
        subcommand: &'static str,
        /// `None` when the child was signalled rather than exiting.
        code: Option<i32>,
        /// The child's stderr, trimmed and bounded. For the caller, never the log.
        stderr: String,
    },

    /// The operation was cancelled and the process group was killed. Nothing
    /// partial is claimed: whether the commit happened is reported by the
    /// caller reading HEAD, not guessed here.
    #[error("the write was cancelled")]
    Cancelled,

    /// The child could not be spawned, or its output could not be read.
    #[error("{context}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
}

impl GitWriteError {
    /// Worth trying again unchanged. Only a held lock is: a hook that refused
    /// will refuse again, and an absent binary stays absent.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Locked)
    }

    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// Build a refusal from a finished child.
    pub(crate) fn refused(subcommand: &'static str, status: ExitStatus, stderr: String) -> Self {
        Self::Refused {
            subcommand,
            code: status.code(),
            stderr: bound(stderr),
        }
    }
}

/// How much of a child's stderr crosses back. A hook can print a whole test
/// suite; the user needs the reason, not the transcript.
pub(crate) const MAX_STDERR_BYTES: usize = 8 * 1024;

/// Trim, and bound at a character boundary so the result is still a `String`.
pub(crate) fn bound(mut text: String) -> String {
    let trimmed = text.trim();
    if trimmed.len() != text.len() {
        text = trimmed.to_owned();
    }
    if text.len() <= MAX_STDERR_BYTES {
        return text;
    }
    let mut end = MAX_STDERR_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str("\n… (truncated)");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_held_lock_is_retryable() {
        assert!(GitWriteError::Locked.is_retryable());
        assert!(!GitWriteError::Cancelled.is_retryable());
        assert!(
            !GitWriteError::Unavailable {
                reason: "not found".into()
            }
            .is_retryable()
        );
    }

    #[test]
    fn stderr_is_bounded_at_a_character_boundary() {
        // A multi-byte character straddling the cut would panic a naive
        // truncate; a hook that prints UTF-8 is not exotic.
        let long = "é".repeat(MAX_STDERR_BYTES);
        let bounded = bound(long);
        assert!(bounded.len() <= MAX_STDERR_BYTES + 32);
        assert!(bounded.ends_with("… (truncated)"));
    }

    #[test]
    fn short_stderr_survives_intact_but_trimmed() {
        assert_eq!(bound("  hook said no\n".to_owned()), "hook said no");
    }
}
