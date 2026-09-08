//! `files.*` — changing the tree (T042, ADR-028).
//!
//! The filesystem work is `gritty_workspace::mutate`; this module owns the
//! commands, the undo journal, the budget hand-over for work that can run long,
//! and the two things the document layer has to be told.
//!
//! Three rules shape it:
//!
//! - **Every response names the directories to re-list.** The T070 watch will
//!   report the same change 200 ms–1 s later, but it is best-effort — a backend
//!   that refuses to start still opens the workspace — so convergence cannot
//!   depend on it. Nothing here emits a `CoreEvent`; the response is the signal.
//! - **A rename re-keys open documents; a delete does not refuse them.** Moving
//!   a file with an unsaved buffer is safe, because `rehome` carries the buffer
//!   to the new path. Deleting one is not, so a delete is `Conflict` with
//!   `details.dirtyDocuments` unless it is forced (hard rule 7).
//! - **Nothing is atomic across entries.** A copy or delete over many paths that
//!   fails or is cancelled part-way says what it did and leaves it there.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use gritty_workspace::{CopyOutcome, EntryKind, WorkspacePath, WorkspaceRoot, mutate};
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use super::WorkspaceManager;
use super::journal::{Journal, JournalEntry, Step};
use crate::operation::{Operation, OperationKind, OperationRegistry, OperationSpec};
use crate::protocol::actor::Actor;
use crate::protocol::error::CoreError;
use crate::protocol::ids::{CorrelationId, WorkspaceId};

/// Operation kind of a transfer or delete that outlived its budget.
pub const FILES_KIND: &str = "files.transfer";
/// How long a `files.*` command waits before handing over to an operation.
/// Shorter than a listing's: a listing is one directory, this can be a tree.
pub const DEFAULT_FILES_BUDGET: Duration = Duration::from_millis(100);

/// What `files.create` should make.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NewEntryKind {
    File,
    Directory,
}

impl From<NewEntryKind> for EntryKind {
    fn from(kind: NewEntryKind) -> Self {
        match kind {
            NewEntryKind::File => Self::File,
            NewEntryKind::Directory => Self::Directory,
        }
    }
}

/// Whether `files.move` moves or copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransferMode {
    Move,
    Copy,
}

/// Whether `files.delete` trashes or destroys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeleteMode {
    /// The OS trash. Recoverable outside Gritty; not undoable inside it on
    /// macOS (ADR-028).
    Trash,
    Permanent,
}

/// `files.create`: one entry, with any missing directories above it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreateEntryRequest {
    pub workspace_id: WorkspaceId,
    /// The whole path, so `a/b/c.ts` creates `a` and `a/b` too.
    pub path: WorkspacePath,
    pub kind: NewEntryKind,
}

/// `files.rename`: rename in place. The new name is a single segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RenameEntryRequest {
    pub workspace_id: WorkspaceId,
    pub path: WorkspacePath,
    pub new_name: String,
}

/// `files.move`: move or copy entries into a directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MoveEntriesRequest {
    pub workspace_id: WorkspaceId,
    pub paths: Vec<WorkspacePath>,
    /// Where they land. `""` is the workspace root.
    pub target_dir: WorkspacePath,
    pub mode: TransferMode,
}

/// `files.delete`: remove entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeleteEntriesRequest {
    pub workspace_id: WorkspaceId,
    pub paths: Vec<WorkspacePath>,
    pub mode: DeleteMode,
    /// Delete anyway when an open document under one of the paths is dirty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub force: Option<bool>,
}

/// `files.undo` / `files.redo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UndoRequest {
    pub workspace_id: WorkspaceId,
}

/// Where one entry ended up, so a caller holding it by path can follow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MovedEntry {
    pub from: WorkspacePath,
    pub to: WorkspacePath,
}

/// What every `files.*` command answers with.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationResult {
    /// Directories whose entry set changed. Deduplicated and sorted; an
    /// over-approximation, because re-listing a directory the user never
    /// expanded costs nothing (the explorer only refreshes loaded nodes).
    pub changed: Vec<WorkspacePath>,
    /// Entries that moved, in the order they were asked for.
    pub moved: Vec<MovedEntry>,
    /// What ⌘Z would reverse now, for the menu item's label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub undo: Option<String>,
    /// What ⇧⌘Z would replay now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub redo: Option<String>,
    /// Entries written before a cancellation stopped the work. `None` when the
    /// operation ran to completion. What it counts is on the disk and stays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cancelled_after: Option<u32>,
}

/// What `files.move` and `files.delete` return: the result when the work fit in
/// the budget, otherwise the operation whose `OperationCompleted.result` will
/// carry it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum FileOperationResponse {
    Ready { result: FileOperationResult },
    Pending { operation: Box<Operation> },
}

/// What the document layer answers when the tree changes under it.
///
/// A trait installed as a `Weak` by the composition root, exactly like
/// [`super::WorkspaceCloseGuard`], so this module never depends on `document`
/// and the two managers do not keep each other alive.
pub trait DocumentTreeGuard: Send + Sync {
    /// Dirty open documents at or under any of `paths`, as
    /// `{ documentId, path }`. Empty when none — which is the common case and
    /// the only one that lets a delete through unforced.
    fn dirty_under(
        &self,
        workspace_id: &WorkspaceId,
        paths: &[WorkspacePath],
    ) -> Vec<serde_json::Value>;

