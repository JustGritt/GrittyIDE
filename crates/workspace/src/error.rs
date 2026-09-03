//! Errors of the workspace domain.
//!
//! The crate must not depend on `gritty-core`, so it cannot produce the core
//! error envelope itself; every variant maps onto exactly one `CoreErrorCode`
//! in `gritty_core::workspace`.

use std::io;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    /// A path argument violates the path policy; `field` names the argument.
    #[error("{field}: {reason}")]
    InvalidPath { field: &'static str, reason: String },
    /// The path (or a component of it) does not exist.
    #[error("{0}")]
    NotFound(String),
    /// The filesystem refused access, or the path resolves outside the root.
    #[error("{0}")]
    PermissionDenied(String),
    /// A directory was required.
    #[error("{0}")]
    NotADirectory(String),
    /// Any other filesystem failure.
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
    /// The caller asked to stop; partial results were discarded.
    #[error("cancelled")]
    Cancelled,
}

impl WorkspaceError {
    pub fn invalid_path(field: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidPath {
            field,
            reason: reason.into(),
        }
    }

    /// Classify an I/O failure that happened while touching `what`.
    pub fn from_io(source: io::Error, what: impl Into<String>) -> Self {
        let what = what.into();
        match source.kind() {
            io::ErrorKind::NotFound => Self::NotFound(format!("{what} does not exist")),
            io::ErrorKind::PermissionDenied => {
                Self::PermissionDenied(format!("{what} is not accessible: {source}"))
            }
            io::ErrorKind::NotADirectory => {
                Self::NotADirectory(format!("{what} is not a directory"))
            }
            _ => Self::Io {
                context: what,
                source,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_kinds_map_to_domain_variants() {
        let not_found = WorkspaceError::from_io(io::Error::from(io::ErrorKind::NotFound), "x");
        assert!(matches!(not_found, WorkspaceError::NotFound(m) if m == "x does not exist"));
        let denied = WorkspaceError::from_io(io::Error::from(io::ErrorKind::PermissionDenied), "y");
        assert!(matches!(denied, WorkspaceError::PermissionDenied(_)));
        let other = WorkspaceError::from_io(io::Error::other("disk"), "z");
        assert!(matches!(other, WorkspaceError::Io { ref context, .. } if context == "z"));
        assert_eq!(other.to_string(), "z: disk");
    }
}
