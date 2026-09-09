//! The write half: `git.stage`, `git.unstage`, `git.discard`, `git.commit`
//! (T102-C, behind ADR-029).
//!
//! Each follows the same four steps, and the order matters:
//!
//! 1. **Validate before anything runs.** An empty pathspec, an empty commit
//!    message, or a path outside the workspace root is `InvalidInput` with no
//!    process spawned and nothing changed.
//! 2. **Refuse rather than surprise.** A discard over a dirty open buffer is
//!    `Conflict` with `details.dirtyDocuments` and a `force` escape — hard rule
//!    7, and the same refusal `files.delete` already makes through the same
//!    [`DocumentTreeGuard`].
//! 3. **Run the write** through `gritty-git-write`, which owns the argv, the
//!    environment and the process group.
//! 4. **Say what changed**, by re-reading the repository with `gix` and
//!    returning it. The caller converges on the response; nothing emits a
//!    `CoreEvent` and nothing re-polls `git.status`.
//!
//! Step 4 is why this module exists rather than the adapter answering directly:
//! the two git crates must not know about each other, so the composition lives
//! here (ADR-029 decision 2).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use gritty_git::{FileState, MAX_PAGE_SIZE, PageOptions, StatusOptions};
use gritty_git_write::{Availability, CommitOptions, GitWrite, GitWriteError};
use gritty_workspace::{WorkspacePath, WorkspaceRoot};

use super::{GitService, finish};
use crate::operation::{OperationKind, OperationSpec};
use crate::protocol::actor::Actor;
use crate::protocol::error::CoreError;
use crate::protocol::ids::CorrelationId;

use super::wire::{
    GitCommitRequest, GitDiscardRequest, GitStageRequest, GitUnstageRequest, GitWriteAvailability,
    GitWriteResponse, GitWriteResult, StashRef,
};

/// The one operation kind all four writes share.
///
/// One rather than four, the way `files.transfer` covers both `files.move` and
/// `files.delete`: a UI matching on the kind should not have to know which verb
/// is running (T102-C requirement 5).
pub const WRITE_KIND: &str = "git.write";

/// Where the message file is written, inside the repository's own git
/// directory so it is never mistaken for a worktree change.
const MESSAGE_FILE: &str = "GRITTY_COMMIT_MSG";

impl GitService {
    /// Install the guard that lets a discard refuse over a dirty buffer.
    /// Called once by the composition root, exactly as `FileService`'s is.
    pub fn set_document_guard(&self, guard: std::sync::Weak<dyn super::DocumentTreeGuard>) {
        if let Ok(mut slot) = self.document_guard.lock() {
            *slot = Some(guard);
        }
    }

    /// The probe verdict, computed at most once per process.
    pub async fn availability(&self) -> GitWriteAvailability {
        match self.probe().await {
            Availability::Available { version, .. } => GitWriteAvailability {
                available: true,
                version: Some(version.clone()),
                reason: None,
            },
            Availability::Unavailable { reason } => GitWriteAvailability {
                available: false,
                version: None,
                reason: Some(reason.clone()),
            },
        }
    }

    async fn probe(&self) -> &Availability {
        self.probe
            // `git` by name, resolved through the user's `PATH`. Deliberately
            // not an absolute path: the whole point of ADR-029 is that a commit
            // is made by *their* git, with their config, their signing key and
            // their hooks — which means the one their shell would have found.
            .get_or_init(|| async { gritty_git_write::probe(&PathBuf::from("git")).await })
            .await
    }

    /// The `git` to run, or the refusal that says why there is none.
    async fn program(&self) -> Result<PathBuf, CoreError> {
        match self.probe().await {
            Availability::Available { program, .. } => Ok(program.clone()),
            // `PermissionDenied` would imply the user could fix it by granting
            // something; this is a missing tool, so it is `NotFound` with the
            // reason attached for the UI to show.
            Availability::Unavailable { reason } => Err(CoreError::not_found(format!(
                "git is unavailable: {reason}"
            ))
            .with_details(serde_json::json!({ "reason": reason }))),
        }
    }