    /// A rename or move committed: re-point every open document at or under
    /// `from`, keeping its buffer, versions and dirty state.
    fn rehome(
        &self,
        workspace_id: &WorkspaceId,
        from: &WorkspacePath,
        to: &WorkspacePath,
        actor: &Actor,
        correlation: &CorrelationId,
    );
}

/// The `files.*` commands of the open workspace.
pub struct FileService {
    workspaces: Arc<WorkspaceManager>,
    registry: Arc<OperationRegistry>,
    handle: Handle,
    /// One journal, for the one workspace that can be open. Re-created when the
    /// id changes, which is what drops an undo history with its workspace.
    journal: Mutex<(Option<WorkspaceId>, Journal)>,
    guard: Mutex<Option<Weak<dyn DocumentTreeGuard>>>,
}

impl FileService {
    pub fn new(
        handle: Handle,
        workspaces: Arc<WorkspaceManager>,
        registry: Arc<OperationRegistry>,
    ) -> Arc<Self> {
        Arc::new(Self {
            workspaces,
            registry,
            handle,
            journal: Mutex::new((None, Journal::new())),
            guard: Mutex::new(None),
        })
    }

    /// Install the document layer's answer to "is anything dirty under here"
    /// and "this moved".
    pub fn set_document_guard(&self, guard: Weak<dyn DocumentTreeGuard>) {
        *self.guard.lock().unwrap_or_else(PoisonError::into_inner) = Some(guard);
    }

