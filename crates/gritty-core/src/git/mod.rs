//! Git reads (T100, extended by T101-C): `git.status`, `git.diff`, `git.log`
//! and `git.show` against the open workspace's root, as domain objects.
//!
//! The repository work lives in `gritty-git`; this module owns identity, the
//! error envelope and the decision to run a read as an operation. All four
//! commands follow `workspace.list`'s shape exactly
//! (`crate::workspace::WorkspaceManager::list`): resolve the [`WorkspaceId`],
//! run the read on the blocking pool, wait up to [`DEFAULT_GIT_BUDGET`], and
//! when it has not finished by then let the same job continue as a cancellable
//! operation whose completion event carries the result. No new event family is
//! introduced — the existing `Operation*` events are the mechanism
//! (`AGENTS.md` rule 9).
//!
//! ## Where the repository is opened
//!
//! **At the workspace root, never discovered above it.** `gix` can walk up
//! from a directory to the repository that contains it; this module does not
//! ask it to. Two reasons: a workspace opened at `repo/crates/git` would
//! otherwise report the status of paths *outside* the workspace root, which
//! the path policy (ADR-010) exists to prevent; and "the workspace root is not
//! a repository" becomes a fact about the workspace rather than a fact about
//! the machine's directory layout. The cost is that opening a subdirectory of
//! a repository reports `NotARepository`, which is a follow-up (T100's report),
//! not a silent behaviour.
//!
//! ## Nothing here writes
//!
//! Four reads, and no fifth command that stages, commits or discards. That is
//! not an omission: a write needs the `git` binary rather than `gix`, because
//! `open.rs` deliberately loads no clean filter and a filterless *write* would
//! put un-cleaned content in the index. ADR-029 is that decision, and an ADR is
//! not an agent's to make.
//!
//! ## What this module is not
//!
//! It is not a `ChangeSet` and knows nothing about workstreams. `ChangeSet`
//! carries test and review status and is workstream-linked; hard rule 6 keeps
//! provider- and workstream-shaped fields out of it, and
//! `crate::workstream`'s own header promises that nothing there reaches into
//! Git. This module keeps both halves of that true: `git/**` never imports
//! `workstream/**`.

pub mod wire;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gritty_git::{
    DiffOptions, GitError, LogOptions, MAX_PAGE_SIZE, PageOptions, Repository, ShowOptions,
    StatusOptions, open,
};
use gritty_workspace::WorkspaceRoot;
use tokio::runtime::Handle;
use tokio::task::JoinError;

use crate::operation::{OperationKind, OperationRegistry, OperationSpec};
use crate::protocol::actor::Actor;
use crate::protocol::error::{CoreError, CoreErrorCode};
use crate::protocol::ids::CorrelationId;
use crate::workspace::WorkspaceManager;

use wire::{
    GitDiffRequest, GitDiffResponse, GitLogRequest, GitLogResponse, GitShowRequest,
    GitShowResponse, GitStatusRequest, GitStatusResponse,
};

/// Operation kind of a status that outlived its budget.
pub const STATUS_KIND: &str = "git.status";
/// Operation kind of a diff that outlived its budget.
pub const DIFF_KIND: &str = "git.diff";
/// Operation kind of a log walk that outlived its budget.
pub const LOG_KIND: &str = "git.log";
/// Operation kind of a blob read that outlived its budget.
pub const SHOW_KIND: &str = "git.show";

/// How long a read waits before handing over to an operation.
/// The same 100 ms `workspace.list` uses: long enough that an ordinary
/// repository answers inline, short enough that a slow one never looks frozen.
pub const DEFAULT_GIT_BUDGET: Duration = Duration::from_millis(100);

/// Default commits per page for `git.log`.
///
/// Small, because a history list is read a screen at a time and the next page
/// costs one more call. The walk is also the one read here whose total cost is
/// unbounded by the repository's *current* state — it is bounded by its whole
/// history — so a modest default matters more than it does for a status.
pub const DEFAULT_LOG_PAGE_SIZE: u32 = 50;