    /// `git.stage`.
    pub async fn stage(
        self: &Arc<Self>,
        request: GitStageRequest,
        actor: Actor,
        correlation: CorrelationId,
        budget: Duration,
    ) -> Result<GitWriteResponse, CoreError> {
        self.simple_write(
            request.workspace_id,
            request.paths,
            actor,
            correlation,
            budget,
            GitWrite::Stage { paths: Vec::new() },
        )
        .await
    }

    /// `git.unstage`.
    pub async fn unstage(
        self: &Arc<Self>,
        request: GitUnstageRequest,
        actor: Actor,
        correlation: CorrelationId,
        budget: Duration,
    ) -> Result<GitWriteResponse, CoreError> {
        self.simple_write(
            request.workspace_id,
            request.paths,
            actor,
            correlation,
            budget,
            GitWrite::Unstage { paths: Vec::new() },
        )
        .await
    }

    /// The shape `stage` and `unstage` share: validate, run, re-read.
    ///
    /// `shape` carries the variant to build; its `paths` are replaced. That is
    /// a small ugliness bought for not repeating forty lines twice.
    async fn simple_write(
        self: &Arc<Self>,
        workspace_id: crate::protocol::ids::WorkspaceId,
        paths: Vec<WorkspacePath>,
        _actor: Actor,
        correlation: CorrelationId,
        _budget: Duration,
        shape: GitWrite,
    ) -> Result<GitWriteResponse, CoreError> {
        let (_workspace, root) = self.workspaces.require(&workspace_id, &correlation)?;
        require_paths(&paths)?;
        if self.open_root(&root).await?.is_none() {
            return Ok(GitWriteResponse::NotARepository);
        }
        let program = self.program().await?;

        let write = match shape {
            GitWrite::Stage { .. } => GitWrite::Stage {
                paths: paths.clone(),
            },
            _ => GitWrite::Unstage {
                paths: paths.clone(),
            },
        };
        gritty_git_write::perform(&program, root.path(), &write)
            .await
            .map_err(to_core_error)?;

        let result = self.describe(&root, paths, None, None, Vec::new()).await?;
        Ok(GitWriteResponse::Ready {
            result: Box::new(result),
        })
    }

    /// `git.discard`.
    ///
    /// Two dispositions, because the two cases are genuinely different:
    /// **tracked** modifications are stashed and then gone from the worktree,
    /// **untracked** files go to the OS Trash. `git stash` does not take
    /// untracked files without `-u`, and `-u` would stash *every* untracked
    /// file rather than the named ones — which is exactly the wholesale write
    /// ADR-029 decision 3 forbids.
    pub async fn discard(
        self: &Arc<Self>,
        request: GitDiscardRequest,
        _actor: Actor,
        correlation: CorrelationId,
        _budget: Duration,
    ) -> Result<GitWriteResponse, CoreError> {
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        require_paths(&request.paths)?;
        if self.open_root(&root).await?.is_none() {
            return Ok(GitWriteResponse::NotARepository);
        }

        // Hard rule 7, before anything is destroyed.
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

        let program = self.program().await?;
        let (tracked, untracked) = self.split_by_tracking(&root, &request.paths).await?;

        // The stash first: nothing is thrown away before there is a copy of it.
        let mut stash = None;
        if !tracked.is_empty() {
            let before = stash_head(&root);
            gritty_git_write::perform(
                &program,
                root.path(),
                &GitWrite::Stash {
                    paths: tracked.clone(),
                },
            )
            .await
            .map_err(to_core_error)?;
            let after = stash_head(&root);
            // A stash was created only if the ref moved. `git stash push` on
            // paths with nothing to save prints "No local changes to save" and
            // exits 0, and reading the ref is how that is known without
            // parsing the message.
            if after.is_some() && after != before {
                stash = after.map(|commit| StashRef {
                    commit,
                    name: "stash@{0}".to_owned(),
                });
            }
        }

        let mut trashed = Vec::new();
        for path in &untracked {
            gritty_workspace::mutate::trash(&root, path).map_err(CoreError::from)?;
            trashed.push(path.clone());
        }

        let result = self
            .describe(&root, request.paths, None, stash, trashed)
            .await?;
        Ok(GitWriteResponse::Ready {
            result: Box::new(result),
        })
    }