    fn document_guard(&self) -> Option<Arc<dyn DocumentTreeGuard>> {
        self.guard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade)
    }

    /// The journal of `workspace_id`, resetting it when the open workspace
    /// changed under us.
    fn with_journal<T>(
        &self,
        workspace_id: &WorkspaceId,
        body: impl FnOnce(&mut Journal) -> T,
    ) -> T {
        let mut held = self.journal.lock().unwrap_or_else(PoisonError::into_inner);
        if held.0.as_ref() != Some(workspace_id) {
            *held = (Some(workspace_id.clone()), Journal::new());
        }
        body(&mut held.1)
    }

    fn labels(&self, workspace_id: &WorkspaceId) -> (Option<String>, Option<String>) {
        self.with_journal(workspace_id, |journal| {
            (
                journal.peek_undo().map(|entry| entry.label.clone()),
                journal.peek_redo().map(|entry| entry.label.clone()),
            )
        })
    }

    fn result(
        &self,
        workspace_id: &WorkspaceId,
        changed: BTreeSet<WorkspacePath>,
        moved: Vec<MovedEntry>,
        cancelled_after: Option<u32>,
    ) -> FileOperationResult {
        let (undo, redo) = self.labels(workspace_id);
        FileOperationResult {
            changed: changed.into_iter().collect(),
            moved,
            undo,
            redo,
            cancelled_after,
        }
    }

    /// `files.create`.
    pub fn create(
        &self,
        request: CreateEntryRequest,
        correlation: CorrelationId,
    ) -> Result<FileOperationResult, CoreError> {
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        mutate::create(&root, &request.path, request.kind.into())?;
        self.with_journal(&workspace.id, |journal| {
            journal.record(JournalEntry::create(
                request.path.clone(),
                request.kind.into(),
            ));
        });
        let mut changed = BTreeSet::new();
        mark_changed(&mut changed, &request.path);
        Ok(self.result(&workspace.id, changed, Vec::new(), None))
    }

    /// `files.rename`.
    pub fn rename(
        &self,
        request: RenameEntryRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<FileOperationResult, CoreError> {
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        let parent = request.path.parent().ok_or_else(|| {
            CoreError::invalid_input("path", "the workspace root cannot be renamed")
        })?;
        // `join` accepts a single segment only, so a "name" containing a
        // separator is rejected here rather than quietly becoming a move.
        let destination = parent.join(&request.new_name).map_err(CoreError::from)?;
        mutate::rename(&root, &request.path, &destination)?;
        self.after_move(
            &workspace.id,
            &[(request.path.clone(), destination.clone())],
            &actor,
            &correlation,
        );
        self.with_journal(&workspace.id, |journal| {
            journal.record(JournalEntry::moved(
                vec![(request.path.clone(), destination.clone())],
                format!("Rename {}", request.path),
            ));
        });
        let mut changed = BTreeSet::new();
        mark_changed(&mut changed, &request.path);
        mark_changed(&mut changed, &destination);
        Ok(self.result(
            &workspace.id,
            changed,
            vec![MovedEntry {
                from: request.path,
                to: destination,
            }],
            None,
        ))
    }

    /// Tell the document layer about entries that moved.
    fn after_move(
        &self,
        workspace_id: &WorkspaceId,
        pairs: &[(WorkspacePath, WorkspacePath)],
        actor: &Actor,
        correlation: &CorrelationId,
    ) {
        let Some(guard) = self.document_guard() else {
            return;
        };
        for (from, to) in pairs {
            guard.rehome(workspace_id, from, to, actor, correlation);
        }
    }

    /// `files.move`: move or copy `paths` into `target_dir`.
    ///
    /// Every destination is planned before anything moves, so a collision is
    /// decided up front and the whole batch journals as one undoable step.
    pub async fn transfer(
        self: &Arc<Self>,
        request: MoveEntriesRequest,
        actor: Actor,
        correlation: CorrelationId,
        budget: Duration,
    ) -> Result<FileOperationResponse, CoreError> {
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        if request.paths.is_empty() {
            return Err(CoreError::invalid_input(
                "paths",
                "must name at least one entry",
            ));
        }
        let target = root.resolve(&request.target_dir)?;
        if !target.canonical().is_dir() {
            return Err(CoreError::invalid_input(
                "targetDir",
                format!("{} is not a directory", request.target_dir),
            ));
        }
        let pairs = plan_transfer(&root, &request)?;

        let token = CancellationToken::new();
        let mut job = {
            let root = Arc::clone(&root);
            let planned = pairs.clone();
            let mode = request.mode;
            let token = token.clone();
            self.handle
                .spawn_blocking(move || transfer_blocking(&root, &planned, mode, &token))
        };
        match tokio::time::timeout(budget, &mut job).await {
            Ok(joined) => {
                let progress = finish_job(joined, "transfer")?;
                Ok(FileOperationResponse::Ready {
                    result: self.settle_transfer(
                        &workspace.id,
                        &pairs,
                        progress,
                        request.mode,
                        &actor,
                        &correlation,
                    ),
                })
            }
            Err(_elapsed) => {
                tracing::debug!(
                    workspace = %workspace.id,
                    entries = pairs.len(),
                    budget_ms = budget.as_millis() as u64,
                    "transfer exceeded its budget; continuing as an operation"
                );
                let spec = OperationSpec {
                    kind: OperationKind::new(FILES_KIND),
                    actor: actor.clone(),
                    workspace_id: Some(workspace.id.clone()),
                    correlation_id: correlation.clone(),
                    durable: false,
                };
                let me = Arc::clone(self);
                let workspace_id = workspace.id.clone();
                let mode = request.mode;
                let operation = self.registry.start(spec, move |ctx| async move {
                    let job_token = CancelOnDrop(token);
                    tokio::select! {
                        biased;
                        () = ctx.cancelled() => {
                            // Stops before the next entry. What is already
                            // moved or copied stays (ADR-028).
                            job_token.0.cancel();
                            Err(CoreError::cancelled("file transfer cancelled"))
                        }
                        joined = &mut job => {
                            let progress = finish_job(joined, "transfer")?;
                            let result = me.settle_transfer(
                                &workspace_id, &pairs, progress, mode, &actor, &correlation,
                            );
                            Ok(Some(serde_json::to_value(result)?))
                        }
                    }
                });
                Ok(FileOperationResponse::Pending {
                    operation: Box::new(operation),
                })
            }
        }
    }

    /// Record what a transfer actually did: only the pairs it completed.
    fn settle_transfer(
        &self,
        workspace_id: &WorkspaceId,
        pairs: &[(WorkspacePath, WorkspacePath)],
        progress: TransferProgress,
        mode: TransferMode,
        actor: &Actor,
        correlation: &CorrelationId,
    ) -> FileOperationResult {
        let done = &pairs[..progress.done];
        let mut changed = BTreeSet::new();
        for (from, to) in done {
            mark_changed(&mut changed, from);
            mark_changed(&mut changed, to);
        }
        if !done.is_empty() {
            let label = transfer_label(mode, done);
            match mode {
                TransferMode::Move => {
                    self.after_move(workspace_id, done, actor, correlation);
                    self.with_journal(workspace_id, |journal| {
                        journal.record(JournalEntry::moved(done.to_vec(), label));
                    });
                }
                TransferMode::Copy => {
                    self.with_journal(workspace_id, |journal| {
                        journal.record(JournalEntry::copied(done.to_vec(), label));
                    });
                }
            }
        }
        let moved = done
            .iter()
            .map(|(from, to)| MovedEntry {
                from: from.clone(),
                to: to.clone(),
            })
            .collect();
        self.result(
            workspace_id,
            changed,
            moved,
            progress.cancelled.then_some(progress.done as u32),
        )
    }

    /// `files.delete`: to the trash, or permanently.
    ///
    /// Refuses while an open document under one of the paths is dirty, unless
    /// `force` — deleting a file is the one operation here that can lose an
    /// unsaved buffer, because nothing carries it anywhere (hard rule 7).
    pub async fn delete(
        self: &Arc<Self>,
        request: DeleteEntriesRequest,
        actor: Actor,
        correlation: CorrelationId,
        budget: Duration,
    ) -> Result<FileOperationResponse, CoreError> {
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        if request.paths.is_empty() {
            return Err(CoreError::invalid_input(
                "paths",
                "must name at least one entry",
            ));
        }
        if request.force != Some(true)
            && let Some(guard) = self.document_guard()
        {
            let dirty = guard.dirty_under(&workspace.id, &request.paths);
            if !dirty.is_empty() {
                return Err(CoreError::conflict(format!(
                    "{} document(s) under the selection have unsaved changes; save or close them, or pass force",
                    dirty.len()
                ))
                .with_details(serde_json::json!({
                    "workspaceId": workspace.id,
                    "dirtyDocuments": dirty,
                })));
            }
        }

        let token = CancellationToken::new();
        let mut job = {
            let root = Arc::clone(&root);
            let paths = request.paths.clone();
            let mode = request.mode;
            let token = token.clone();
            self.handle
                .spawn_blocking(move || delete_blocking(&root, &paths, mode, &token))
        };
        let paths = request.paths.clone();
        let settle = |done: usize, cancelled: bool| {
            let mut changed = BTreeSet::new();
            for path in &paths[..done] {
                mark_changed(&mut changed, path);
            }
            // Deliberately not journalled: a trashed entry is recovered from
            // the Trash and a permanent one is gone (ADR-028).
            self.result(
                &workspace.id,
                changed,
                Vec::new(),
                cancelled.then_some(done as u32),
            )
        };
        match tokio::time::timeout(budget, &mut job).await {
            Ok(joined) => {
                let progress = finish_job(joined, "delete")?;
                Ok(FileOperationResponse::Ready {
                    result: settle(progress.done, progress.cancelled),
                })
            }
            Err(_elapsed) => {
                let spec = OperationSpec {
                    kind: OperationKind::new(FILES_KIND),
                    actor,
                    workspace_id: Some(workspace.id.clone()),
                    correlation_id: correlation,
                    durable: false,
                };
                let me = Arc::clone(self);
                let workspace_id = workspace.id.clone();
                let paths = request.paths.clone();
                let operation = self.registry.start(spec, move |ctx| async move {
                    let job_token = CancelOnDrop(token);
                    tokio::select! {
                        biased;
                        () = ctx.cancelled() => {
                            job_token.0.cancel();
                            Err(CoreError::cancelled("delete cancelled"))
                        }
                        joined = &mut job => {
                            let progress = finish_job(joined, "delete")?;
                            let mut changed = BTreeSet::new();
                            for path in &paths[..progress.done] {
                                mark_changed(&mut changed, path);
                            }
                            let result = me.result(
                                &workspace_id,
                                changed,
                                Vec::new(),
                                progress.cancelled.then_some(progress.done as u32),
                            );
                            Ok(Some(serde_json::to_value(result)?))
                        }
                    }
                });
                Ok(FileOperationResponse::Pending {
                    operation: Box::new(operation),
                })
            }
        }
    }

    /// `files.undo` and `files.redo`.
    ///
    /// The entry is taken, performed, and only then moved to the other stack —
    /// so a refusal leaves the history exactly as it was, and the user can try
    /// again after fixing whatever moved.
    pub async fn step(
        self: &Arc<Self>,
        request: UndoRequest,
        direction: Direction,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<FileOperationResult, CoreError> {
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        let taken = self.with_journal(&workspace.id, |journal| match direction {
            Direction::Undo => journal.take_undo(),
            Direction::Redo => journal.take_redo(),
        });
        let Some(entry) = taken else {
            return Err(CoreError::conflict(match direction {
                Direction::Undo => "there is nothing to undo",
                Direction::Redo => "there is nothing to redo",
            }));
        };
        let step = match direction {
            Direction::Undo => entry.undo.clone(),
            Direction::Redo => entry.redo.clone(),
        };
        let outcome = {
            let root = Arc::clone(&root);
            let step = step.clone();
            self.handle
                .spawn_blocking(move || perform_step(&root, &step))
                .await
        };
        let performed = match finish_job(outcome, "undo") {
            Ok(performed) => performed,
            Err(error) => {
                self.with_journal(&workspace.id, |journal| match direction {
                    Direction::Undo => journal.undo_failed(entry),
                    Direction::Redo => journal.redo_failed(entry),
                });
                return Err(error);
            }
        };
        self.after_move(&workspace.id, &performed.moved, &actor, &correlation);
        self.with_journal(&workspace.id, |journal| match direction {
            Direction::Undo => journal.undo_succeeded(entry),
            Direction::Redo => journal.redo_succeeded(entry),
        });
        let moved = performed
            .moved
            .iter()
            .map(|(from, to)| MovedEntry {
                from: from.clone(),
                to: to.clone(),
            })
            .collect();
        Ok(self.result(&workspace.id, performed.changed, moved, None))
    }
}

/// Which stack [`FileService::step`] works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Undo,
    Redo,
}

