//! `git.status`: what the repository looks like right now.
//!
//! Three sources are merged into one per-path answer, which is why this is
//! more than a translation layer:
//!
//! - **HEAD → index** (`gix::status::Item::TreeIndex`) fills [`FileStatus::index`].
//! - **index → worktree** (`gix::status::Item::IndexWorktree`) fills
//!   [`FileStatus::worktree`], including conflicts and type changes.
//! - **the directory walk**, part of the same stream, contributes untracked
//!   (and, on request, ignored) entries.
//!
//! With the `parallel` feature those three run concurrently and arrive in an
//! undefined order, so a path can be reported twice — staged *and* then
//! modified again in the worktree is the everyday case. Entries are therefore
//! merged by path into one [`FileStatus`] with two independent sides, which is
//! also the shape `git status --porcelain` has for the same reason.
//!
//! Nothing here writes. `gix`'s status can offer to write back a refreshed
//! index (`Outcome::write_changes`), which would make the *next* status
//! cheaper; the offer is declined, because T100 is read-only and a read that
//! quietly rewrites `.git/index` is not one.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gritty_workspace::WorkspacePath;

use crate::error::GitError;
use crate::model::{
    EntryKind, FileState, FileStatus, RepositoryState, RepositoryStatus, UpstreamStatus,
};
use crate::open::{Repository, upstream_ref_name};
use crate::page::{self, PageOptions};
use crate::path;

/// What `git.status` should report.
#[derive(Debug, Clone, Default)]
pub struct StatusOptions {
    pub page: PageOptions,
    /// Include files `.gitignore` excludes. Off by default: the ignored set is
    /// `target/` and `node_modules/`, which is unbounded work for a question
    /// nobody asked.
    pub include_ignored: bool,
}

/// Read the repository's status.
///
/// `interrupt` is both the cancellation predicate for this crate's own loops
/// and the flag handed to `gix`'s status platform, so setting it stops the
/// producer threads as well as the consumer. Cancellation returns
/// [`GitError::Cancelled`] and nothing partial.
pub fn status(
    repo: &Repository,
    options: &StatusOptions,
    interrupt: &Arc<AtomicBool>,
) -> Result<RepositoryStatus, GitError> {
    let local = repo.to_thread_local();
    let cancelled = || interrupt.load(Ordering::Relaxed);

    let head = read_head(&local)?;
    let state = repository_state(&local);
    let upstream = match head.branch.as_deref() {
        Some(branch) => upstream_status(&local, branch, head.id.as_deref(), &cancelled)?,
        None => None,
    };

    let (entries, unrepresentable) = if local.is_bare() {
        (BTreeMap::new(), 0)
    } else {
        collect_entries(&local, options, interrupt, &cancelled)?
    };
    let page = page::page_collected(
        entries.into_values().collect(),
        &options.page,
        |entry: &FileStatus| entry.path.to_string(),
    );

    // Counts and durations only. A branch name, a path and a commit message are
    // all repository content, and `AGENTS.md` rule 13 keeps them out of logs.
    tracing::debug!(
        entries = page.entries.len(),
        total = page.total,
        truncated = page.truncated,
        unrepresentable,
        detached = head.detached,
        "git status read"
    );

    Ok(RepositoryStatus {
        branch: head.branch,
        detached: head.detached,
        head: head.id,
        state,
        upstream,
        entries: page.entries,
        total: page.total,
        truncated: page.truncated,
        cursor: page.cursor,
        unrepresentable,
    })
}

struct Head {
    branch: Option<String>,
    detached: bool,
    id: Option<String>,
}

fn read_head(repo: &gix::Repository) -> Result<Head, GitError> {
    let head = repo
        .head()
        .map_err(|error| GitError::repository("could not read HEAD", error))?;
    let (branch, detached) = match &head.kind {
        gix::head::Kind::Symbolic(reference) => (
            Some(String::from_utf8_lossy(reference.name.shorten()).into_owned()),
            false,
        ),
        // An unborn branch is a branch: the user is on `main`, there is simply
        // no commit yet. Reporting it as detached would be a different and
        // wrong claim.
        gix::head::Kind::Unborn(name) => (
            Some(String::from_utf8_lossy(name.shorten()).into_owned()),
            false,
        ),
        gix::head::Kind::Detached { .. } => (None, true),
    };
    let id = head.id().map(|id| id.to_string());
    Ok(Head {
        branch,
        detached,
        id,
    })
}