    /// `git.commit`.
    ///
    /// The one write that is an `Operation`: a `pre-commit` hook can run a
    /// whole test suite, so it can outlive any budget (hard rule 10, ADR-029
    /// decision 6). Cancelling it kills the hook's process group.
    pub async fn commit(
        self: &Arc<Self>,
        request: GitCommitRequest,
        actor: Actor,
        correlation: CorrelationId,
        budget: Duration,
    ) -> Result<GitWriteResponse, CoreError> {
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        if request.message.trim().is_empty() {
            return Err(CoreError::invalid_input(
                "message",
                "a commit message cannot be empty",
            ));
        }
        if self.open_root(&root).await?.is_none() {
            return Ok(GitWriteResponse::NotARepository);
        }
        let program = self.program().await?;

        // Written into the git directory, so it is never a worktree change and
        // never reaches the index. The adapter passes it with `-F`; the message
        // never touches a command line.
        let message_file = root.path().join(".git").join(MESSAGE_FILE);
        std::fs::write(&message_file, request.message.as_bytes()).map_err(CoreError::from)?;

        let write = GitWrite::Commit {
            message_file,
            options: CommitOptions {
                amend: request.amend.unwrap_or(false),
                signoff: request.signoff.unwrap_or(false),
                no_verify: request.no_verify.unwrap_or(false),
            },
        };

        let mut running =
            gritty_git_write::spawn(&program, root.path(), &write).map_err(to_core_error)?;

        // The budget hand-over the reads use, with one difference: what
        // continues is a *child process*, not a `spawn_blocking` job, so
        // cancellation kills a process group rather than setting a flag. No
        // operation in this tree owned a child before this one.
        let finished = match tokio::time::timeout(budget, running.wait()).await {
            Ok(finished) => finished.map_err(to_core_error)?,
            Err(_elapsed) => {
                tracing::debug!(
                    workspace = %workspace.id,
                    budget_ms = budget.as_millis() as u64,
                    "git commit exceeded its budget; continuing as an operation"
                );
                let service = Arc::clone(self);
                let root_for_body = Arc::clone(&root);
                let paths = Vec::new();
                let spec = OperationSpec {
                    kind: OperationKind::new(WRITE_KIND),
                    actor,
                    workspace_id: Some(workspace.id),
                    correlation_id: correlation,
                    durable: false,
                };
                let operation = self.registry.start(spec, move |ctx| async move {
                    let outcome = tokio::select! {
                        biased;
                        () = ctx.cancelled() => {
                            // The whole group, so the hook dies with it.
                            running.terminate().await;
                            return Err(CoreError::cancelled("git commit cancelled"));
                        }
                        finished = running.wait() => finished,
                    };
                    let finished = outcome.map_err(to_core_error)?;
                    let checked = gritty_git_write::check(root_for_body.path(), &write, finished)
                        .map_err(to_core_error)?;
                    drop(checked);
                    let head = head_id(&root_for_body);
                    let result = service
                        .describe(&root_for_body, paths, head, None, Vec::new())
                        .await?;
                    Ok(Some(serde_json::to_value(result)?))
                });
                return Ok(GitWriteResponse::Pending {
                    operation: Box::new(operation),
                });
            }
        };

        gritty_git_write::check(root.path(), &write, finished).map_err(to_core_error)?;
        let result = self
            .describe(&root, Vec::new(), head_id(&root), None, Vec::new())
            .await?;
        Ok(GitWriteResponse::Ready {
            result: Box::new(result),
        })
    }

