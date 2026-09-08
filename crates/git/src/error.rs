//! Failure vocabulary of the Git layer.
//!
//! `gritty-core` maps these onto the `CoreError` envelope; nothing above this
//! crate ever sees a `gix` error type. The variants are deliberately coarse:
//! they say what the caller can do about it, not which of `gix`'s several
//! dozen error enums produced it.

use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// The path exists and is readable but is not a Git repository. This is
    /// **not** an error at the protocol level (`security.md`: nothing is
    /// filtered, a plain folder is a value): `gritty-core` turns it into the
    /// `NotARepository` arm of the response.
    #[error("{} is not a git repository", path.display())]
    NotARepository { path: PathBuf },

    /// The revision string did not resolve to an object in this repository.
    /// The caller supplied it, so this is `InvalidInput` upstream.
    #[error("could not resolve revision {spec}")]
    UnknownRevision { spec: String },

    /// A revision resolved to something that is not a commit or a tree, so
    /// there is nothing to diff against.
    #[error("revision {spec} does not name a commit or a tree")]
    NotATree { spec: String },

    /// The revision resolved, and the path is not in it. Kept apart from
    /// [`Self::UnknownRevision`] because the two send a reader to different
    /// places: a bad revision is a mistake in the request, while a path that is
    /// absent at a revision is usually the true and interesting answer — the
    /// file had not been added yet.
    #[error("{path} does not exist at {spec}")]
    NotFoundAtRevision { path: String, spec: String },

    /// The repository was found but something inside it could not be read: a
    /// corrupt object, an unreadable pack, a permission failure, a stale lock
    /// held by another process. Retryable, because the common cases are
    /// transient.
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },

    /// The repository was found but `gix` refused to answer. Kept apart from
    /// [`Self::Io`] because it is a repository-shaped failure rather than a
    /// machine-shaped one, and retrying it unchanged will not help.
    #[error("{context}: {reason}")]
    Repository { context: String, reason: String },

    /// The caller's cancellation predicate returned `true`. Nothing partial is
    /// returned with it.
    #[error("git operation cancelled")]
    Cancelled,
}

impl GitError {
    pub(crate) fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// Wrap any `gix` error. The message is the library's own `Display`, which
    /// names objects by id and paths by name — never file *content*, which is
    /// what `AGENTS.md` rule 13 is about.
    pub(crate) fn repository(context: impl Into<String>, reason: impl std::fmt::Display) -> Self {
        Self::Repository {
            context: context.into(),
            reason: reason.to_string(),
        }
    }

    /// `true` when retrying the same request unchanged may reasonably succeed.
    /// `gritty-core` uses this for the envelope's `retryable` flag.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Io { .. })
    }

    /// `true` when the caller supplied something unusable, as opposed to the
    /// repository or the machine failing: the core answers `InvalidInput`
    /// rather than `Io`.
    pub fn is_caller_fault(&self) -> bool {
        matches!(self, Self::UnknownRevision { .. } | Self::NotATree { .. })
    }
}