fn repository_state(repo: &gix::Repository) -> RepositoryState {
    if repo.is_bare() {
        return RepositoryState::Bare;
    }
    match repo.state() {
        None => pending_sequence(repo.path()).unwrap_or(RepositoryState::Clean),
        Some(state) => match state {
            gix::state::InProgress::Merge => RepositoryState::Merging,
            // `ApplyMailboxRebase` is a `.git/rebase-apply` with neither an
            // `applying` nor a `rebasing` marker. Git's own reader
            // (`wt_status_check_rebase`) calls that a rebase, since only
            // `applying` makes it `git am`; so does this.
            gix::state::InProgress::Rebase
            | gix::state::InProgress::RebaseInteractive
            | gix::state::InProgress::ApplyMailboxRebase => RepositoryState::Rebasing,
            gix::state::InProgress::CherryPick | gix::state::InProgress::CherryPickSequence => {
                RepositoryState::CherryPicking
            }
            gix::state::InProgress::Revert | gix::state::InProgress::RevertSequence => {
                RepositoryState::Reverting
            }
            gix::state::InProgress::Bisect => RepositoryState::Bisecting,
            gix::state::InProgress::ApplyMailbox => RepositoryState::ApplyingMailbox,
        },
    }
}

/// Bytes of `.git/sequencer/todo` read to find its first command.
const TODO_HEAD_BYTES: u64 = 4096;

/// A cherry-pick or revert sequence that still has commits to apply after the
/// one it stopped on was concluded with a plain `git commit`.
///
/// That commit removes `CHERRY_PICK_HEAD`/`REVERT_HEAD`, which is all `gix`'s
/// `state()` looks for, and leaves `.git/sequencer/todo` behind — so `gix`
/// reports nothing in progress while `git status` says "Cherry-pick currently
/// in progress" and the user still has to `--continue` or `--quit`. Git decides
/// from the todo list's first command (`sequencer_get_last_command`), and so
/// does this. Only the head of the file is read: it is repository content
/// (`AGENTS.md` rule 13), and the first command is all the answer needs.
fn pending_sequence(git_dir: &std::path::Path) -> Option<RepositoryState> {
    use std::io::Read;

    let file = std::fs::File::open(git_dir.join("sequencer").join("todo")).ok()?;
    let mut head = Vec::new();
    file.take(TODO_HEAD_BYTES).read_to_end(&mut head).ok()?;
    let text = head.trim_ascii_start();
    let command_end = text.iter().position(|b| *b == b' ' || *b == b'\t')?;
    match &text[..command_end] {
        b"pick" | b"p" => Some(RepositoryState::CherryPicking),
        b"revert" => Some(RepositoryState::Reverting),
        _ => None,
    }
}