/// Default files per page for `git.diff`.
///
/// Far below [`MAX_PAGE_SIZE`], deliberately: a status entry is a path and two
/// enums, while a diff entry carries hunks, so the same page count is three
/// orders of magnitude more payload. A caller that wants more can ask.
pub const DEFAULT_DIFF_PAGE_SIZE: u32 = 100;

/// Reads the repository at the open workspace's root.
pub struct GitService {
    workspaces: Arc<WorkspaceManager>,
    registry: Arc<OperationRegistry>,
    handle: Handle,
    /// Tests slow the blocking read down so the budget hand-over and its
    /// cancellation can be exercised deterministically, rather than by racing
    /// a real repository against a short budget. The same hook
    /// `WorkspaceManager::listing_delay` exists for, and for the same reason.
    #[cfg(test)]
    read_delay: std::sync::Mutex<Duration>,
}

impl GitService {
    pub fn new(
        workspaces: Arc<WorkspaceManager>,
        registry: Arc<OperationRegistry>,
        handle: Handle,
    ) -> Arc<Self> {
        Arc::new(Self {
            workspaces,
            registry,
            handle,
            #[cfg(test)]
            read_delay: std::sync::Mutex::new(Duration::ZERO),
        })
    }

    #[cfg(test)]
    fn delay(&self) -> Duration {
        *self
            .read_delay
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `git.status`. See the module documentation for the budget hand-over.
    pub async fn status(
        self: &Arc<Self>,
        request: GitStatusRequest,
        actor: Actor,
        correlation: CorrelationId,
        budget: Duration,
    ) -> Result<GitStatusResponse, CoreError> {
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        let page = page_options(request.cursor, request.page_size, MAX_PAGE_SIZE as u32)?;
        let Some(repository) = self.open_root(&root).await? else {
            return Ok(GitStatusResponse::NotARepository);
        };
        let options = StatusOptions {
            page,
            include_ignored: request.include_ignored.unwrap_or(false),
        };

        let interrupt = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let delay = self.delay();
        let mut job = {
            let interrupt = Arc::clone(&interrupt);
            self.handle.spawn_blocking(move || {
                #[cfg(test)]
                std::thread::sleep(delay);
                gritty_git::status(&repository, &options, &interrupt)
            })
        };
        match tokio::time::timeout(budget, &mut job).await {
            Ok(joined) => Ok(GitStatusResponse::Ready {
                repository: Box::new(finish(joined, "git status")?),
            }),
            Err(_elapsed) => {
                tracing::debug!(
                    workspace = %workspace.id,
                    budget_ms = budget.as_millis() as u64,
                    "git status exceeded its budget; continuing as an operation"
                );
                let spec = OperationSpec {
                    kind: OperationKind::new(STATUS_KIND),
                    actor,
                    workspace_id: Some(workspace.id),
                    correlation_id: correlation,
                    durable: false,
                };
                let operation = self.registry.start(spec, move |ctx| async move {
                    // Stops the blocking read when the body finishes, is
                    // cancelled, or is aborted at an await point.
                    let _stop = InterruptOnDrop(interrupt);
                    tokio::select! {
                        biased;
                        () = ctx.cancelled() => Err(CoreError::cancelled("git status cancelled")),
                        joined = &mut job => {
                            let status = finish(joined, "git status")?;
                            Ok(Some(serde_json::to_value(status)?))
                        }
                    }
                });
                Ok(GitStatusResponse::Pending {
                    operation: Box::new(operation),
                })
            }
        }
    }

    /// `git.diff`. See the module documentation for the budget hand-over.
    pub async fn diff(
        self: &Arc<Self>,
        request: GitDiffRequest,
        actor: Actor,
        correlation: CorrelationId,
        budget: Duration,
    ) -> Result<GitDiffResponse, CoreError> {
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        let page = page_options(request.cursor, request.page_size, DEFAULT_DIFF_PAGE_SIZE)?;
        let Some(repository) = self.open_root(&root).await? else {
            return Ok(GitDiffResponse::NotARepository);
        };
        let options = DiffOptions {
            page,
            target: request.target.clone(),
            path: request.path.clone(),
            limits: Default::default(),
        };

        let interrupt = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let delay = self.delay();
        let mut job = {
            let interrupt = Arc::clone(&interrupt);
            self.handle.spawn_blocking(move || {
                #[cfg(test)]
                std::thread::sleep(delay);
                gritty_git::diff(&repository, &options, &interrupt)
            })
        };
        match tokio::time::timeout(budget, &mut job).await {
            Ok(joined) => Ok(GitDiffResponse::Ready {
                diff: Box::new(finish(joined, "git diff")?),
            }),
            Err(_elapsed) => {
                tracing::debug!(
                    workspace = %workspace.id,
                    budget_ms = budget.as_millis() as u64,
                    "git diff exceeded its budget; continuing as an operation"
                );
                let spec = OperationSpec {
                    kind: OperationKind::new(DIFF_KIND),
                    actor,
                    workspace_id: Some(workspace.id),
                    correlation_id: correlation,
                    durable: false,
                };
                let operation = self.registry.start(spec, move |ctx| async move {
                    let _stop = InterruptOnDrop(interrupt);
                    tokio::select! {
                        biased;
                        () = ctx.cancelled() => Err(CoreError::cancelled("git diff cancelled")),
                        joined = &mut job => {
                            let diff = finish(joined, "git diff")?;
                            Ok(Some(serde_json::to_value(diff)?))
                        }
                    }
                });
                Ok(GitDiffResponse::Pending {
                    operation: Box::new(operation),
                })
            }
        }
    }

    /// `git.log`. See the module documentation for the budget hand-over.
    pub async fn log(
        self: &Arc<Self>,
        request: GitLogRequest,
        actor: Actor,
        correlation: CorrelationId,
        budget: Duration,
    ) -> Result<GitLogResponse, CoreError> {
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        let page = page_options(request.cursor, request.page_size, DEFAULT_LOG_PAGE_SIZE)?;
        let Some(repository) = self.open_root(&root).await? else {
            return Ok(GitLogResponse::NotARepository);
        };
        let options = LogOptions {
            page,
            start: request.start.clone(),
            path: request.path.clone(),
        };

        let interrupt = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let delay = self.delay();
        let mut job = {
            let interrupt = Arc::clone(&interrupt);
            self.handle.spawn_blocking(move || {
                #[cfg(test)]
                std::thread::sleep(delay);
                gritty_git::log(&repository, &options, &interrupt)
            })
        };
        match tokio::time::timeout(budget, &mut job).await {
            Ok(joined) => Ok(GitLogResponse::Ready {
                page: Box::new(finish(joined, "git log")?),
            }),
            Err(_elapsed) => {
                tracing::debug!(
                    workspace = %workspace.id,
                    budget_ms = budget.as_millis() as u64,
                    "git log exceeded its budget; continuing as an operation"
                );
                let spec = OperationSpec {
                    kind: OperationKind::new(LOG_KIND),
                    actor,
                    workspace_id: Some(workspace.id),
                    correlation_id: correlation,
                    durable: false,
                };
                let operation = self.registry.start(spec, move |ctx| async move {
                    let _stop = InterruptOnDrop(interrupt);
                    tokio::select! {
                        biased;
                        () = ctx.cancelled() => Err(CoreError::cancelled("git log cancelled")),
                        joined = &mut job => {
                            let page = finish(joined, "git log")?;
                            Ok(Some(serde_json::to_value(page)?))
                        }
                    }
                });
                Ok(GitLogResponse::Pending {
                    operation: Box::new(operation),
                })
            }
        }
    }

    /// `git.show`. See the module documentation for the budget hand-over.
    pub async fn show(
        self: &Arc<Self>,
        request: GitShowRequest,
        actor: Actor,
        correlation: CorrelationId,
        budget: Duration,
    ) -> Result<GitShowResponse, CoreError> {
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        let Some(repository) = self.open_root(&root).await? else {
            return Ok(GitShowResponse::NotARepository);
        };
        let options = ShowOptions {
            path: request.path.clone(),
            revision: request.revision.clone(),
            max_bytes: None,
        };

        let interrupt = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let delay = self.delay();
        let mut job = {
            let interrupt = Arc::clone(&interrupt);
            self.handle.spawn_blocking(move || {
                #[cfg(test)]
                std::thread::sleep(delay);
                gritty_git::show(&repository, &options, &interrupt)
            })
        };
        match tokio::time::timeout(budget, &mut job).await {
            Ok(joined) => Ok(GitShowResponse::Ready {
                blob: Box::new(finish(joined, "git show")?),
            }),
            Err(_elapsed) => {
                tracing::debug!(
                    workspace = %workspace.id,
                    budget_ms = budget.as_millis() as u64,
                    "git show exceeded its budget; continuing as an operation"
                );
                let spec = OperationSpec {
                    kind: OperationKind::new(SHOW_KIND),
                    actor,
                    workspace_id: Some(workspace.id),
                    correlation_id: correlation,
                    durable: false,
                };
                let operation = self.registry.start(spec, move |ctx| async move {
                    let _stop = InterruptOnDrop(interrupt);
                    tokio::select! {
                        biased;
                        () = ctx.cancelled() => Err(CoreError::cancelled("git show cancelled")),
                        joined = &mut job => {
                            let blob = finish(joined, "git show")?;
                            Ok(Some(serde_json::to_value(blob)?))
                        }
                    }
                });
                Ok(GitShowResponse::Pending {
                    operation: Box::new(operation),
                })
            }
        }
    }

    /// Open the workspace root, or `Ok(None)` when it is not a repository.
    ///
    /// Runs on the blocking pool because it reads `.git/config` and the ref
    /// store, but without a budget: it is a handful of small file reads and
    /// does not traverse anything. Doing it before the operation split is what
    /// lets `NotARepository` be an immediate answer rather than an operation
    /// that completes with "there was nothing to do".
    async fn open_root(&self, root: &Arc<WorkspaceRoot>) -> Result<Option<Repository>, CoreError> {
        let path = root.path().to_path_buf();
        let opened = self
            .handle
            .spawn_blocking(move || open(&path))
            .await
            .map_err(|join| CoreError::internal(format!("git open task failed: {join}")))?;
        match opened {
            Ok(repository) => Ok(Some(repository)),
            Err(GitError::NotARepository { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

/// Sets the interrupt flag when dropped, so a blocking `gix` read stops as
/// soon as nothing is waiting for it. `spawn_blocking` cannot be aborted, so
/// this flag is the only thing that ends the work early.
struct InterruptOnDrop(Arc<AtomicBool>);

impl Drop for InterruptOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn finish<T>(joined: Result<Result<T, GitError>, JoinError>, what: &str) -> Result<T, CoreError> {
    joined
        .map_err(|join| CoreError::internal(format!("{what} task failed: {join}")))?
        .map_err(CoreError::from)
}

fn page_options(
    cursor: Option<String>,
    page_size: Option<u32>,
    default: u32,
) -> Result<PageOptions, CoreError> {
    let page_size = match page_size {
        None => default as usize,
        Some(size) if (1..=MAX_PAGE_SIZE as u32).contains(&size) => size as usize,
        Some(_) => {
            return Err(CoreError::invalid_input(
                "pageSize",
                format!("must be between 1 and {MAX_PAGE_SIZE}"),
            ));
        }
    };
    Ok(PageOptions { page_size, cursor })
}

impl From<GitError> for CoreError {
    fn from(error: GitError) -> Self {
        match error {
            // Reached only when a repository disappears between the open and
            // the read; the ordinary case is answered as a value before this.
            GitError::NotARepository { ref path } => {
                Self::not_found(format!("{} is not a git repository", path.display()))
            }
            GitError::UnknownRevision { ref spec } => Self::invalid_input(
                "target",
                format!("no revision named {spec} in this repository"),
            ),
            GitError::NotATree { ref spec } => {
                Self::invalid_input("target", format!("{spec} does not name a commit or a tree"))
            }
            // `NotFound` rather than `InvalidInput`: the request was well
            // formed and the repository simply does not have that path at that
            // revision, which is usually the true and interesting answer.
            GitError::NotFoundAtRevision { ref path, ref spec } => {
                Self::not_found(format!("{path} does not exist at {spec}"))
            }
            GitError::Cancelled => Self::cancelled("git read cancelled"),
            // A corrupt object, an unreadable pack or a stale lock held by
            // another process: `Io`, never a panic and never partial data
            // presented as complete (T100's failure table).
            ref other @ (GitError::Io { .. } | GitError::Repository { .. }) => {
                Self::new(CoreErrorCode::Io, other.to_string()).with_retryable(other.is_retryable())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gritty_git::fixtures::{self, TestRepo};
    use tokio::time::timeout;

    use super::*;
    use crate::event::testing::RecordingSink;
    use crate::event::{CoreEvent, EventBus};
    use crate::operation::OperationStatus;
    use crate::protocol::ids::WorkspaceId;
    use crate::workspace::UnsavedPolicy;

    struct Harness {
        service: Arc<GitService>,
        workspaces: Arc<WorkspaceManager>,
        registry: Arc<OperationRegistry>,
        rx: tokio::sync::mpsc::UnboundedReceiver<crate::event::EventEnvelope>,
    }

    fn harness() -> Harness {
        let (sink, rx) = RecordingSink::with_channel();
        let events = EventBus::new(sink);
        let registry = OperationRegistry::new(Handle::current(), events.clone());
        let workspaces = WorkspaceManager::new(Handle::current(), events, registry.clone());
        let service = GitService::new(workspaces.clone(), registry.clone(), Handle::current());
        Harness {
            service,
            workspaces,
            registry,
            rx,
        }
    }

    impl Harness {
        fn open(&self, path: &std::path::Path) -> crate::workspace::Workspace {
            self.workspaces
                .open(
                    path,
                    UnsavedPolicy::Refuse,
                    Actor::User,
                    CorrelationId::new(),
                )
                .expect("open workspace")
        }

        /// Make every read take at least `delay` before it touches the
        /// repository, so the budget hand-over is a decision rather than a
        /// race against a real filesystem.
        fn slow_reads(&self, delay: Duration) {
            *self
                .service
                .read_delay
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = delay;
        }

        /// Wait for the `OperationCompleted` event, which is where the result
        /// of a budget hand-over actually travels: the `Operation` snapshot
        /// carries status, not payload.
        async fn completed(&mut self) -> crate::event::OperationCompletedPayload {
            let envelope = timeout(GENEROUS, async {
                loop {
                    let envelope = self.rx.recv().await.expect("event stream open");
                    if envelope.event.name() == "OperationCompleted" {
                        return envelope;
                    }
                }
            })
            .await
            .expect("the operation completed in time");
            let CoreEvent::OperationCompleted(payload) = envelope.event else {
                unreachable!("filtered by name");
            };
            payload
        }
    }

    fn status_request(workspace_id: WorkspaceId) -> GitStatusRequest {
        GitStatusRequest {
            workspace_id,
            cursor: None,
            page_size: None,
            include_ignored: None,
        }
    }

    fn diff_request(workspace_id: WorkspaceId) -> GitDiffRequest {
        GitDiffRequest {
            workspace_id,
            target: None,
            path: None,
            cursor: None,
            page_size: None,
        }
    }

    /// Long enough that a real read of a fixture repository always finishes
    /// inline, so a test that wants `Ready` gets it without racing anything.
    const GENEROUS: Duration = Duration::from_secs(30);
    /// Paired with [`Harness::slow_reads`]: the budget the read is made to
    /// exceed on purpose.
    const SHORT_BUDGET: Duration = Duration::from_millis(10);

    #[tokio::test]
    async fn status_reports_the_repository_at_the_open_workspace_root() {
        let h = harness();
        let repo = fixtures::clean();
        repo.write("changed.txt", "x\n");
        let workspace = h.open(repo.path());

        let response = h
            .service
            .status(
                status_request(workspace.id),
                Actor::User,
                CorrelationId::new(),
                GENEROUS,
            )
            .await
            .expect("status");
        let GitStatusResponse::Ready { repository } = response else {
            panic!("expected a ready status, got {response:?}");
        };
        assert_eq!(repository.branch.as_deref(), Some("main"));
        assert_eq!(repository.total, 1);
    }

    /// `security.md`: nothing is filtered, and a folder that is simply not a
    /// repository is a value the UI renders, not a red error envelope.
    #[tokio::test]
    async fn a_plain_folder_is_not_a_repository_rather_than_an_error() {
        let h = harness();
        let dir = TestRepo::plain_directory();
        let workspace = h.open(dir.path());

        let status = h
            .service
            .status(
                status_request(workspace.id.clone()),
                Actor::User,
                CorrelationId::new(),
                GENEROUS,
            )
            .await
            .expect("status");
        assert!(matches!(status, GitStatusResponse::NotARepository));

        let diff = h
            .service
            .diff(
                diff_request(workspace.id),
                Actor::User,
                CorrelationId::new(),
                GENEROUS,
            )
            .await
            .expect("diff");
        assert!(matches!(diff, GitDiffResponse::NotARepository));
    }

    #[tokio::test]
    async fn a_mismatched_workspace_id_is_not_found() {
        let h = harness();
        let repo = fixtures::clean();
        h.open(repo.path());

        let error = h
            .service
            .status(
                status_request(WorkspaceId::new()),
                Actor::User,
                CorrelationId::new(),
                GENEROUS,
            )
            .await
            .expect_err("not found");
        assert_eq!(error.code, CoreErrorCode::NotFound);
    }

    #[tokio::test]
    async fn an_unknown_revision_is_invalid_input_naming_the_field() {
        let h = harness();
        let repo = fixtures::clean();
        let workspace = h.open(repo.path());

        let error = h
            .service
            .diff(
                GitDiffRequest {
                    target: Some("no-such-revision".to_owned()),
                    ..diff_request(workspace.id)
                },
                Actor::User,
                CorrelationId::new(),
                GENEROUS,
            )
            .await
            .expect_err("invalid input");
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert_eq!(
            error.details.as_ref().and_then(|d| d["field"].as_str()),
            Some("target")
        );
    }

    #[tokio::test]
    async fn an_out_of_range_page_size_is_rejected_before_the_repository_is_touched() {
        let h = harness();
        let dir = TestRepo::plain_directory();
        let workspace = h.open(dir.path());

        let error = h
            .service
            .status(
                GitStatusRequest {
                    page_size: Some(0),
                    ..status_request(workspace.id)
                },
                Actor::User,
                CorrelationId::new(),
                GENEROUS,
            )
            .await
            .expect_err("invalid input");
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
    }

    /// The `workspace.list` hand-off, reused: a read that outlives its budget
    /// becomes a cancellable operation and no new event family is invented for
    /// it.
    #[tokio::test]
    async fn a_status_that_outlives_its_budget_becomes_an_operation() {
        let mut h = harness();
        let repo = fixtures::clean();
        repo.write("changed.txt", "x\n");
        let workspace = h.open(repo.path());
        h.slow_reads(Duration::from_millis(200));

        let response = h
            .service
            .status(
                status_request(workspace.id.clone()),
                Actor::User,
                CorrelationId::new(),
                SHORT_BUDGET,
            )
            .await
            .expect("status");
        let GitStatusResponse::Pending { operation } = response else {
            panic!("expected a pending status, got {response:?}");
        };
        assert_eq!(operation.kind.as_str(), STATUS_KIND);
        assert_eq!(operation.workspace_id, Some(workspace.id));

        // The page arrives on `OperationCompleted`, reusing the existing event
        // family: T100 introduces none of its own.
        let payload = h.completed().await;
        assert_eq!(payload.operation.id, operation.id);
        assert_eq!(payload.operation.status, OperationStatus::Completed);
        let page: gritty_git::RepositoryStatus =
            serde_json::from_value(payload.result.expect("a result"))
                .expect("the completion result is the status");
        assert_eq!(page.total, 1);
    }

    #[tokio::test]
    async fn cancelling_a_pending_status_ends_it_cancelled() {
        let mut h = harness();
        let repo = fixtures::clean();
        repo.write("changed.txt", "x\n");
        let workspace = h.open(repo.path());
        // Long enough that the cancellation cannot be a coincidence, short
        // enough that dropping the runtime does not wait on it: a
        // `spawn_blocking` job cannot be aborted, only abandoned.
        h.slow_reads(Duration::from_secs(2));

        let response = h
            .service
            .status(
                status_request(workspace.id),
                Actor::User,
                CorrelationId::new(),
                SHORT_BUDGET,
            )
            .await
            .expect("status");
        let GitStatusResponse::Pending { operation } = response else {
            panic!("expected a pending status, got {response:?}");
        };
        h.registry.cancel(&operation.id).expect("cancel");

        // The body abandons the read rather than waiting the 30 seconds out.
        let started = std::time::Instant::now();
        let envelope = timeout(GENEROUS, async {
            loop {
                let envelope = h.rx.recv().await.expect("event stream open");
                if envelope.event.name() == "OperationCancelled" {
                    return envelope;
                }
            }
        })
        .await
        .expect("the operation ended in time");
        let CoreEvent::OperationCancelled(payload) = envelope.event else {
            unreachable!("filtered by name");
        };
        assert_eq!(payload.operation.id, operation.id);
        assert_eq!(payload.operation.status, OperationStatus::Cancelled);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "cancellation waited for the read to finish ({:?})",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn a_diff_that_outlives_its_budget_becomes_an_operation() {
        let mut h = harness();
        let repo = fixtures::clean();
        repo.write("README.md", "changed\n");
        let workspace = h.open(repo.path());
        h.slow_reads(Duration::from_millis(200));

        let response = h
            .service
            .diff(
                diff_request(workspace.id),
                Actor::User,
                CorrelationId::new(),
                SHORT_BUDGET,
            )
            .await
            .expect("diff");
        let GitDiffResponse::Pending { operation } = response else {
            panic!("expected a pending diff, got {response:?}");
        };
        assert_eq!(operation.kind.as_str(), DIFF_KIND);

        let payload = h.completed().await;
        assert_eq!(payload.operation.id, operation.id);
        assert_eq!(payload.operation.status, OperationStatus::Completed);
        let summary: gritty_git::DiffSummary =
            serde_json::from_value(payload.result.expect("a result"))
                .expect("the completion result is the diff");
        assert_eq!(summary.total, 1);
    }

    /// Hard rule 5, checked against what actually crosses the wire rather than
    /// against the Rust types: no field of a serialized response is a string
    /// carrying a whole diff or status blob.
    #[tokio::test]
    async fn no_response_field_carries_a_whole_diff_as_one_string() {
        let h = harness();
        let repo = fixtures::clean();
        repo.write("README.md", "one\ntwo\nthree\n");
        let workspace = h.open(repo.path());

        let diff = h
            .service
            .diff(
                diff_request(workspace.id.clone()),
                Actor::User,
                CorrelationId::new(),
                GENEROUS,
            )
            .await
            .expect("diff");
        let status = h
            .service
            .status(
                status_request(workspace.id),
                Actor::User,
                CorrelationId::new(),
                GENEROUS,
            )
            .await
            .expect("status");

        for value in [
            serde_json::to_value(&diff).expect("serialize diff"),
            serde_json::to_value(&status).expect("serialize status"),
        ] {
            let mut strings = Vec::new();
            collect_strings(&value, &mut strings);
            for found in strings {
                assert!(
                    !found.contains('\n'),
                    "a response field carries a multi-line blob: {found:?}"
                );
                assert!(
                    !found.starts_with("diff --git") && !found.starts_with("@@"),
                    "a response field carries unified-diff text: {found:?}"
                );
            }
        }
    }

    fn collect_strings(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::String(text) => out.push(text.clone()),
            serde_json::Value::Array(items) => {
                for item in items {
                    collect_strings(item, out);
                }
            }
            serde_json::Value::Object(fields) => {
                for field in fields.values() {
                    collect_strings(field, out);
                }
            }
            _ => {}
        }
    }
}