/// Stops the blocking job when the operation body finishes, is cancelled, or is
/// aborted at an await point. The same guard `workspace.list` uses.
struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// How far a batch got before it stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TransferProgress {
    /// Entries fully completed. Only these are journalled or reported as moved.
    done: usize,
    cancelled: bool,
}

fn finish_job<T>(
    joined: Result<Result<T, CoreError>, tokio::task::JoinError>,
    what: &str,
) -> Result<T, CoreError> {
    joined.map_err(|join| CoreError::internal(format!("{what} task failed: {join}")))?
}

/// Choose a free destination for every entry, in order, so two entries with the
/// same name in one batch do not both claim it.
fn plan_transfer(
    root: &WorkspaceRoot,
    request: &MoveEntriesRequest,
) -> Result<Vec<(WorkspacePath, WorkspacePath)>, CoreError> {
    let mut pairs: Vec<(WorkspacePath, WorkspacePath)> = Vec::new();
    for path in &request.paths {
        let Some(name) = path.name() else {
            return Err(CoreError::invalid_input(
                "paths",
                "the workspace root cannot be moved",
            ));
        };
        let planned: Vec<WorkspacePath> = pairs.iter().map(|(_, to)| to.clone()).collect();
        let taken = |candidate: &WorkspacePath| {
            planned.contains(candidate) || mutate::exists(root, candidate)
        };
        let destination = match request.mode {
            // Only a copy invents a new name. A *move* onto a taken name is a
            // conflict the user has to resolve: silently landing as
            // `main copy.rs` would be a rename nobody asked for, and dragging a
            // file onto the folder it is already in would do exactly that.
            TransferMode::Copy => destination_in(&request.target_dir, name, &taken)?,
            TransferMode::Move => {
                let destination = request.target_dir.join(name).map_err(CoreError::from)?;
                if destination == *path {
                    return Err(CoreError::invalid_input(
                        "targetDir",
                        format!("{path} is already in {}", request.target_dir),
                    ));
                }
                if taken(&destination) {
                    return Err(CoreError::conflict(format!("{destination} already exists")));
                }
                destination
            }
        };
        pairs.push((path.clone(), destination));
    }
    Ok(pairs)
}

