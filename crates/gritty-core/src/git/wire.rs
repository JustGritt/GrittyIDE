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

use gritty_git::{BlobContent, CommitPage, DiffSummary, RepositoryStatus};
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
    Ready {
        repository: Box<RepositoryStatus>,
        /// Whether the write half is usable, and why not when it is not
        /// (T102-C). Absent when the probe has not run — never assume "yes"
        /// from its absence.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        write: Option<GitWriteAvailability>,
    },
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

/// `git.log`: a bounded page of the history behind a revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GitLogRequest {
    pub workspace_id: WorkspaceId,
    /// Revision to walk back from; `HEAD` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub start: Option<String>,
    /// Restrict the walk to commits that touched this path. The bounded way to
    /// ask "what happened to this file", instead of paging all of history and
    /// filtering in the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<WorkspacePath>,
    /// Continue a truncated walk (`CommitPage.cursor`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cursor: Option<String>,
    /// Commits per page, `1..=5000`. Defaults to
    /// [`DEFAULT_LOG_PAGE_SIZE`](super::DEFAULT_LOG_PAGE_SIZE), which is small
    /// because a history list is read a screen at a time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub page_size: Option<u32>,
}

/// What `git.log` returns. The same three arms as [`GitStatusResponse`], for
/// the same reasons.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum GitLogResponse {
    Ready {
        page: Box<CommitPage>,
    },
    NotARepository,
    /// `OperationCompleted.result` is the [`CommitPage`].
    Pending {
        operation: Box<Operation>,
    },
}

/// `git.show`: the content of one path at one revision.
///
/// This is the *original* side of a diff. Unlike the other three reads it names
/// exactly one object, so it does not page: a blob is whole or it is refused
/// for being too large, and there is no middle state a cursor could describe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GitShowRequest {
    pub workspace_id: WorkspaceId,
    pub path: WorkspacePath,
    /// Revision to read the path at; `HEAD` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub revision: Option<String>,
}

/// What `git.show` returns. The same three arms as [`GitStatusResponse`], for
/// the same reasons.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum GitShowResponse {
    Ready {
        blob: Box<BlobContent>,
    },
    NotARepository,
    /// `OperationCompleted.result` is the [`BlobContent`].
    Pending {
        operation: Box<Operation>,
    },
}

// ---------------------------------------------------------------------------
// The write half (T102-C, behind ADR-029)
// ---------------------------------------------------------------------------
//
// Four commands that move a file between the bands the Changes panel already
// draws, and one that turns the staged band into a commit. They differ from the
// four reads above in three ways worth stating once, here, rather than on each
// type.
//
// **The pathspec is always explicit.** There is no "all" flag on any request.
// Gritty ships a terminal, so the user's own `git` may be running beside this
// one; a wildcard write would sweep up whatever another process changed a
// moment ago (ADR-029 decision 3).
//
// **The response says what changed**, modelled on `FileOperationResult`: the
// paths to re-read, and the repository as it now stands, so the panel converges
// on this response instead of issuing a `git.status` behind it. Nothing emits a
// new `CoreEvent` — the response is the signal, exactly as `files.rs` decided.
//
// **All four share one `OperationKind`, `git.write`**, the way `files.transfer`
// covers both `files.move` and `files.delete`. A UI matching on the kind should
// not have to know which of four verbs is running.

/// `git.stage`: copy the worktree content of named paths into the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GitStageRequest {
    pub workspace_id: WorkspaceId,
    /// Explicit, and never empty. No wildcard, no "stage everything".
    pub paths: Vec<WorkspacePath>,
}

/// `git.unstage`: take named paths back out of the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GitUnstageRequest {
    pub workspace_id: WorkspaceId,
    pub paths: Vec<WorkspacePath>,
}

