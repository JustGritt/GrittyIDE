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
//! - [`mutate`] (T042): create, rename/move, copy, remove and trash, resolved
//!   through the same root policy and never following a final symlink.
//! - [`file`] and [`detect`] (T030): bounded whole-file reads, content
//!   detection (binary / UTF-8 / BOM / large), hashing, atomic replacement.
//! - [`fixtures`]: the deterministic `large-repo` generator and the
//!   `weird-files` stager used by tests and by the examples.

pub mod detect;
pub mod error;
pub mod file;
pub mod fixtures;
pub mod listing;
pub mod mutate;
pub mod path;
pub mod root;
pub mod watch;

pub use detect::{
    ContentKind, DocumentEncoding, LARGE_FILE_THRESHOLD, MAX_DOCUMENT_BYTES, decode_text,
    detect_content, encode_text, is_large_file, language_id,
};
pub use error::WorkspaceError;
pub use file::{
    AtomicWriteFault, ContentHash, DiskProbe, FileSnapshot, FileStat, is_temp_file_name,
    probe_file, read_file, write_file_atomic,
};
pub use listing::{
    DEFAULT_PAGE_SIZE, DirEntry, DirEntryKind, DirectoryPage, IGNORED_DIRECTORY_NAMES, ListOptions,
    ListStats, Listing, MAX_PAGE_SIZE, SymlinkInfo, is_ignored, list_directory,
};
pub use mutate::{CopyOutcome, EntryKind};
pub use path::WorkspacePath;
pub use root::{ResolvedPath, WorkspaceRoot, canonicalize};
pub use watch::{
    ChangeBatch, DEBOUNCE, Degradation, MAX_DIRECTORIES, MAX_WAIT, Watch, WatchConfig, WatchEvent,
    WatchStrategy, watch,
};
