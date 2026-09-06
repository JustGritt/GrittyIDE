//! The Git domain model: what Gritty knows about a repository, in Gritty's own
//! types.
//!
//! Two rules shape everything here.
//!
//! **`AGENTS.md` rule 5 — raw `git diff` is never the ChangeSet domain model.**
//! No field in this module carries a diff, a status or a hunk as one opaque
//! string. [`DiffLine::content`] is a single line of a single file, which is
//! the smallest unit the UI can render and the point at which text stops being
//! a format and starts being data.
//!
//! **`AGENTS.md` rule 13 — repository content is untrusted input.** Branch
//! names, paths and commit ids come out of files an attacker may have written.
//! They are decoded lossily to UTF-8, passed through unchanged and never
//! interpreted (`docs/architecture/security.md`, the same policy the workspace
//! uses for filenames). Nothing here is logged.
//!
//! What is deliberately **absent**: any link to a workstream, any test or
//! review status, and anything an agent reported. That is `ChangeSet`
//! (`docs/Gritty IDE Project Plan.md` §7), it is workstream-linked, and rule 6
//! keeps it out of here. This module builds its primitives, not itself.

use gritty_workspace::WorkspacePath;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Which multi-step operation the repository is in the middle of.
///
/// Named rather than reported as one opaque "dirty" flag, because the answer
/// changes what the user can do next: a rebase in progress and a clean
/// repository are not two shades of the same thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RepositoryState {
    /// No sequencer operation is in progress. Says nothing about whether the
    /// worktree has changes — that is what the entries are for.
    Clean,
    Merging,
    Rebasing,
    CherryPicking,
    Reverting,
    Bisecting,
    /// `git am` is applying a mailbox.
    ApplyingMailbox,
    /// The repository has no working tree. Status has no worktree side and a
    /// worktree diff is an empty, labelled result rather than an error.
    Bare,
}

/// How far the current branch has diverged from its upstream, counted locally.
///
/// Nothing is fetched to produce this (T100 explicitly forbids remote
/// operations): it compares the local branch with the local copy of its
/// tracking ref, so it is as stale as the last fetch the user ran. Absent
/// entirely when the branch has no upstream, rather than reported as `0/0`,
/// which would be a different and wrong claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamStatus {
    /// The tracking ref's full name, e.g. `refs/remotes/origin/main`.
    pub ref_name: String,
    /// Commits on the branch that the upstream does not have.
    pub ahead: u32,
    /// Commits on the upstream that the branch does not have.
    pub behind: u32,
}

/// What happened to one file on one side of the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FileState {
    /// Unchanged on this side. An entry with `Unmodified` on both sides is
    /// never emitted; it exists so one side can be unchanged while the other
    /// is not.
    Unmodified,
    Added,
    Modified,
    Deleted,
    /// The content moved from [`FileStatus::renamed_from`].
    Renamed,
    /// Regular file became a symlink, a symlink became a directory, and so on.
    TypeChanged,
    /// Present in the worktree and not in the index. Only ever the worktree
    /// side.
    Untracked,
    /// Excluded by `.gitignore`. Only reported when the caller asks for
    /// ignored files, which the default does not.
    Ignored,
}

/// What kind of thing the entry is, so a submodule can be *reported* without
/// being *entered* (T100 explicitly forbids recursion into one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntryKind {
    File,
    Symlink,
    /// A nested repository. Gritty says it changed; it does not say how.
    Submodule,
    /// An untracked directory the walk collapsed rather than expanding into
    /// its files.
    Directory,
}

/// One changed path in a repository status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileStatus {
    /// Repository-relative, which is workspace-relative because the
    /// repository is opened *at* the workspace root and never discovered
    /// above it.
    pub path: WorkspacePath,
    /// HEAD → index.
    pub index: FileState,
    /// Index → worktree.
    pub worktree: FileState,
    pub kind: EntryKind,
    /// The file has unresolved merge conflicts. A flag on the entry rather
    /// than one opaque error for the whole repository, so a conflicted merge
    /// still reports every other file correctly.
    pub conflicted: bool,
    /// Where a renamed file came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub renamed_from: Option<WorkspacePath>,
}

