//! Failure vocabulary of the persistence layer. The core maps these onto the
//! `CoreError` envelope; the desktop shell never sees SQLite errors directly.

use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The file is not a database or fails an integrity check.
    #[error("database {} is corrupt: {reason}", path.display())]
    Corrupt { path: PathBuf, reason: String },
    /// The file was written by a newer Gritty; migrations are forward-only.
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    NewerSchema { found: i64, supported: i64 },
    /// A migration failed; its transaction was rolled back, nothing was applied.
    #[error("migration {version} ({name}) failed: {source}")]
    Migration {
        version: i64,
        name: &'static str,
        #[source]
        source: rusqlite::Error,
    },
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
}

impl PersistenceError {
    /// `true` when the database file itself is damaged (as opposed to a
    /// transient or logical failure): the caller moves it aside and starts fresh.
    pub fn is_corrupt(&self) -> bool {
        match self {
            Self::Corrupt { .. } => true,
            Self::Sqlite(error) | Self::Migration { source: error, .. } => is_corrupt_sqlite(error),
            Self::NewerSchema { .. } | Self::Io { .. } => false,
        }
    }

    pub(crate) fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}

pub(crate) fn is_corrupt_sqlite(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(
                failure.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            )
    )
}