fn transfer_blocking(
    root: &WorkspaceRoot,
    pairs: &[(WorkspacePath, WorkspacePath)],
    mode: TransferMode,
    token: &CancellationToken,
) -> Result<TransferProgress, CoreError> {
    let mut done = 0;
    for (from, to) in pairs {
        if token.is_cancelled() {
            return Ok(TransferProgress {
                done,
                cancelled: true,
            });
        }
        match mode {
            TransferMode::Move => mutate::rename(root, from, to)?,
            TransferMode::Copy => {
                let CopyOutcome { cancelled, .. } =
                    mutate::copy(root, from, to, &|| token.is_cancelled())?;
                if cancelled {
                    return Ok(TransferProgress {
                        done,
                        cancelled: true,
                    });
                }
            }
        }
        done += 1;
    }
    Ok(TransferProgress {
        done,
        cancelled: false,
    })
}

fn delete_blocking(
    root: &WorkspaceRoot,
    paths: &[WorkspacePath],
    mode: DeleteMode,
    token: &CancellationToken,
) -> Result<TransferProgress, CoreError> {
    let mut done = 0;
    for path in paths {
        if token.is_cancelled() {
            return Ok(TransferProgress {
                done,
                cancelled: true,
            });
        }
        match mode {
            DeleteMode::Trash => mutate::trash(root, path)?,
            DeleteMode::Permanent => mutate::remove(root, path)?,
        }
        done += 1;
    }
    Ok(TransferProgress {
        done,
        cancelled: false,
    })
}

/// What performing one journal step changed.
struct Performed {
    changed: BTreeSet<WorkspacePath>,
    moved: Vec<(WorkspacePath, WorkspacePath)>,
}

/// Perform one [`Step`]. Every failure is reported rather than swallowed, and
/// nothing is forced: an undo that would have to overwrite something that
/// appeared in the meantime fails with `Conflict` (ADR-028).
fn perform_step(root: &WorkspaceRoot, step: &Step) -> Result<Performed, CoreError> {
    let mut changed = BTreeSet::new();
    let mut moved = Vec::new();
    match step {
        Step::Remove(paths) => {
            for path in paths {
                // Idempotent on purpose. This step's goal is that `path` is
                // absent, and something else having already removed it is that
                // goal reached, not a failure — removing nothing overwrites
                // nothing, so the rule this relaxes is not the one protecting
                // the user's data.
                if !mutate::exists(root, path) {
                    mark_changed(&mut changed, path);
                    continue;
                }
                mutate::remove(root, path)?;
                mark_changed(&mut changed, path);
            }
        }
        Step::Move(pairs) => {
            for (from, to) in pairs {
                mutate::rename(root, from, to)?;
                mark_changed(&mut changed, from);
                mark_changed(&mut changed, to);
                moved.push((from.clone(), to.clone()));
            }
        }
        Step::Create(entries) => {
            for (path, kind) in entries {
                mutate::create(root, path, *kind)?;
                mark_changed(&mut changed, path);
            }
        }
        Step::Copy(pairs) => {
            for (from, to) in pairs {
                mutate::copy(root, from, to, &|| false)?;
                mark_changed(&mut changed, to);
            }
        }
    }
    Ok(Performed { changed, moved })
}

fn transfer_label(mode: TransferMode, done: &[(WorkspacePath, WorkspacePath)]) -> String {
    let verb = match mode {
        TransferMode::Move => "Move",
        TransferMode::Copy => "Copy",
    };
    match done {
        [(from, _)] => format!("{verb} {from}"),
        many => format!("{verb} {} items", many.len()),
    }
}

/// Every ancestor directory of `path`, root included: the directories whose
/// listing a change to `path` can invalidate. A create may have made
/// intermediate directories, so the whole chain is named rather than just the
/// immediate parent.
fn mark_changed(changed: &mut BTreeSet<WorkspacePath>, path: &WorkspacePath) {
    let mut at = path.parent();
    while let Some(current) = at {
        at = current.parent();
        changed.insert(current);
    }
}

