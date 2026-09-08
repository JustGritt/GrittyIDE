//! Gritty git (T100): read-only repository status and structured diffs.
//!
//! This crate is the only place in the tree that opens a Git repository, and
//! it is deliberately a *boundary*, not an abstraction over backends
//! (ADR-023 decision 3): there is no trait with one implementation and no
//! feature-flagged second library. What makes the choice reversible is that
//! `gix` types never leave this crate — [`RepositoryStatus`] and
//! [`DiffSummary`] are Gritty's own, so replacing the library is a rewrite of
//! `open.rs`/`status.rs`/`diff.rs` with the model and its tests unchanged.
//!
//! It knows nothing about workspaces-as-identity, operations, events or the
//! core protocol: `gritty-core::git` owns the [`WorkspaceId`] lookup, the
//! budget-then-operation hand-off and the error envelope, and reaches the
//! repository through [`status`] and [`diff`] here. That split is the same one
//! `gritty-terminal`/`gritty_core::terminal` already uses.
//!
//! ## What this crate will not do
//!
//! - **Nothing writes.** No staging, committing, checkout, branch or worktree
//!   creation, and no `.git` file is opened for writing — including the index
//!   lock, which a read never needs (T100's failure table). T400 will want
//!   writes; that is a later decision with a later risk review.
//! - **Nothing reaches the network.** `gix` is built without its transport,
//!   credential and prompt features, so there is no code path to a remote to
//!   begin with. Ahead/behind (see [`model::UpstreamStatus`]) compares two
//!   local refs and is therefore as stale as the user's last fetch.
//! - **No external program runs.** ADR-023's second decision, and the reason
//!   [`open`] exists as its own module: `gix` will honour a repository's own
//!   request to run a clean/smudge filter or a `diff.<driver>.textconv`, so a
//!   repository is opened with those switched off.
//!   `open::tests::no_program_named_by_the_repository_can_run` proves it
//!   against a repository that asks for one, because "in-process" is a
//!   configuration, not a property of the library (`AGENTS.md` rule 12).
//! - **Nothing is cached.** Every call reads the repository again. T100
//!   forbids a speculative index or cache without an ADR (`AGENTS.md` rule 9),
//!   and pagination is therefore recompute-and-skip rather than a held cursor.
//!
//! ## Untrusted input
//!
//! A repository's objects, refs, index and `.git/config` are a file format an
//! attacker may control (`AGENTS.md` rule 13). Paths, branch names and line
//! content are decoded lossily to UTF-8, passed through unchanged and never
//! interpreted; nothing from a repository is logged, and tracing records only
//! ids, counts and durations.
//!
//! [`WorkspaceId`]: https://docs.rs/gritty-core

pub mod diff;
pub mod error;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub mod log;
pub mod model;
pub mod open;
mod page;
mod path;
pub mod show;
pub mod status;

pub use diff::{DiffOptions, diff};
pub use error::GitError;
pub use log::{LogOptions, log};
pub use model::{
    BlobContent, CommitPage, CommitSignature, CommitSummary, DiffChangeKind, DiffHunk, DiffLimits,
    DiffLine, DiffLineKind, DiffSummary, EntryKind, FileDiff, FileState, FileStatus,
    MAX_SHOW_BYTES, NoTextReason, RepositoryState, RepositoryStatus, UpstreamStatus,
};
pub use open::{Repository, open};
pub use page::{CANCEL_CHECK_INTERVAL, DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, PageOptions};
pub use show::{ShowOptions, show};
pub use status::{StatusOptions, status};
