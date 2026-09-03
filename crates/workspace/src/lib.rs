//! Gritty workspace domain (T020): the canonical workspace root, the policy
//! for paths that cross the UI/core boundary, and one-level directory
//! listing that never scans recursively.
//!
//! This crate knows nothing about Tauri, the core protocol or the operation
//! model. `gritty-core` owns workspace lifecycle (ids, events, operations) and
//! maps [`WorkspaceError`] onto the core error envelope.
//!
//! - [`WorkspaceRoot`]: an absolute, canonicalized directory; every path the
//!   UI sends is resolved against it and rejected when it escapes.
//! - [`WorkspacePath`]: the wire representation of a path inside the root
//!   (`/`-separated, relative, no `.`/`..`; `""` is the root).
//! - [`list_directory`]: one directory level, sorted, paginated, cancellable.
//! - [`fixtures`]: the deterministic `large-repo` generator used by tests and
//!   by `examples/generate_large_repo.rs`.

pub mod error;
pub mod fixtures;
pub mod listing;
pub mod path;
pub mod root;

pub use error::WorkspaceError;
pub use listing::{
    DEFAULT_PAGE_SIZE, DirEntry, DirEntryKind, DirectoryPage, IGNORED_DIRECTORY_NAMES, ListOptions,
    ListStats, Listing, MAX_PAGE_SIZE, SymlinkInfo, list_directory,
};
pub use path::WorkspacePath;
pub use root::{ResolvedPath, WorkspaceRoot, canonicalize};
