//! Wire types of the `git.status` and `git.diff` commands (T100).
//!
//! The domain payloads — [`RepositoryStatus`], [`DiffSummary`] — live in
//! `gritty-git` beside the code that builds them, the way [`DirectoryPage`]
//! lives in `gritty-workspace`. What this module adds is the request shape and
//! the three-way response every read of a repository has.
//!
//! **Both responses are a tagged union with a `NotARepository` arm, and that
//! arm is a value rather than an error.** A user who opens a folder that
//! happens not to be a repository has not made a mistake, and Gritty's job is
//! to say what is there (`docs/architecture/security.md`: nothing is
//! filtered). Making it an error would put a red envelope in front of every
//! non-repository workspace and teach the UI to ignore errors.
//!
//! The `Pending` arm mirrors [`ListDirectoryResponse`]: work that outlives a
//! short budget becomes a cancellable operation, and no new event family is
//! invented for it — the existing `Operation*` events carry it
//! (`AGENTS.md` rule 10, and rule 9 against speculative layers).
//!
//! [`DirectoryPage`]: gritty_workspace::DirectoryPage
//! [`ListDirectoryResponse`]: crate::workspace::ListDirectoryResponse

use gritty_git::{DiffSummary, RepositoryStatus};
use gritty_workspace::WorkspacePath;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::operation::Operation;
use crate::protocol::ids::WorkspaceId;

/// `git.status`: the state of the repository at the open workspace's root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusRequest {
    pub workspace_id: WorkspaceId,
    /// Continue a truncated listing (`RepositoryStatus.cursor`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cursor: Option<String>,
    /// Entries per page, `1..=5000`; defaults to 5000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub page_size: Option<u32>,
    /// Also report files `.gitignore` excludes. Off by default: an ignored
    /// `target/` directory is a million entries the user did not ask for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub include_ignored: Option<bool>,
}

/// What `git.status` returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum GitStatusResponse {
    /// The repository was read within the budget.
    Ready { repository: Box<RepositoryStatus> },
    /// The workspace root is a directory, and not a Git repository. A value,
    /// not an error.
    NotARepository,
    /// The read outlived its budget and continues as a cancellable operation
    /// whose `OperationCompleted.result` is the [`RepositoryStatus`].
    Pending { operation: Box<Operation> },
}

/// `git.diff`: what changed between a revision and the working tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffRequest {
    pub workspace_id: WorkspaceId,
    /// Revision to compare the working tree against; `HEAD` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub target: Option<String>,
    /// Restrict the diff to one file. The bounded way to ask for the hunks of
    /// the file the user selected, instead of paging every changed file's
    /// hunks to reach one of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<WorkspacePath>,
    /// Continue a truncated listing (`DiffSummary.cursor`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cursor: Option<String>,
    /// Files per page, `1..=5000`. Defaults to
    /// [`DEFAULT_DIFF_PAGE_SIZE`](super::DEFAULT_DIFF_PAGE_SIZE), which is far
    /// below the maximum because every entry carries hunks: 5000 files of
    /// hunks is a response nobody reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub page_size: Option<u32>,
}

/// What `git.diff` returns. The same three arms as [`GitStatusResponse`], for
/// the same reasons.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum GitDiffResponse {
    Ready {
        diff: Box<DiffSummary>,
    },
    NotARepository,
    /// `OperationCompleted.result` is the [`DiffSummary`].
    Pending {
        operation: Box<Operation>,
    },
}
