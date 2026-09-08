//! Errors of the search domain.
//!
//! The crate must not depend on `gritty-core`, so it cannot produce the core
//! error envelope itself; every variant maps onto exactly one
//! `CoreErrorCode` in `gritty_core::search`, the way [`WorkspaceError`] does
//! in `gritty_core::workspace`.

use std::io;

use gritty_workspace::WorkspaceError;

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// The request cannot be turned into a search at all: an empty query, one
    /// that is too long, or a pattern the regex engine refuses. Fast
    /// validation, before an operation exists.
    #[error("{field}: {reason}")]
    InvalidQuery { field: &'static str, reason: String },
    /// The workspace root itself failed (it disappeared, or became
    /// unreadable). A single *file* that fails is counted and skipped, never
    /// raised: see [`crate::SkipCounts`].
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    /// The walk could not read the tree it was given.
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
    /// The caller asked to stop; partial results were already delivered to the
    /// sink and are not withdrawn.
    #[error("cancelled")]
    Cancelled,
}

impl SearchError {
    pub fn invalid_query(field: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidQuery {
            field,
            reason: reason.into(),
        }
    }

    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}