/// Where an entry lands in `target_dir`, avoiding a name that is already taken
/// the way every file manager does: `name copy`, then `name copy 2`.
///
/// Pure, and separated from the filesystem call so the naming rule is tested
/// without one. `taken` answers whether a candidate already exists.
pub fn destination_in(
    target_dir: &WorkspacePath,
    name: &str,
    taken: &dyn Fn(&WorkspacePath) -> bool,
) -> Result<WorkspacePath, CoreError> {
    let candidate = target_dir.join(name).map_err(CoreError::from)?;
    if !taken(&candidate) {
        return Ok(candidate);
    }
    let (stem, extension) = split_name(name);
    for attempt in 1..1000 {
        let suffix = if attempt == 1 {
            " copy".to_string()
        } else {
            format!(" copy {attempt}")
        };
        let candidate = target_dir
            .join(&format!("{stem}{suffix}{extension}"))
            .map_err(CoreError::from)?;
        if !taken(&candidate) {
            return Ok(candidate);
        }
    }
    Err(CoreError::conflict(format!(
        "{name} and its copies already exist in {target_dir}"
    )))
}

/// Split at the last dot, so `a.tar.gz` becomes `a.tar` + `.gz` and a dotfile
/// like `.gitignore` keeps its whole name as the stem — a copy of it should be
/// `.gitignore copy`, not `. copy gitignore`.
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(0) | None => (name, ""),
        Some(index) => name.split_at(index),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(raw: &str) -> WorkspacePath {
        WorkspacePath::parse(raw).expect("valid wire path")
    }

    #[test]
    fn changed_names_every_ancestor_of_a_created_path() {
        let mut changed = BTreeSet::new();
        mark_changed(&mut changed, &path("a/b/c.ts"));
        let named: Vec<&str> = changed.iter().map(WorkspacePath::as_str).collect();
        assert_eq!(named, vec!["", "a", "a/b"]);
    }

    #[test]
    fn a_free_name_is_used_unchanged() {
        let free = destination_in(&path("dst"), "a.ts", &|_| false).expect("free");
        assert_eq!(free.as_str(), "dst/a.ts");
    }

    #[test]
    fn a_taken_name_becomes_copy_then_copy_2() {
        let taken_once = |candidate: &WorkspacePath| candidate.as_str() == "dst/a.ts";
        let first = destination_in(&path("dst"), "a.ts", &taken_once).expect("first copy");
        assert_eq!(first.as_str(), "dst/a copy.ts");

        let taken_twice =
            |candidate: &WorkspacePath| matches!(candidate.as_str(), "dst/a.ts" | "dst/a copy.ts");
        let second = destination_in(&path("dst"), "a.ts", &taken_twice).expect("second copy");
        assert_eq!(second.as_str(), "dst/a copy 2.ts");
    }

    #[test]
    fn a_dotfile_and_a_double_extension_keep_their_names() {
        assert_eq!(split_name(".gitignore"), (".gitignore", ""));
        assert_eq!(split_name("a.tar.gz"), ("a.tar", ".gz"));
        assert_eq!(split_name("Makefile"), ("Makefile", ""));

        let taken = |candidate: &WorkspacePath| candidate.as_str() == "dst/.gitignore";
        let copied = destination_in(&path("dst"), ".gitignore", &taken).expect("copy");
        assert_eq!(copied.as_str(), "dst/.gitignore copy");
    }

    #[test]
    fn a_name_with_a_separator_is_refused_rather_than_becoming_a_move() {
        let refused = destination_in(&path("dst"), "sneaky/../../etc", &|_| false).unwrap_err();
        assert_eq!(
            refused.code,
            crate::protocol::error::CoreErrorCode::InvalidInput
        );
    }

    // ---- the service, against a real filesystem ----

    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::event::EventBus;
    use crate::event::testing::RecordingSink;
    use crate::protocol::error::CoreErrorCode;
    use crate::workspace::{UnsavedPolicy, Workspace};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir()
                .join(format!("gritty-files-{label}-{}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("src")).expect("scratch dir");
            fs::write(dir.join("src/main.rs"), "fn main() {}").expect("seed file");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Stands in for `DocumentManager`: records what it was asked and what it
    /// was told, so the two directions of the seam are asserted without the
    /// document layer.
    #[derive(Default)]
    struct FakeDocuments {
        dirty: Mutex<Vec<WorkspacePath>>,
        rehomed: Mutex<Vec<(WorkspacePath, WorkspacePath)>>,
    }

    impl DocumentTreeGuard for FakeDocuments {
        fn dirty_under(
            &self,
            _workspace_id: &WorkspaceId,
            paths: &[WorkspacePath],
        ) -> Vec<serde_json::Value> {
            let dirty = self.dirty.lock().expect("lock");
            dirty
                .iter()
                .filter(|candidate| {
                    paths.iter().any(|path| {
                        *candidate == path
                            || candidate
                                .as_str()
                                .strip_prefix(path.as_str())
                                .is_some_and(|rest| rest.starts_with('/'))
                    })
                })
                .map(|path| serde_json::json!({ "path": path }))
                .collect()
        }

        fn rehome(
            &self,
            _workspace_id: &WorkspaceId,
            from: &WorkspacePath,
            to: &WorkspacePath,
            _actor: &Actor,
            _correlation: &CorrelationId,
        ) {
            self.rehomed
                .lock()
                .expect("lock")
                .push((from.clone(), to.clone()));
        }
    }

    struct Harness {
        files: Arc<FileService>,
        documents: Arc<FakeDocuments>,
        workspace: Workspace,
        dir: TempDir,
    }

    fn harness(label: &str) -> Harness {
        let (sink, _rx) = RecordingSink::with_channel();
        let events = EventBus::new(sink);
        let registry = OperationRegistry::new(Handle::current(), events.clone());
        let workspaces = WorkspaceManager::new(Handle::current(), events, registry.clone());
        let dir = TempDir::new(label);
        let workspace = workspaces
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .expect("open workspace");
        let files = FileService::new(Handle::current(), workspaces, registry);
        let documents = Arc::new(FakeDocuments::default());
        let as_guard: Arc<dyn DocumentTreeGuard> = documents.clone();
        files.set_document_guard(Arc::downgrade(&as_guard));
        // The composition root holds this; the test holds it through `documents`.
        std::mem::forget(as_guard);
        Harness {
            files,
            documents,
            workspace,
            dir,
        }
    }

    impl Harness {
        fn create(&self, raw: &str, kind: NewEntryKind) -> Result<FileOperationResult, CoreError> {
            self.files.create(
                CreateEntryRequest {
                    workspace_id: self.workspace.id.clone(),
                    path: path(raw),
                    kind,
                },
                CorrelationId::new(),
            )
        }

        async fn undo(&self) -> Result<FileOperationResult, CoreError> {
            self.files
                .step(
                    UndoRequest {
                        workspace_id: self.workspace.id.clone(),
                    },
                    Direction::Undo,
                    Actor::User,
                    CorrelationId::new(),
                )
                .await
        }

        async fn redo(&self) -> Result<FileOperationResult, CoreError> {
            self.files
                .step(
                    UndoRequest {
                        workspace_id: self.workspace.id.clone(),
                    },
                    Direction::Redo,
                    Actor::User,
                    CorrelationId::new(),
                )
                .await
        }
    }

    const BUDGET: Duration = Duration::from_secs(5);

    #[tokio::test]
    async fn a_create_is_undone_by_removing_it_and_redone_by_making_it_again() {
        let h = harness("create-undo");
        let made = h.create("a/b/c.ts", NewEntryKind::File).expect("create");
        assert!(h.dir.0.join("a/b/c.ts").is_file());
        // Every ancestor is named, because `a` and `a/b` are new too.
        let changed: Vec<&str> = made.changed.iter().map(WorkspacePath::as_str).collect();
        assert_eq!(changed, vec!["", "a", "a/b"]);
        assert_eq!(made.undo.as_deref(), Some("Create a/b/c.ts"));

        h.undo().await.expect("undo");
        assert!(!h.dir.0.join("a/b/c.ts").exists());

        let redone = h.redo().await.expect("redo");
        assert!(h.dir.0.join("a/b/c.ts").is_file());
        assert_eq!(redone.redo, None, "nothing left to redo");
    }

    #[tokio::test]
    async fn a_rename_tells_the_document_layer_where_the_file_went() {
        let h = harness("rename");
        let renamed = h
            .files
            .rename(
                RenameEntryRequest {
                    workspace_id: h.workspace.id.clone(),
                    path: path("src/main.rs"),
                    new_name: "app.rs".to_string(),
                },
                Actor::User,
                CorrelationId::new(),
            )
            .expect("rename");

        assert!(h.dir.0.join("src/app.rs").is_file());
        assert_eq!(renamed.moved.len(), 1);
        assert_eq!(renamed.moved[0].to.as_str(), "src/app.rs");
        assert_eq!(
            *h.documents.rehomed.lock().expect("lock"),
            vec![(path("src/main.rs"), path("src/app.rs"))],
            "the document layer is told, because no event can carry a rename"
        );

        h.undo().await.expect("undo");
        assert!(h.dir.0.join("src/main.rs").is_file());
        assert_eq!(
            h.documents.rehomed.lock().expect("lock").len(),
            2,
            "the undo moves the document back too"
        );
    }

    #[tokio::test]
    async fn a_rename_whose_name_is_a_path_is_refused() {
        let h = harness("rename-path");
        let refused = h
            .files
            .rename(
                RenameEntryRequest {
                    workspace_id: h.workspace.id.clone(),
                    path: path("src/main.rs"),
                    new_name: "../escaped.rs".to_string(),
                },
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap_err();
        assert_eq!(refused.code, CoreErrorCode::InvalidInput);
        assert!(h.dir.0.join("src/main.rs").is_file());
    }

    #[tokio::test]
    async fn a_delete_refuses_an_unsaved_buffer_until_it_is_forced() {
        let h = harness("delete-dirty");
        h.documents
            .dirty
            .lock()
            .expect("lock")
            .push(path("src/main.rs"));

        let request = |force: Option<bool>| DeleteEntriesRequest {
            workspace_id: h.workspace.id.clone(),
            paths: vec![path("src")],
            mode: DeleteMode::Permanent,
            force,
        };

        let refused = h
            .files
            .delete(request(None), Actor::User, CorrelationId::new(), BUDGET)
            .await
            .unwrap_err();
        assert_eq!(refused.code, CoreErrorCode::Conflict);
        assert!(
            refused.details.expect("details")["dirtyDocuments"]
                .as_array()
                .is_some_and(|dirty| dirty.len() == 1),
            "the refusal names what is unsaved so the UI can offer the choice"
        );
        assert!(h.dir.0.join("src/main.rs").is_file());

        h.files
            .delete(
                request(Some(true)),
                Actor::User,
                CorrelationId::new(),
                BUDGET,
            )
            .await
            .expect("forced delete");
        assert!(!h.dir.0.join("src").exists());
    }

    #[tokio::test]
    async fn a_delete_is_never_undoable() {
        let h = harness("delete-undo");
        h.create("gone.txt", NewEntryKind::File).expect("create");
        h.undo().await.expect("undo the create");

        h.create("gone.txt", NewEntryKind::File)
            .expect("create again");
        h.files
            .delete(
                DeleteEntriesRequest {
                    workspace_id: h.workspace.id.clone(),
                    paths: vec![path("gone.txt")],
                    mode: DeleteMode::Permanent,
                    force: None,
                },
                Actor::User,
                CorrelationId::new(),
                BUDGET,
            )
            .await
            .expect("delete");

        // The create is still the newest journal entry: the delete recorded
        // none, so ⌘Z reaches past it rather than pretending to restore. The
        // file is already gone, and undoing "create" onto an absent file is
        // that step's goal already reached, not a failure.
        let after = h.undo().await.expect("undo reaches past the delete");
        assert_eq!(after.undo, None, "the journal held only the create");
        assert!(!h.dir.0.join("gone.txt").exists());
    }

    #[tokio::test]
    async fn a_copy_onto_a_taken_name_becomes_a_copy_and_undoes_to_nothing() {
        let h = harness("copy");
        let copied = h
            .files
            .transfer(
                MoveEntriesRequest {
                    workspace_id: h.workspace.id.clone(),
                    paths: vec![path("src/main.rs")],
                    target_dir: path("src"),
                    mode: TransferMode::Copy,
                },
                Actor::User,
                CorrelationId::new(),
                BUDGET,
            )
            .await
            .expect("copy");
        let FileOperationResponse::Ready { result } = copied else {
            panic!("a one-file copy fits in the budget");
        };
        assert_eq!(result.moved[0].to.as_str(), "src/main copy.rs");
        assert!(h.dir.0.join("src/main copy.rs").is_file());

        h.undo().await.expect("undo");
        assert!(!h.dir.0.join("src/main copy.rs").exists());
        assert!(
            h.dir.0.join("src/main.rs").is_file(),
            "undoing a copy removes the copy, never the original"
        );
    }

    #[tokio::test]
    async fn a_move_never_invents_a_name_the_way_a_copy_does() {
        let h = harness("move-names");
        let move_into = |target: &str| {
            let request = MoveEntriesRequest {
                workspace_id: h.workspace.id.clone(),
                paths: vec![path("src/main.rs")],
                target_dir: path(target),
                mode: TransferMode::Move,
            };
            h.files
                .transfer(request, Actor::User, CorrelationId::new(), BUDGET)
        };

        // Dropping a file on the folder it already lives in must not quietly
        // become "main copy.rs".
        let onto_itself = move_into("src").await.unwrap_err();
        assert_eq!(onto_itself.code, CoreErrorCode::InvalidInput);
        assert!(h.dir.0.join("src/main.rs").is_file());
        assert!(!h.dir.0.join("src/main copy.rs").exists());

        // And a move onto a name something else holds is a conflict for the
        // user to resolve, not a silent rename and not an overwrite.
        fs::write(h.dir.0.join("main.rs"), "a different file").expect("write");
        let onto_taken = move_into("").await.unwrap_err();
        assert_eq!(onto_taken.code, CoreErrorCode::Conflict);
        assert_eq!(
            fs::read_to_string(h.dir.0.join("main.rs")).expect("read"),
            "a different file"
        );
    }

    #[tokio::test]
    async fn an_undo_whose_destination_was_taken_is_refused_and_stays_undoable() {
        let h = harness("undo-conflict");
        h.files
            .rename(
                RenameEntryRequest {
                    workspace_id: h.workspace.id.clone(),
                    path: path("src/main.rs"),
                    new_name: "app.rs".to_string(),
                },
                Actor::User,
                CorrelationId::new(),
            )
            .expect("rename");
        // Something else takes the old name in the meantime.
        fs::write(h.dir.0.join("src/main.rs"), "someone else").expect("write");

        let refused = h.undo().await.unwrap_err();
        assert_eq!(refused.code, CoreErrorCode::Conflict);
        assert_eq!(
            fs::read_to_string(h.dir.0.join("src/main.rs")).expect("read"),
            "someone else",
            "the undo never overwrites what appeared"
        );

        // Still on the undo stack: fix the collision and it works.
        fs::remove_file(h.dir.0.join("src/main.rs")).expect("remove");
        h.undo().await.expect("undo now that the name is free");
        assert!(h.dir.0.join("src/main.rs").is_file());
    }

    #[tokio::test]
    async fn an_undo_with_an_empty_journal_is_a_conflict_not_a_panic() {
        let h = harness("undo-empty");
        let refused = h.undo().await.unwrap_err();
        assert_eq!(refused.code, CoreErrorCode::Conflict);
    }
}
