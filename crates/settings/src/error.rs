//! What can go wrong, in the terms the core turns into an error envelope.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// A client asked for a key this build does not declare.
    ///
    /// Deliberately an error, unlike an unknown key found *in the file*, which
    /// is preserved (ADR-031 §5). A file containing one is a downgrade; a
    /// client asking for one is a bug, and storing it would write something
    /// nothing will ever read.
    #[error("unknown setting `{0}`")]
    UnknownKey(String),

    /// A value that does not fit its setting's kind.
    #[error("`{key}` expects {expected}")]
    InvalidValue { key: String, expected: String },

    /// The document was loaded from a file that did not parse, so writing
    /// would destroy whatever the user was editing (ADR-031 §5).
    #[error("`{0}` could not be parsed; fix or remove the file before saving from Gritty")]
    Unparseable(PathBuf),

    /// There is nowhere to store settings — no application data directory.
    /// Values apply for this session only.
    #[error("no settings file is available; changes apply to this session only")]
    Unavailable,

    /// The atomic write refused or failed. The previous file is intact —
    /// that is what temp+rename buys — so the value applies for this session
    /// and the failure is reported rather than swallowed.
    #[error("could not save `{path}`: {reason}")]
    Write { path: PathBuf, reason: String },

    #[error("{action} `{path}`: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl SettingsError {
    pub(crate) fn io(
        action: &'static str,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }
}