/// `git.discard`: throw away the changes to named paths.
///
/// The one irreversible corner of the write surface, and the reason it is not
/// actually irreversible: a tracked path is **stashed** before it is reverted,
/// and an untracked one goes to the **Trash** (`files.delete`'s disposition).
/// Both are recoverable, which is recoverability's place in the priority order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GitDiscardRequest {
    pub workspace_id: WorkspaceId,
    pub paths: Vec<WorkspacePath>,
    /// Discard anyway when an open document under one of the paths is dirty.
    /// Without it that is a `Conflict` carrying `details.dirtyDocuments`
    /// (hard rule 7), exactly as `files.delete` refuses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub force: Option<bool>,
}

/// `git.commit`: turn the staged band into a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitRequest {
    pub workspace_id: WorkspaceId,
    /// Committed verbatim. Empty or whitespace-only is `InvalidInput`: there is
    /// no such thing as a commit with no message.
    pub message: String,
    /// Replace the previous commit rather than adding one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub amend: Option<bool>,
    /// Append a `Signed-off-by` trailer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub signoff: Option<bool>,
    /// Skip the repository's own hooks. Running them is what makes a Gritty
    /// commit a real commit (ADR-029 decision 4), so this is a visible choice
    /// and never a default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub no_verify: Option<bool>,
}

/// Where a discard put the work it took away.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StashRef {
    /// Full hex id of the stash commit, so it can be named even after further
    /// stashes push it down the list.
    pub commit: String,
    /// What `git stash list` calls it *right now* — `stash@{0}`. Positional,
    /// and stale as soon as anything else is stashed, which is why `commit` is
    /// carried beside it.
    pub name: String,
}

/// What a write changed. Modelled on `FileOperationResult`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GitWriteResult {
    /// Paths whose status the write moved. An over-approximation is fine and
    /// expected — re-reading a row the user cannot see costs nothing.
    pub changed: Vec<WorkspacePath>,
    /// The repository as it stands after the write, so the caller converges on
    /// this response rather than issuing a `git.status` behind it.
    pub repository: Box<RepositoryStatus>,
    /// Full hex id of the commit this write created, when it created one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub commit: Option<String>,
    /// Present when a discard stashed tracked work, so the UI can say where it
    /// went rather than merely that it is gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stash: Option<StashRef>,
    /// Untracked paths sent to the Trash instead of the stash. `git stash` does
    /// not take untracked files without `-u`, and `-u` would stash *every*
    /// untracked file rather than the named ones — so these travel the route
    /// `files.delete` already established.
    pub trashed: Vec<WorkspacePath>,
}

/// What all four writes return.
///
/// One response type rather than four, because the arms and their meanings are
/// identical. `Pending` is reachable from any of them, but in practice only a
/// commit takes it: a `pre-commit` hook can run a whole test suite, while the
/// other three are index operations that finish in milliseconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum GitWriteResponse {
    Ready {
        result: Box<GitWriteResult>,
    },
    /// The workspace root is a directory and not a Git repository. A value, not
    /// an error, exactly as for the reads.
    NotARepository,
    /// The write outlived its budget and continues as a cancellable operation
    /// whose `OperationCompleted.result` is the [`GitWriteResult`]. Cancelling
    /// it kills the hook's process group.
    Pending {
        operation: Box<Operation>,
    },
}

/// Whether the write half of the git surface can be used at all.
///
/// ADR-029 decision 5: a missing or too-old `git` disables staging and
/// committing **with a stated reason**, never silently. This rides on
/// `git.status` — which the panel already reads — so the UI learns the verdict
/// before the user reaches for a button, rather than on the first failed click.
///
/// It is deliberately *not* a field of [`RepositoryStatus`]: that type belongs
/// to `gritty-git`, which must not know that a write adapter exists (ADR-029
/// decision 2 keeps the two crates independent). Composing them is
/// `gritty-core::git`'s job, and this is that composition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GitWriteAvailability {
    pub available: bool,
    /// `git version 2.50.1 (Apple Git-155)`, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    /// Why not, in words meant for the user. Present exactly when
    /// `available` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
}