    fn document_guard(&self) -> Option<Arc<dyn super::DocumentTreeGuard>> {
        self.document_guard
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().and_then(std::sync::Weak::upgrade))
    }

    /// Which of `paths` git already tracks. Read through `gix`, because that is
    /// the half of the tree that reads.
    async fn split_by_tracking(
        self: &Arc<Self>,
        root: &Arc<WorkspaceRoot>,
        paths: &[WorkspacePath],
    ) -> Result<(Vec<WorkspacePath>, Vec<WorkspacePath>), CoreError> {
        let status = self.read_status(root).await?;
        let mut tracked = Vec::new();
        let mut untracked = Vec::new();
        for path in paths {
            let entry = status.entries.iter().find(|entry| &entry.path == path);
            let is_untracked = entry.is_some_and(|entry| {
                entry.worktree == FileState::Untracked || entry.index == FileState::Untracked
            });
            if is_untracked {
                untracked.push(path.clone());
            } else {
                tracked.push(path.clone());
            }
        }
        Ok((tracked, untracked))
    }

    /// Re-read the repository so the response can say what it now is.
    async fn describe(
        self: &Arc<Self>,
        root: &Arc<WorkspaceRoot>,
        changed: Vec<WorkspacePath>,
        commit: Option<String>,
        stash: Option<StashRef>,
        trashed: Vec<WorkspacePath>,
    ) -> Result<GitWriteResult, CoreError> {
        let repository = self.read_status(root).await?;
        let mut changed = changed;
        changed.sort();
        changed.dedup();
        Ok(GitWriteResult {
            changed,
            repository: Box::new(repository),
            commit,
            stash,
            trashed,
        })
    }

    async fn read_status(
        self: &Arc<Self>,
        root: &Arc<WorkspaceRoot>,
    ) -> Result<gritty_git::RepositoryStatus, CoreError> {
        let Some(repository) = self.open_root(root).await? else {
            return Err(CoreError::not_found(
                "the workspace root stopped being a git repository",
            ));
        };
        let options = StatusOptions {
            page: PageOptions {
                page_size: MAX_PAGE_SIZE,
                cursor: None,
            },
            include_ignored: false,
        };
        let interrupt = Arc::new(AtomicBool::new(false));
        let joined = self
            .handle
            .spawn_blocking(move || gritty_git::status(&repository, &options, &interrupt))
            .await;
        finish(joined, "git status")
    }
}

/// An empty pathspec is a caller error, not "do nothing": a UI that sends one
/// has a bug, and answering "fine" would hide it.
fn require_paths(paths: &[WorkspacePath]) -> Result<(), CoreError> {
    if paths.is_empty() {
        return Err(CoreError::invalid_input(
            "paths",
            "at least one path is required; there is no wholesale write",
        ));
    }
    Ok(())
}

/// Full hex id of `refs/stash`, or `None` when there is no stash at all.
///
/// Read from the file rather than by running `git rev-parse`: it is one small
/// read, it cannot fail in an interesting way, and it avoids a second process
/// on the discard path. A packed-refs-only stash reads as `None`, which
/// understates rather than misreports — the work is still stashed, the response
/// merely does not name where.
fn stash_head(root: &Arc<WorkspaceRoot>) -> Option<String> {
    let path = root.path().join(".git").join("refs").join("stash");
    let text = std::fs::read_to_string(path).ok()?;
    let id = text.trim();
    (id.len() == 40 && id.chars().all(|c| c.is_ascii_hexdigit())).then(|| id.to_owned())
}

/// Full hex id of HEAD, by the same reasoning as [`stash_head`].
fn head_id(root: &Arc<WorkspaceRoot>) -> Option<String> {
    let git_dir = root.path().join(".git");
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref: ") {
        let text = std::fs::read_to_string(git_dir.join(reference)).ok()?;
        let id = text.trim().to_owned();
        return (id.len() == 40).then_some(id);
    }
    (head.len() == 40).then(|| head.to_owned())
}

/// The adapter's refusals, in the envelope the WebView switches on.
fn to_core_error(error: GitWriteError) -> CoreError {
    match error {
        GitWriteError::Unavailable { reason } => {
            CoreError::not_found(format!("git is unavailable: {reason}"))
        }
        // Retryable, and named rather than retried: a silent loop against a
        // lock another `git` is holding is how a hang gets built.
        GitWriteError::Locked => CoreError::conflict(
            "another git process is holding the index lock; try again in a moment",
        )
        .with_retryable(true),
        // The hook's own words reach the user who has to act on them. They are
        // returned, never logged: this is output from a program the repository
        // named (rule 13).
        GitWriteError::Refused {
            subcommand,
            code,
            stderr,
        } => CoreError::conflict(format!("git {subcommand} refused the write")).with_details(
            serde_json::json!({
                "subcommand": subcommand,
                "exitCode": code,
                "stderr": stderr,
            }),
        ),
        GitWriteError::Cancelled => CoreError::cancelled("the write was cancelled"),
        GitWriteError::Io { context, source } => {
            CoreError::from(source).with_details(serde_json::json!({ "context": context }))
        }
    }
}