/// Ahead/behind against the local copy of the tracking ref. Nothing is
/// fetched; see [`crate::model::UpstreamStatus`].
fn upstream_status(
    repo: &gix::Repository,
    branch: &str,
    head: Option<&str>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<UpstreamStatus>, GitError> {
    let Some(ref_name) = upstream_ref_name(repo, branch) else {
        return Ok(None);
    };
    // A configured upstream whose ref is not present locally (never fetched,
    // or pruned) is "no upstream" rather than an error: there is nothing to
    // count against.
    let Ok(mut reference) = repo.find_reference(ref_name.as_str()) else {
        return Ok(None);
    };
    let Ok(upstream_id) = reference.peel_to_id() else {
        return Ok(None);
    };
    let upstream_id = upstream_id.detach();
    let Some(head) = head else {
        return Ok(None);
    };
    let Ok(head_id) = gix::ObjectId::from_hex(head.as_bytes()) else {
        return Ok(None);
    };
    if head_id == upstream_id {
        return Ok(Some(UpstreamStatus {
            ref_name,
            ahead: 0,
            behind: 0,
        }));
    }
    let ahead = count_commits(repo, head_id, upstream_id, cancelled)?;
    let behind = count_commits(repo, upstream_id, head_id, cancelled)?;
    Ok(Some(UpstreamStatus {
        ref_name,
        ahead,
        behind,
    }))
}

/// Commits reachable from `tip` but not from `hidden`.
///
/// The walk is drained through [`page::drain`] so a divergence of a hundred
/// thousand commits is as cancellable as a large worktree.
fn count_commits(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    hidden: gix::ObjectId,
    cancelled: &dyn Fn() -> bool,
) -> Result<u32, GitError> {
    let walk = match repo.rev_walk([tip]).with_hidden([hidden]).all() {
        Ok(walk) => walk,
        // A shallow clone or a missing object makes the count unanswerable.
        // Reporting `0` would be a claim; reporting the error would fail a
        // whole status over one number, so the count is skipped.
        Err(_) => return Ok(0),
    };
    let mut count = 0u32;
    page::drain(walk, cancelled, "could not walk commits", |_info| {
        count = count.saturating_add(1);
        Ok(())
    })?;
    Ok(count)
}

type Entries = BTreeMap<WorkspacePath, FileStatus>;

fn collect_entries(
    repo: &gix::Repository,
    options: &StatusOptions,
    interrupt: &Arc<AtomicBool>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Entries, u32), GitError> {
    let mut platform = repo
        .status(gix::progress::Discard)
        .map_err(|error| GitError::repository("could not prepare the status", error))?
        .should_interrupt_owned(Arc::clone(interrupt))
        // A submodule is reported as an entry, never entered: T100's failure
        // table, and the reason there is no recursion budget to blow.
        .index_worktree_submodules(gix::status::Submodule::Given {
            ignore: gix::submodule::config::Ignore::All,
            check_dirty: false,
        });
    if options.include_ignored {
        platform = platform.dirwalk_options(|walk| {
            walk.emit_ignored(Some(gix::dir::walk::EmissionMode::CollapseDirectory))
        });
    }
    let iter = platform
        .into_iter(None)
        .map_err(|error| GitError::repository("could not start the status", error))?;

    let mut entries: Entries = BTreeMap::new();
    let mut unrepresentable = 0u32;
    page::drain(iter, cancelled, "could not read the status", |item| {
        match item {
            gix::status::Item::TreeIndex(change) => {
                apply_tree_index(&mut entries, &mut unrepresentable, &change);
            }
            gix::status::Item::IndexWorktree(item) => {
                apply_index_worktree(&mut entries, &mut unrepresentable, &item);
            }
        }
        Ok(())
    })?;
    Ok((entries, unrepresentable))
}

/// Look up (or create) the entry for `raw`, or count it as unrepresentable.
fn entry_for<'a>(
    entries: &'a mut Entries,
    unrepresentable: &mut u32,
    raw: &[u8],
) -> Option<&'a mut FileStatus> {
    let Some(path) = path::decode(raw) else {
        *unrepresentable = unrepresentable.saturating_add(1);
        return None;
    };
    Some(entries.entry(path.clone()).or_insert_with(|| FileStatus {
        path,
        index: FileState::Unmodified,
        worktree: FileState::Unmodified,
        kind: EntryKind::File,
        conflicted: false,
        renamed_from: None,
    }))
}

fn apply_tree_index(
    entries: &mut Entries,
    unrepresentable: &mut u32,
    change: &gix::diff::index::Change,
) {
    use gix::diff::index::Change;

    let (location, state, source) = match change {
        Change::Addition { location, .. } => (location, FileState::Added, None),
        Change::Deletion { location, .. } => (location, FileState::Deleted, None),
        Change::Modification {
            location,
            previous_entry_mode,
            entry_mode,
            ..
        } => {
            let state = if kind_of_mode(*previous_entry_mode) == kind_of_mode(*entry_mode) {
                FileState::Modified
            } else {
                FileState::TypeChanged
            };
            (location, state, None)
        }
        Change::Rewrite {
            location,
            source_location,
            copy,
            ..
        } => (
            location,
            if *copy {
                // A copy leaves the source in place, so it is an addition with
                // a known origin rather than a rename.
                FileState::Added
            } else {
                FileState::Renamed
            },
            Some(source_location),
        ),
    };
    let mode = entry_mode_of(change);
    let Some(entry) = entry_for(entries, unrepresentable, location.as_ref()) else {
        return;
    };
    entry.index = state;
    if let Some(kind) = mode {
        entry.kind = kind;
    }
    if let Some(source) = source {
        entry.renamed_from = path::decode(source.as_ref());
    }
}

