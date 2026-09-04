//! Failure vocabulary of the terminal layer. The core maps these onto the
//! `CoreError` envelope; the desktop shell never sees PTY errors directly.

use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    /// The pseudo-terminal itself could not be allocated (fd exhaustion,
    /// a sandbox without `/dev/ptmx`).
    #[error("could not open a pseudo-terminal: {0}")]
    OpenPty(String),
    /// The shell could not be started. Kept apart from [`Self::OpenPty`] so the
    /// core can answer "your shell is missing" instead of "PTY unavailable".
    #[error("could not start {}: {source}", program.display())]
    Spawn {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The requested working directory is not a usable directory.
    #[error("{} is not a usable working directory: {reason}", path.display())]
    Cwd { path: PathBuf, reason: String },
    /// Writing to, resizing or signalling a session that is already gone.
    #[error("the terminal has exited")]
    Exited,
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
}

impl TerminalError {
    pub(crate) fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// `true` when the caller supplied something unusable (as opposed to the
    /// host failing): the core answers `InvalidInput`/`NotFound` rather than `Io`.
    pub fn is_caller_fault(&self) -> bool {
        match self {
            Self::Cwd { .. } => true,
            // `InvalidInput` is a path the caller malformed (a relative
            // shell), not a machine that failed: it belongs on this side.
            Self::Spawn { source, .. } => matches!(
                source.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::PermissionDenied
                    | io::ErrorKind::InvalidInput
            ),
            Self::OpenPty(_) | Self::Exited | Self::Io { .. } => false,
        }
    }
}