/// A repository as `git.status` sees it: one bounded page of changed files
/// plus the facts that describe the repository as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryStatus {
    /// Short branch name (`main`, not `refs/heads/main`). Absent when HEAD is
    /// detached, and absent on an unborn branch in a repository with no
    /// commits yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branch: Option<String>,
    /// HEAD does not point at a branch.
    pub detached: bool,
    /// Hex object id HEAD resolves to. Absent only on an unborn branch, where
    /// there is genuinely no commit — never backfilled from anything an agent
    /// reported, which is a different and untrusted source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head: Option<String>,
    pub state: RepositoryState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub upstream: Option<UpstreamStatus>,
    /// This page of changed files, sorted by path.
    pub entries: Vec<FileStatus>,
    /// How many changed files there were in total, across all pages.
    pub total: u64,
    /// More entries follow; pass `cursor` back to continue.
    pub truncated: bool,
    /// Opaque continuation token, present when `truncated`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cursor: Option<String>,
    /// Entries dropped because their path cannot be expressed as a
    /// [`WorkspacePath`] — in practice a filename containing a backslash,
    /// which is unaddressable on Windows and therefore unaddressable in the
    /// protocol (ADR-010).
    ///
    /// Counted rather than silently skipped: a status that quietly hid a file
    /// would be the "partial data shown as complete" the failure table
    /// forbids.
    pub unrepresentable: u32,
}

/// What happened to one file between two revisions, or between a revision and
/// the working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiffChangeKind {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    TypeChanged,
}

/// Which side of the diff a line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

/// One line of one hunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// The line without its trailing newline. Untrusted repository content:
    /// decoded lossily, passed through, never interpreted, never logged.
    pub content: String,
}

/// A contiguous run of changed lines with its surrounding context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    /// 1-based first line on the old side; `0` when the hunk adds to an empty
    /// file, matching unified-diff convention.
    pub old_start: u32,
    pub old_lines: u32,
    /// 1-based first line on the new side.
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
    /// The hunk had more lines than [`DiffLimits::max_hunk_lines`] and `lines`
    /// stops short. Said explicitly rather than discovered by the reader,
    /// the way `OutputBatch.truncated_bytes` says it for terminal output.
    pub truncated: bool,
}

/// One file's diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: WorkspacePath,
    /// Where a renamed or copied file came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub old_path: Option<WorkspacePath>,
    pub change: DiffChangeKind,
    /// Lines added, over the whole file — not only the hunks in this response.
    pub additions: u32,
    /// Lines removed, over the whole file.
    pub deletions: u32,
    /// Either side is binary. Then `hunks` is empty and `size` is all Gritty
    /// says about it: binary content never crosses the wire
    /// (`security.md`).
    pub binary: bool,
    /// Size in bytes of the new side (the old side, for a deletion), when it
    /// is known. Only meaningful with `binary`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub size: Option<u64>,
    pub hunks: Vec<DiffHunk>,
    /// The file had more hunks than [`DiffLimits::max_hunks`] and `hunks`
    /// stops short.
    pub hunks_truncated: bool,
}

/// What `git.diff` returns: one bounded page of files, each with bounded
/// hunks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiffSummary {
    /// The revision the diff was taken against, as the caller wrote it
    /// (`HEAD` when they wrote nothing).
    pub target: String,
    /// The object id `target` resolved to. Absent when the repository has no
    /// commits yet, in which case everything tracked reads as an addition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub resolved: Option<String>,
    /// This page of files, sorted by path.
    pub files: Vec<FileDiff>,
    /// How many files differed in total, across all pages.
    pub total: u64,
    /// More files follow; pass `cursor` back to continue.
    pub truncated: bool,
    /// Opaque continuation token, present when `truncated`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cursor: Option<String>,
    /// Files dropped because their path is not expressible as a
    /// [`WorkspacePath`]; see [`RepositoryStatus::unrepresentable`].
    pub unrepresentable: u32,
}

/// The bounds `git.diff` applies so one response cannot be unbounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffLimits {
    /// Hunks kept per file.
    pub max_hunks: usize,
    /// Lines kept per hunk.
    pub max_hunk_lines: usize,
    /// Lines of unchanged context on each side of a change.
    pub context_lines: u32,
    /// Files above this many bytes are reported with counts but no hunks, the
    /// way a binary file is. Reading a 200 MB text file into lines to render
    /// a diff nobody will scroll is work with no reader.
    pub max_blob_bytes: u64,
}

impl Default for DiffLimits {
    fn default() -> Self {
        Self {
            max_hunks: 200,
            max_hunk_lines: 1_000,
            context_lines: 3,
            max_blob_bytes: 2 * 1024 * 1024,
        }
    }
}