fn entry_mode_of(change: &gix::diff::index::Change) -> Option<EntryKind> {
    use gix::diff::index::Change;
    let mode = match change {
        Change::Addition { entry_mode, .. }
        | Change::Deletion { entry_mode, .. }
        | Change::Modification { entry_mode, .. }
        | Change::Rewrite { entry_mode, .. } => entry_mode,
    };
    Some(kind_of_mode(*mode))
}

fn kind_of_mode(mode: gix::index::entry::Mode) -> EntryKind {
    use gix::index::entry::Mode;
    if mode.contains(Mode::COMMIT) {
        EntryKind::Submodule
    } else if mode.contains(Mode::SYMLINK) {
        EntryKind::Symlink
    } else if mode.contains(Mode::DIR) {
        EntryKind::Directory
    } else {
        EntryKind::File
    }
}

fn apply_index_worktree(
    entries: &mut Entries,
    unrepresentable: &mut u32,
    item: &gix::status::index_worktree::Item,
) {
    use gix::status::index_worktree::{Item, RewriteSource};
    use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};

    match item {
        Item::Modification {
            rela_path,
            status,
            entry,
            ..
        } => {
            let kind = kind_of_mode(entry.mode);
            let Some(target) = entry_for(entries, unrepresentable, rela_path.as_ref()) else {
                return;
            };
            target.kind = kind;
            match status {
                // A conflict is a flag on the file, not one opaque error for
                // the repository: every other file still reports correctly.
                EntryStatus::Conflict { .. } => {
                    target.conflicted = true;
                    target.worktree = FileState::Modified;
                }
                EntryStatus::Change(Change::Removed) => target.worktree = FileState::Deleted,
                EntryStatus::Change(Change::Type { .. }) => {
                    target.worktree = FileState::TypeChanged;
                }
                EntryStatus::Change(Change::Modification { .. })
                | EntryStatus::Change(Change::SubmoduleModification(_)) => {
                    target.worktree = FileState::Modified;
                }
                // `git add --intent-to-add`: the index promises a file that has
                // no content yet, so the content is entirely a worktree
                // addition.
                EntryStatus::IntentToAdd => target.worktree = FileState::Added,
                // Not a change at all — only a stat refresh `gix` would like to
                // write back, which this read declines to do.
                EntryStatus::NeedsUpdate(_) => {}
            }
        }
        Item::DirectoryContents { entry, .. } => {
            let state = match entry.status {
                gix::dir::entry::Status::Untracked => FileState::Untracked,
                gix::dir::entry::Status::Ignored(_) => FileState::Ignored,
                // Tracked and pruned entries reach here only as noise from the
                // walk; the tracked side of the status already covers them.
                gix::dir::entry::Status::Tracked | gix::dir::entry::Status::Pruned => return,
            };
            let kind = match entry.disk_kind {
                Some(gix::dir::entry::Kind::Directory) => EntryKind::Directory,
                Some(gix::dir::entry::Kind::Symlink) => EntryKind::Symlink,
                Some(gix::dir::entry::Kind::Repository) => EntryKind::Submodule,
                _ => EntryKind::File,
            };
            let Some(target) = entry_for(entries, unrepresentable, entry.rela_path.as_ref()) else {
                return;
            };
            target.worktree = state;
            target.kind = kind;
        }
        Item::Rewrite {
            source,
            dirwalk_entry,
            ..
        } => {
            let source_path = match source {
                RewriteSource::RewriteFromIndex {
                    source_rela_path, ..
                } => path::decode(source_rela_path.as_ref()),
                RewriteSource::CopyFromDirectoryEntry {
                    source_dirwalk_entry,
                    ..
                } => path::decode(source_dirwalk_entry.rela_path.as_ref()),
            };
            let Some(target) =
                entry_for(entries, unrepresentable, dirwalk_entry.rela_path.as_ref())
            else {
                return;
            };
            target.worktree = FileState::Renamed;
            target.renamed_from = source_path;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::fixtures::{self, TestRepo};
    use crate::open::open;

    fn read(repo: &TestRepo) -> RepositoryStatus {
        read_with(repo, &StatusOptions::default())
    }

    fn read_with(repo: &TestRepo, options: &StatusOptions) -> RepositoryStatus {
        let opened = open(repo.path()).expect("open");
        status(&opened, options, &Arc::new(AtomicBool::new(false))).expect("status")
    }

    fn entry<'a>(status: &'a RepositoryStatus, path: &str) -> &'a FileStatus {
        status
            .entries
            .iter()
            .find(|entry| entry.path.to_string() == path)
            .unwrap_or_else(|| {
                panic!(
                    "no entry for {path}; had {:?}",
                    status
                        .entries
                        .iter()
                        .map(|e| e.path.to_string())
                        .collect::<Vec<_>>()
                )
            })
    }

    #[test]
    fn a_clean_repository_reports_its_branch_and_head_and_no_entries() {
        let repo = fixtures::clean();
        let head = repo.head_id();
        let status = read(&repo);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert!(!status.detached);
        assert_eq!(status.head.as_deref(), Some(head.as_str()));
        assert_eq!(status.state, RepositoryState::Clean);
        assert_eq!(status.upstream, None);
        assert!(status.entries.is_empty(), "{:?}", status.entries);
        assert_eq!(status.total, 0);
        assert_eq!(status.unrepresentable, 0);
    }

    #[test]
    fn reports_staged_unstaged_untracked_and_deleted_files_separately() {
        let repo = fixtures::clean();
        repo.write("staged.txt", "one\n");
        repo.git(&["add", "staged.txt"]);
        repo.write("README.md", "changed\n");
        repo.write("untracked.txt", "new\n");
        repo.remove("src/main.rs");

        let status = read(&repo);
        assert_eq!(entry(&status, "staged.txt").index, FileState::Added);
        assert_eq!(entry(&status, "staged.txt").worktree, FileState::Unmodified);
        assert_eq!(entry(&status, "README.md").index, FileState::Unmodified);
        assert_eq!(entry(&status, "README.md").worktree, FileState::Modified);
        assert_eq!(
            entry(&status, "untracked.txt").worktree,
            FileState::Untracked
        );
        assert_eq!(entry(&status, "src/main.rs").worktree, FileState::Deleted);
    }

    #[test]
    fn a_staged_rename_carries_its_source() {
        let repo = fixtures::clean();
        repo.git(&["mv", "README.md", "READ.md"]);
        let status = read(&repo);
        let renamed = entry(&status, "READ.md");
        assert_eq!(renamed.index, FileState::Renamed);
        assert_eq!(
            renamed.renamed_from.as_ref().map(ToString::to_string),
            Some("README.md".to_owned())
        );
    }

    #[test]
    fn ignored_files_are_absent_unless_asked_for() {
        let repo = fixtures::clean();
        repo.write(".gitignore", "ignored.txt\n");
        repo.commit_all("ignore");
        repo.write("ignored.txt", "hidden\n");

        let plain = read(&repo);
        assert!(
            !plain
                .entries
                .iter()
                .any(|entry| entry.path.to_string() == "ignored.txt"),
            "ignored files must not appear by default"
        );

        let with_ignored = read_with(
            &repo,
            &StatusOptions {
                include_ignored: true,
                ..Default::default()
            },
        );
        assert_eq!(
            entry(&with_ignored, "ignored.txt").worktree,
            FileState::Ignored
        );
    }

    #[test]
    fn a_merge_conflict_flags_the_file_and_names_the_state() {
        let repo = fixtures::merge_conflict();
        let status = read(&repo);
        assert_eq!(status.state, RepositoryState::Merging);
        assert!(
            entry(&status, "conflict.txt").conflicted,
            "the conflicted file must be flagged"
        );
        // Every other fact about the repository still reports.
        assert_eq!(status.branch.as_deref(), Some("main"));
    }

    /// Covers `gix`'s `RebaseInteractive`: the default merge backend marks even
    /// a plain `git rebase` interactive.
    #[test]
    fn a_rebase_stopped_on_a_conflict_is_rebasing() {
        let repo = fixtures::rebase_conflict();
        let status = read(&repo);
        assert_eq!(status.state, RepositoryState::Rebasing);
        assert!(entry(&status, "conflict.txt").conflicted);
        // A rebase runs on a detached HEAD.
        assert!(status.detached);
    }

    /// Covers `gix`'s `Rebase`, which current Git only produces through the
    /// apply backend's `rebase-apply/rebasing`.
    #[test]
    fn a_rebase_on_the_apply_backend_is_rebasing_and_not_a_mailbox() {
        let repo = fixtures::rebase_apply_conflict();
        let status = read(&repo);
        assert_eq!(status.state, RepositoryState::Rebasing);
        assert!(entry(&status, "conflict.txt").conflicted);
    }

    /// Covers `gix`'s `ApplyMailboxRebase`: a `rebase-apply` with neither
    /// marker. No current Git leaves one behind, so the marker is removed from
    /// a real apply-backend rebase; Git's own status reads what remains as a
    /// rebase, and this used to report it as `git am`.
    #[test]
    fn a_rebase_apply_directory_without_markers_is_rebasing() {
        let repo = fixtures::rebase_apply_conflict();
        std::fs::remove_file(repo.path().join(".git/rebase-apply/rebasing"))
            .expect("remove marker");
        assert_eq!(read(&repo).state, RepositoryState::Rebasing);
    }

    #[test]
    fn an_interactive_rebase_stopped_on_edit_is_rebasing_with_a_clean_worktree() {
        let repo = fixtures::interactive_rebase_edit();
        let status = read(&repo);
        assert_eq!(status.state, RepositoryState::Rebasing);
        assert!(status.entries.is_empty(), "{:?}", status.entries);
    }

    /// Covers `gix`'s `CherryPick`: one commit, no sequencer.
    #[test]
    fn a_cherry_pick_stopped_on_a_conflict_is_cherry_picking() {
        let repo = fixtures::cherry_pick_conflict();
        let status = read(&repo);
        assert_eq!(status.state, RepositoryState::CherryPicking);
        assert!(entry(&status, "conflict.txt").conflicted);
        assert_eq!(status.branch.as_deref(), Some("main"));
    }

    /// Covers `gix`'s `CherryPickSequence`.
    #[test]
    fn a_cherry_pick_sequence_stopped_on_a_conflict_is_cherry_picking() {
        let repo = fixtures::cherry_pick_sequence_conflict();
        let status = read(&repo);
        assert_eq!(status.state, RepositoryState::CherryPicking);
        assert!(entry(&status, "conflict.txt").conflicted);
    }

    /// `gix` reports nothing in progress here; `git status` says "Cherry-pick
    /// currently in progress", and the second pick is still waiting.
    #[test]
    fn a_cherry_pick_sequence_committed_mid_way_is_still_cherry_picking() {
        let repo = fixtures::sequence_committed_mid_way(fixtures::cherry_pick_sequence_conflict());
        let status = read(&repo);
        assert_eq!(status.state, RepositoryState::CherryPicking);
        assert!(status.entries.is_empty(), "{:?}", status.entries);
    }

    /// Covers `gix`'s `Revert`: one commit, no sequencer.
    #[test]
    fn a_revert_stopped_on_a_conflict_is_reverting() {
        let repo = fixtures::revert_conflict();
        let status = read(&repo);
        assert_eq!(status.state, RepositoryState::Reverting);
        assert!(entry(&status, "conflict.txt").conflicted);
    }

    /// Covers `gix`'s `RevertSequence`.
    #[test]
    fn a_revert_sequence_stopped_on_a_conflict_is_reverting() {
        let repo = fixtures::revert_sequence_conflict();
        let status = read(&repo);
        assert_eq!(status.state, RepositoryState::Reverting);
        assert!(entry(&status, "conflict.txt").conflicted);
    }

    /// The revert twin of the committed cherry-pick sequence: `git status`
    /// says "Revert currently in progress".
    #[test]
    fn a_revert_sequence_committed_mid_way_is_still_reverting() {
        let repo = fixtures::sequence_committed_mid_way(fixtures::revert_sequence_conflict());
        assert_eq!(read(&repo).state, RepositoryState::Reverting);
    }

    #[test]
    fn a_bisect_in_progress_is_bisecting_on_a_detached_midpoint() {
        let repo = fixtures::bisecting();
        let status = read(&repo);
        assert_eq!(status.state, RepositoryState::Bisecting);
        assert!(status.detached);
        assert_eq!(status.branch, None);
    }

    /// Covers `gix`'s `ApplyMailbox`: `rebase-apply/applying`.
    #[test]
    fn a_mailbox_stopped_on_a_conflict_is_applying_a_mailbox() {
        let repo = fixtures::am_conflict();
        let status = read(&repo);
        assert_eq!(status.state, RepositoryState::ApplyingMailbox);
        assert!(entry(&status, "conflict.txt").conflicted);
    }

    #[test]
    fn a_sequencer_todo_names_the_operation_by_its_first_command_only() {
        let dir = TestRepo::plain_directory();
        let todo = |contents: &str| {
            dir.write("sequencer/todo", contents);
            pending_sequence(dir.path())
        };
        assert_eq!(
            todo("pick 983c0d7 side\n"),
            Some(RepositoryState::CherryPicking)
        );
        assert_eq!(
            todo("p 983c0d7 side\n"),
            Some(RepositoryState::CherryPicking)
        );
        assert_eq!(
            todo("\n  revert 4e18036 main\npick 1 x\n"),
            Some(RepositoryState::Reverting)
        );
        // Anything else is not a sequence `git status` reports.
        assert_eq!(todo("exec make\npick 1 x\n"), None);
        assert_eq!(todo("pickaxe 1 x\n"), None);
        assert_eq!(todo("pick"), None);
        assert_eq!(todo(""), None);
        std::fs::remove_file(dir.path().join("sequencer/todo")).expect("remove todo");
        assert_eq!(pending_sequence(dir.path()), None);
    }

    #[test]
    fn a_detached_head_has_no_branch_and_keeps_its_commit_id() {
        let repo = fixtures::clean();
        let head = repo.head_id();
        repo.git(&["checkout", "--quiet", "--detach", "HEAD"]);
        let status = read(&repo);
        assert!(status.detached);
        assert_eq!(status.branch, None);
        assert_eq!(status.head.as_deref(), Some(head.as_str()));
    }

    #[test]
    fn a_bare_repository_is_labelled_and_has_no_worktree_entries() {
        let repo = TestRepo::init_bare();
        let status = read(&repo);
        assert_eq!(status.state, RepositoryState::Bare);
        assert!(status.entries.is_empty());
        assert_eq!(status.total, 0);
    }

    #[test]
    fn a_repository_with_no_commits_has_a_branch_but_no_head() {
        let repo = TestRepo::init();
        repo.write("first.txt", "hello\n");
        let status = read(&repo);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert!(!status.detached);
        assert_eq!(status.head, None);
        assert_eq!(entry(&status, "first.txt").worktree, FileState::Untracked);
    }

    #[test]
    fn ahead_and_behind_are_counted_against_the_local_tracking_ref() {
        let repo = fixtures::clean();
        // A tracking ref that exists only locally: nothing is fetched.
        repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        repo.git(&["config", "branch.main.remote", "origin"]);
        repo.git(&["config", "branch.main.merge", "refs/heads/main"]);

        let level = read(&repo);
        let upstream = level.upstream.expect("upstream");
        assert_eq!(upstream.ref_name, "refs/remotes/origin/main");
        assert_eq!((upstream.ahead, upstream.behind), (0, 0));

        repo.write("ahead.txt", "a\n");
        repo.commit_all("one ahead");
        repo.write("ahead2.txt", "b\n");
        repo.commit_all("two ahead");

        let ahead = read(&repo).upstream.expect("upstream");
        assert_eq!((ahead.ahead, ahead.behind), (2, 0));
    }

    #[test]
    fn a_submodule_is_reported_as_an_entry_and_not_entered() {
        let repo = fixtures::clean();
        let inner = fixtures::clean();
        repo.git(&[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "--quiet",
            "add",
            inner.path().to_str().expect("utf-8 path"),
            "sub",
        ]);
        let status = read(&repo);
        let entry = entry(&status, "sub");
        assert_eq!(entry.kind, EntryKind::Submodule);
        // Nothing inside the submodule is listed.
        assert!(
            !status
                .entries
                .iter()
                .any(|entry| entry.path.to_string().starts_with("sub/")),
            "the submodule must not be entered"
        );
    }

    #[test]
    fn pages_entries_and_continues_from_the_cursor() {
        let repo = fixtures::clean();
        for index in 0..5 {
            repo.write(&format!("p{index}.txt"), "x\n");
        }
        let first = read_with(
            &repo,
            &StatusOptions {
                page: PageOptions {
                    page_size: 2,
                    cursor: None,
                },
                ..Default::default()
            },
        );
        assert_eq!(first.entries.len(), 2);
        assert!(first.truncated);
        assert_eq!(first.total, 5);

        let second = read_with(
            &repo,
            &StatusOptions {
                page: PageOptions {
                    page_size: 10,
                    cursor: first.cursor.clone(),
                },
                ..Default::default()
            },
        );
        assert_eq!(second.entries.len(), 3);
        assert!(!second.truncated);
        // No entry appears in both pages.
        for entry in &second.entries {
            assert!(!first.entries.iter().any(|first| first.path == entry.path));
        }
    }

    #[test]
    fn a_cancelled_status_returns_cancelled_and_nothing_partial() {
        let repo = fixtures::clean();
        repo.write("changed.txt", "x\n");
        let opened = open(repo.path()).expect("open");
        let interrupt = Arc::new(AtomicBool::new(true));
        let error = status(&opened, &StatusOptions::default(), &interrupt).expect_err("cancelled");
        assert!(matches!(error, GitError::Cancelled));
    }

    /// T100 §6: cancelling a status on a repository large enough to threaten
    /// the budget stops the work, and stops it well before the read would have
    /// finished on its own.
    ///
    /// The measurement is deliberately relative rather than absolute. The
    /// *bound* — at most one check interval of further work after the flag
    /// flips — is proven exactly by `page::tests::
    /// cancellation_is_noticed_within_one_check_interval`, on a synthetic
    /// sequence where the consumed count is observable. What this test adds is
    /// the part that unit test cannot reach: that the flag actually travels
    /// into `gix`'s producer threads, so a real, expensive status is abandoned
    /// rather than run to completion and then discarded.
    #[test]
    fn cancelling_a_large_status_abandons_the_work_rather_than_finishing_it() {
        const FILES: usize = 3_000;
        let repo = fixtures::many_changes(FILES);
        let opened = open(repo.path()).expect("open");

        // What the whole read costs, measured on this machine, now.
        let started = std::time::Instant::now();
        let full = status(
            &opened,
            &StatusOptions::default(),
            &Arc::new(AtomicBool::new(false)),
        )
        .expect("status");
        let baseline = started.elapsed();
        assert_eq!(
            full.total, FILES as u64,
            "the fixture must produce a genuinely large status"
        );

        // Cancel a tenth of the way in. The second read is at worst as slow as
        // the first (its caches are warmer, not colder), so this lands while
        // the status is still running.
        let interrupt = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&interrupt);
        let flipper = std::thread::spawn(move || {
            std::thread::sleep(baseline / 10);
            flag.store(true, Ordering::SeqCst);
        });
        let started = std::time::Instant::now();
        let result = status(&opened, &StatusOptions::default(), &interrupt);
        let cancelled_after = started.elapsed();
        flipper.join().expect("flipper thread");

        assert!(
            matches!(result, Err(GitError::Cancelled)),
            "a cancelled status must return Cancelled and nothing partial, got {:?}",
            result.map(|status| status.total)
        );
        assert!(
            cancelled_after < baseline,
            "cancelling took {cancelled_after:?}, no less than the {baseline:?} the \
             whole read costs: the flag never reached the work"
        );
    }
}
