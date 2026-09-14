//! Workspace lifecycle (T020): one open workspace at a time, identified by a
//! [`WorkspaceId`], announced through `WorkspaceOpened` / `WorkspaceClosed`,
//! and listed one directory level per command.
//!
//! The filesystem work lives in `gritty-workspace`; this module owns identity,
//! events, error mapping and the decision to run a listing as an operation:
//! `list` runs the listing on the blocking pool and waits up to a budget
//! (`DEFAULT_LIST_BUDGET`); when the directory is not done by then the same
//! job continues as a cancellable `workspace.list` operation whose completion
//! event carries the page. Wire types are exported to TypeScript from here.

pub mod files;
pub mod find;
pub mod journal;
pub mod recents;
pub mod watch;

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use gritty_workspace::{
    DirectoryPage, ListOptions, Listing, MAX_PAGE_SIZE, WorkspaceError, WorkspacePath,
    WorkspaceRoot, list_directory,
};
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::task::JoinError;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::event::{
    CoreEvent, EventBus, EventOrigin, WorkspaceClosedPayload, WorkspaceOpenedPayload,
};
use crate::operation::{Operation, OperationKind, OperationRegistry, OperationSpec};
use crate::protocol::actor::Actor;
use crate::protocol::error::{CoreError, CoreErrorCode};
use crate::protocol::ids::{CorrelationId, WorkspaceId};
use crate::protocol::time::Timestamp;

/// Operation kind of a listing that outlived its budget.
pub const LIST_KIND: &str = "workspace.list";
/// How long `workspace.list` waits before handing the listing over to an operation.
pub const DEFAULT_LIST_BUDGET: Duration = Duration::from_millis(100);

/// An open workspace as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: WorkspaceId,
    /// Canonical absolute root in the platform's native form. Display only:
    /// commands address files with workspace-relative `WorkspacePath`s.
    pub root: String,
    /// Last component of the root (the whole root for a drive root).
    pub name: String,
    pub opened_at: Timestamp,
}

/// `workspace.open`. Without `path` the shell shows the native directory picker.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OpenWorkspaceRequest {
    /// Absolute native path of the directory to open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<String>,
    /// Replace the current workspace even when it has documents with unsaved
    /// changes (T030). Without it the command fails with `Conflict`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub force: Option<bool>,
}

/// `workspace.close`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CloseWorkspaceRequest {
    pub workspace_id: WorkspaceId,
    /// Close even when documents have unsaved changes (T030). Without it the
    /// command fails with `Conflict` listing them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub force: Option<bool>,
}

/// `workspace.list`: one level of `path` (`""` is the root).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ListDirectoryRequest {
    pub workspace_id: WorkspaceId,
    pub path: WorkspacePath,
    /// Continue a truncated listing (`DirectoryPage.cursor`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cursor: Option<String>,
    /// Entries per page, `1..=5000`; defaults to 5000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub page_size: Option<u32>,
}

/// What `workspace.list` returns: the page when the directory was read within
/// the budget, otherwise the operation that will complete with the page in
/// `OperationCompleted.result` (or end `CANCELLED` / `FAILED`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ListDirectoryResponse {
    Ready { page: DirectoryPage },
    Pending { operation: Box<Operation> },
}

/// One entry of the workspace history (T049), for the startup screen.
///
/// Not a [`Workspace`]: nothing here is open, so there is no [`WorkspaceId`].
/// The key is the canonical root, which is what the history is stored under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecentWorkspace {
    /// Canonical absolute root in the platform's native form. Pass it back to
    /// `workspace.open` or `workspace.forget`.
    pub root: String,
    /// The same path shortened for display: `~` for the user's home directory,
    /// otherwise identical to `root`. The renderer has no filesystem and does
    /// not know `$HOME`, so the shortening happens here.
    pub display_root: String,
    /// Last component of the root (the whole root for a drive root).
    pub name: String,
    pub last_opened_at: Timestamp,
    /// Whether the root is still a directory. A `false` entry is shown as
    /// unavailable rather than hidden: the user chose to open it once, and
    /// silently dropping it would explain nothing.
    pub exists: bool,
}

/// `workspace.recents`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecentWorkspacesRequest {
    /// How many entries to return, `1..=MAX_RECENTS`; defaults to
    /// [`DEFAULT_RECENTS`]. Every entry costs one `stat`, so the caller says
    /// how many it will actually show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub limit: Option<u32>,
}

/// `workspace.forget`: drop one root from the history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ForgetWorkspaceRequest {
    /// Canonical root, as returned by `workspace.recents`.
    pub root: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkspaceClosedReason {
    /// `workspace.close`.
    User,
    /// Another workspace was opened (one at a time for now).
    Replaced,
    /// The root directory disappeared while the workspace was open.
    RootMissing,
}

/// What to do with unsaved document changes when a user action would close
/// the workspace they belong to (T030).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsavedPolicy {
    /// Fail with `Conflict` and change nothing (the default).
    Refuse,
    /// The UI confirmed that unsaved changes may be dropped (`force: true`).
    Discard,
}

impl UnsavedPolicy {
    pub fn from_force(force: Option<bool>) -> Self {
        if force == Some(true) {
            Self::Discard
        } else {
            Self::Refuse
        }
    }
}

/// Installed by the document manager (T030) so that closing a workspace never
/// silently drops unsaved buffers. Both hooks run under the workspace lock;
/// implementations must not call back into the manager.
pub trait WorkspaceCloseGuard: Send + Sync {
    /// Runs before a user-initiated close (`USER`) or replacement
    /// (`REPLACED`). An `Err` (normally `Conflict`) vetoes it; nothing changes.
    fn before_close(
        &self,
        workspace: &Workspace,
        reason: WorkspaceClosedReason,
        policy: UnsavedPolicy,
    ) -> Result<(), CoreError>;

    /// Runs after `WorkspaceClosed` was emitted, whatever the reason.
    fn after_close(
        &self,
        workspace: &Workspace,
        reason: WorkspaceClosedReason,
        actor: &Actor,
        correlation: Option<&CorrelationId>,
    );
}

struct OpenWorkspace {
    workspace: Workspace,
    root: Arc<WorkspaceRoot>,
}

pub struct WorkspaceManager {
    current: Mutex<Option<OpenWorkspace>>,
    events: Arc<EventBus>,
    registry: Arc<OperationRegistry>,
    handle: Handle,
    close_guard: Mutex<Option<Weak<dyn WorkspaceCloseGuard>>>,
    /// The watch of the open workspace, if one could be started (T070). At
    /// most one, matching the single-open-workspace model; dropping it stops
    /// the watch, which is why every close path goes through `emit_closed`.
    watch: Mutex<Option<gritty_workspace::Watch>>,
    /// Handed to the watch's callback so it can reach the close path without
    /// keeping the manager alive.
    me: Weak<Self>,
    /// Tests slow the blocking listing down so the budget hand-over and its
    /// cancellation can be exercised deterministically.
    #[cfg(test)]
    listing_delay: Mutex<Duration>,
    /// The same hook for `find_files` (T130-C), and for the same reason: a
    /// ranked walk over a temporary tree finishes in microseconds, so
    /// cancelling one mid-flight would be a race rather than a test.
    #[cfg(test)]
    pub(super) find_delay: Mutex<Duration>,
}

impl WorkspaceManager {
    pub fn new(
        handle: Handle,
        events: Arc<EventBus>,
        registry: Arc<OperationRegistry>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            current: Mutex::new(None),
            events,
            registry,
            handle,
            close_guard: Mutex::new(None),
            watch: Mutex::new(None),
            me: me.clone(),
            #[cfg(test)]
            listing_delay: Mutex::new(Duration::ZERO),
            #[cfg(test)]
            find_delay: Mutex::new(Duration::ZERO),
        })
    }

    /// Install the guard consulted when a workspace closes (one per process,
    /// set by the composition root; a `Weak` so the two managers do not keep
    /// each other alive).
    pub fn set_close_guard(&self, guard: Weak<dyn WorkspaceCloseGuard>) {
        *self
            .close_guard
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(guard);
    }

    fn guard(&self) -> Option<Arc<dyn WorkspaceCloseGuard>> {
        self.close_guard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade)
    }

    /// Open `path` (absolute). Reopening the current root returns it unchanged;
    /// opening another root closes the current one first (`REPLACED`), which
    /// the close guard may veto (`Conflict`) under `UnsavedPolicy::Refuse`. A
    /// failure leaves the current workspace untouched.
    pub fn open(
        &self,
        path: &Path,
        policy: UnsavedPolicy,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Workspace, CoreError> {
        let root = WorkspaceRoot::open(path)?;
        let mut current = self.lock();
        if let Some(open) = current.as_ref()
            && open.root.path() == root.path()
        {
            return Ok(open.workspace.clone());
        }
        if let Some(previous) = current.as_ref()
            && let Some(guard) = self.guard()
        {
            guard.before_close(&previous.workspace, WorkspaceClosedReason::Replaced, policy)?;
        }
        if let Some(previous) = current.take() {
            self.emit_closed(
                &previous,
                WorkspaceClosedReason::Replaced,
                actor.clone(),
                Some(correlation.clone()),
            );
        }
        let workspace = Workspace {
            id: WorkspaceId::new(),
            root: root.path().to_string_lossy().into_owned(),
            name: root.name(),
            opened_at: Timestamp::now(),
        };
        tracing::info!(workspace = %workspace.id, root = %workspace.root, "workspace opened");
        self.events.emit(
            &EventOrigin {
                workspace_id: Some(workspace.id.clone()),
                correlation_id: Some(correlation),
                actor,
            },
            CoreEvent::WorkspaceOpened(WorkspaceOpenedPayload {
                workspace: workspace.clone(),
            }),
        );
        let root = Arc::new(root);
        // Started after the workspace is announced, so nothing the watch says
        // can arrive before `WorkspaceOpened`; a backend that refuses leaves
        // `None` here and every on-demand check stays as correct as it was.
        *self.watch.lock().unwrap_or_else(PoisonError::into_inner) =
            self.start_watch(&workspace, &root);
        *current = Some(OpenWorkspace {
            workspace: workspace.clone(),
            root,
        });
        Ok(workspace)
    }

    /// Close the workspace with this id; `NotFound` when it is not the open
    /// one, `Conflict` when the close guard refuses (unsaved documents under
    /// `UnsavedPolicy::Refuse`).
    pub fn close(
        &self,
        id: &WorkspaceId,
        policy: UnsavedPolicy,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Workspace, CoreError> {
        let mut current = self.lock();
        if !current
            .as_ref()
            .is_some_and(|open| open.workspace.id == *id)
        {
            return Err(CoreError::not_found(format!("workspace {id} is not open")));
        }
        if let Some(open) = current.as_ref()
            && let Some(guard) = self.guard()
        {
            guard.before_close(&open.workspace, WorkspaceClosedReason::User, policy)?;
        }
        let open = current.take().expect("checked above");
        self.emit_closed(&open, WorkspaceClosedReason::User, actor, Some(correlation));
        Ok(open.workspace)
    }

    /// The open workspace, or `None`. A root that disappeared closes the
    /// workspace (`ROOT_MISSING`) and yields `None`.
    /// The open workspace's id and root, for the callers that need the root
    /// itself rather than the display string on [`Workspace`] — classifying a
    /// dropped path against it, for one.
    pub fn current_open(&self) -> Option<(WorkspaceId, Arc<WorkspaceRoot>)> {
        self.lock()
            .as_ref()
            .map(|open| (open.workspace.id.clone(), open.root.clone()))
    }

    pub fn current(&self, correlation: &CorrelationId) -> Option<Workspace> {
        let mut current = self.lock();
        let present = current.as_ref().is_some_and(|open| open.root.is_present());
        if !present && let Some(gone) = current.take() {
            self.emit_closed(
                &gone,
                WorkspaceClosedReason::RootMissing,
                Actor::System,
                Some(correlation.clone()),
            );
        }
        current.as_ref().map(|open| open.workspace.clone())
    }

    /// List one directory level. Runs off the caller's thread; see the module
    /// documentation for the budget / operation hand-over.
    pub async fn list(
        self: &Arc<Self>,
        request: ListDirectoryRequest,
        actor: Actor,
        correlation: CorrelationId,
        budget: Duration,
    ) -> Result<ListDirectoryResponse, CoreError> {
        let (workspace, root) = self.require(&request.workspace_id, &correlation)?;
        let page_size = match request.page_size {
            None => MAX_PAGE_SIZE,
            Some(size) if (1..=MAX_PAGE_SIZE).contains(&(size as usize)) => size as usize,
            Some(_) => {
                return Err(CoreError::invalid_input(
                    "pageSize",
                    format!("must be between 1 and {MAX_PAGE_SIZE}"),
                ));
            }
        };
        let options = ListOptions {
            page_size,
            cursor: request.cursor.clone(),
        };
        let token = CancellationToken::new();
        #[cfg(test)]
        let delay = *self
            .listing_delay
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut job = {
            let root = Arc::clone(&root);
            let path = request.path.clone();
            let token = token.clone();
            self.handle.spawn_blocking(move || {
                #[cfg(test)]
                std::thread::sleep(delay);
                list_directory(&root, &path, &options, &|| token.is_cancelled())
            })
        };
        match tokio::time::timeout(budget, &mut job).await {
            Ok(joined) => Ok(ListDirectoryResponse::Ready {
                page: finish_listing(joined, &request.path)?,
            }),
            Err(_elapsed) => {
                tracing::debug!(
                    workspace = %workspace.id,
                    path = %request.path,
                    budget_ms = budget.as_millis() as u64,
                    "listing exceeded its budget; continuing as an operation"
                );
                let spec = OperationSpec {
                    kind: OperationKind::new(LIST_KIND),
                    actor,
                    workspace_id: Some(workspace.id),
                    correlation_id: correlation,
                    durable: false,
                };
                let path = request.path.clone();
                let operation = self.registry.start(spec, move |ctx| async move {
                    // Stops the blocking job when the body finishes, is
                    // cancelled, or is aborted at an await point.
                    let job_token = CancelOnDrop(token);
                    tokio::select! {
                        biased;
                        () = ctx.cancelled() => {
                            job_token.0.cancel();
                            Err(CoreError::cancelled("directory listing cancelled"))
                        }
                        joined = &mut job => {
                            let page = finish_listing(joined, &path)?;
                            Ok(Some(serde_json::to_value(page)?))
                        }
                    }
                });
                Ok(ListDirectoryResponse::Pending {
                    operation: Box::new(operation),
                })
            }
        }
    }

    /// The open workspace with this id and its root, or `NotFound`. A root that
    /// disappeared closes the workspace (`ROOT_MISSING`) first.
    pub(crate) fn require(
        &self,
        id: &WorkspaceId,
        correlation: &CorrelationId,
    ) -> Result<(Workspace, Arc<WorkspaceRoot>), CoreError> {
        let mut current = self.lock();
        if !current
            .as_ref()
            .is_some_and(|open| open.workspace.id == *id)
        {
            return Err(CoreError::not_found(format!("workspace {id} is not open")));
        }
        let present = current.as_ref().is_some_and(|open| open.root.is_present());
        if !present {
            let gone = current.take().expect("checked above");
            self.emit_closed(
                &gone,
                WorkspaceClosedReason::RootMissing,
                Actor::System,
                Some(correlation.clone()),
            );
            return Err(CoreError::not_found(format!(
                "workspace root {} no longer exists",
                gone.workspace.root
            ))
            .with_details(serde_json::json!({
                "reason": WorkspaceClosedReason::RootMissing,
                "workspaceId": id,
            })));
        }
        let open = current.as_ref().expect("checked above");
        Ok((open.workspace.clone(), Arc::clone(&open.root)))
    }

    fn emit_closed(
        &self,
        open: &OpenWorkspace,
        reason: WorkspaceClosedReason,
        actor: Actor,
        correlation_id: Option<CorrelationId>,
    ) {
        tracing::info!(workspace = %open.workspace.id, ?reason, "workspace closed");
        // Every close — user, replaced, root missing — funnels through here,
        // so stopping the watch here means no path can leave one running.
        // Dropping the handle never joins the pump thread, which is what lets
        // the pump itself reach this (a root that disappeared) without
        // deadlocking against its own close.
        drop(
            self.watch
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        self.events.emit(
            &EventOrigin {
                workspace_id: Some(open.workspace.id.clone()),
                correlation_id: correlation_id.clone(),
                actor: actor.clone(),
            },
            CoreEvent::WorkspaceClosed(WorkspaceClosedPayload {
                workspace_id: open.workspace.id.clone(),
                root: open.workspace.root.clone(),
                reason,
            }),
        );
        if let Some(guard) = self.guard() {
            guard.after_close(&open.workspace, reason, &actor, correlation_id.as_ref());
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<OpenWorkspace>> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn finish_listing(
    joined: Result<Result<Listing, WorkspaceError>, JoinError>,
    path: &WorkspacePath,
) -> Result<DirectoryPage, CoreError> {
    let listing = joined
        .map_err(|join| CoreError::internal(format!("directory listing task failed: {join}")))??;
    tracing::debug!(
        %path,
        entries = listing.page.entries.len(),
        total = listing.page.total,
        truncated = listing.page.truncated,
        directories_read = listing.stats.directories_read,
        elapsed_ms = listing.stats.elapsed.as_millis() as u64,
        "directory listed"
    );
    Ok(listing.page)
}

impl From<WorkspaceError> for CoreError {
    fn from(error: WorkspaceError) -> Self {
        match error {
            WorkspaceError::InvalidPath { field, reason } => Self::invalid_input(field, reason),
            WorkspaceError::NotFound(message) => Self::not_found(message),
            WorkspaceError::PermissionDenied(message) => {
                Self::new(CoreErrorCode::PermissionDenied, message)
            }
            WorkspaceError::NotADirectory(message) => Self::invalid_input("path", message),
            WorkspaceError::NotAFile(message) => Self::invalid_input("path", message),
            // `Conflict`, not `InvalidInput`: the request was well formed and
            // the disk disagreed, which is the same shape a dirty save takes.
            WorkspaceError::AlreadyExists(message) => Self::conflict(message),
            WorkspaceError::TooLarge { path, size, limit } => Self::invalid_input(
                "path",
                format!("{path} is {size} bytes, above the {limit}-byte document limit"),
            )
            .with_details(serde_json::json!({
                "field": "path",
                "reason": "too large",
                "size": size,
                "limit": limit,
            })),
            WorkspaceError::Io { context, source } => {
                let mut mapped = Self::from(source);
                mapped.message = format!("{context}: {}", mapped.message);
                mapped
            }
            // `Io`, as before, with a reason the explorer keys on to offer a
            // permanent delete in place of a generic error (TRASH-FALLBACK).
            WorkspaceError::TrashRefused { context, reason } => {
                Self::new(CoreErrorCode::Io, format!("{context}: {reason}"))
                    .with_details(serde_json::json!({ "kind": "Other", "reason": TRASH_REFUSED }))
            }
            WorkspaceError::Cancelled => Self::cancelled("directory listing cancelled"),
        }
    }
}

/// `details.reason` of an `Io` error the OS trash caused: the one I/O failure
/// a caller can answer, by offering a permanent delete (TRASH-FALLBACK).
pub const TRASH_REFUSED: &str = "TRASH_REFUSED";

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::mpsc;
    use tokio::time::timeout;

    use super::*;
    use crate::event::EventEnvelope;
    use crate::event::testing::RecordingSink;
    use crate::operation::OperationStatus;

    struct Harness {
        manager: Arc<WorkspaceManager>,
        registry: Arc<OperationRegistry>,
        sink: Arc<RecordingSink>,
        rx: mpsc::UnboundedReceiver<EventEnvelope>,
    }

    fn harness() -> Harness {
        let (sink, rx) = RecordingSink::with_channel();
        let events = EventBus::new(sink.clone());
        let registry = OperationRegistry::new(Handle::current(), events.clone());
        let manager = WorkspaceManager::new(Handle::current(), events, registry.clone());
        Harness {
            manager,
            registry,
            sink,
            rx,
        }
    }

    impl Harness {
        async fn next_named(&mut self, name: &str) -> EventEnvelope {
            timeout(Duration::from_secs(5), async {
                loop {
                    let envelope = self.rx.recv().await.expect("event stream open");
                    if envelope.event.name() == name {
                        return envelope;
                    }
                }
            })
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {name}"))
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir()
                .join(format!("gritty-core-ws-{label}-{}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(path.join("src")).unwrap();
            fs::write(path.join("README.md"), "hi").unwrap();
            fs::write(path.join("src/main.rs"), "fn main() {}").unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn list_request(workspace: &Workspace, path: &str) -> ListDirectoryRequest {
        ListDirectoryRequest {
            workspace_id: workspace.id.clone(),
            path: WorkspacePath::parse(path).unwrap(),
            cursor: None,
            page_size: None,
        }
    }

    /// Make every listing take at least `delay` before it touches the disk.
    fn slow_listings(h: &Harness, delay: Duration) {
        *h.manager.listing_delay.lock().unwrap() = delay;
    }

    const SHORT_BUDGET: Duration = Duration::from_millis(10);

    #[tokio::test]
    async fn open_announces_the_workspace_and_current_returns_it() {
        let mut h = harness();
        let dir = TempDir::new("open");
        let correlation = CorrelationId::new();
        let workspace = h
            .manager
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                correlation.clone(),
            )
            .unwrap();
        assert!(workspace.id.is_well_formed());
        assert_eq!(workspace.name, dir.0.file_name().unwrap().to_string_lossy());
        assert!(PathBuf::from(&workspace.root).is_absolute());
        let opened = h.next_named("WorkspaceOpened").await;
        assert_eq!(opened.meta.workspace_id, Some(workspace.id.clone()));
        assert_eq!(opened.meta.correlation_id, Some(correlation));
        let CoreEvent::WorkspaceOpened(payload) = opened.event else {
            panic!("expected WorkspaceOpened");
        };
        assert_eq!(payload.workspace, workspace);
        assert_eq!(h.manager.current(&CorrelationId::new()), Some(workspace));
    }

    #[tokio::test]
    async fn reopening_the_same_root_is_idempotent_and_another_root_replaces_it() {
        let mut h = harness();
        let first = TempDir::new("first");
        let second = TempDir::new("second");
        let a = h
            .manager
            .open(
                &first.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        let again = h
            .manager
            .open(
                &first.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        assert_eq!(a, again);
        let b = h
            .manager
            .open(
                &second.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(
            h.sink.names(),
            vec!["WorkspaceOpened", "WorkspaceClosed", "WorkspaceOpened"]
        );
        let closed = h.next_named("WorkspaceClosed").await;
        let CoreEvent::WorkspaceClosed(payload) = closed.event else {
            panic!("expected WorkspaceClosed");
        };
        assert_eq!(payload.workspace_id, a.id);
        assert_eq!(payload.reason, WorkspaceClosedReason::Replaced);
        assert_eq!(h.manager.current(&CorrelationId::new()), Some(b));
    }

    #[tokio::test]
    async fn open_failures_leave_state_untouched() {
        let h = harness();
        let dir = TempDir::new("failures");
        let workspace = h
            .manager
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        let missing = std::env::temp_dir().join("gritty-core-ws-does-not-exist");
        let error = h
            .manager
            .open(
                &missing,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::NotFound);
        let file = dir.0.join("README.md");
        let error = h
            .manager
            .open(
                &file,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        let error = h
            .manager
            .open(
                Path::new("relative"),
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert_eq!(h.manager.current(&CorrelationId::new()), Some(workspace));
        assert_eq!(h.sink.names(), vec!["WorkspaceOpened"]);
    }

    #[tokio::test]
    async fn close_requires_the_open_workspace_id() {
        let mut h = harness();
        let dir = TempDir::new("close");
        let error = h
            .manager
            .close(
                &WorkspaceId::new(),
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::NotFound);
        let workspace = h
            .manager
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        let error = h
            .manager
            .close(
                &WorkspaceId::new(),
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::NotFound);
        let closed = h
            .manager
            .close(
                &workspace.id,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        assert_eq!(closed, workspace);
        let event = h.next_named("WorkspaceClosed").await;
        let CoreEvent::WorkspaceClosed(payload) = event.event else {
            panic!("expected WorkspaceClosed");
        };
        assert_eq!(payload.reason, WorkspaceClosedReason::User);
        assert_eq!(h.manager.current(&CorrelationId::new()), None);
        let error = h
            .manager
            .close(
                &workspace.id,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::NotFound);
    }

    #[tokio::test]
    async fn listing_within_budget_returns_the_page_directly() {
        let h = harness();
        let dir = TempDir::new("list");
        let workspace = h
            .manager
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        let response = h
            .manager
            .list(
                list_request(&workspace, ""),
                Actor::User,
                CorrelationId::new(),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        let ListDirectoryResponse::Ready { page } = response else {
            panic!("expected a ready page, got {response:?}");
        };
        let names: Vec<&str> = page.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src", "README.md"]);
        assert!(h.registry.list().is_empty(), "no operation was needed");

        let error = h
            .manager
            .list(
                list_request(&workspace, "nope"),
                Actor::User,
                CorrelationId::new(),
                Duration::from_secs(5),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::NotFound);
        let error = h
            .manager
            .list(
                ListDirectoryRequest {
                    page_size: Some(0),
                    ..list_request(&workspace, "")
                },
                Actor::User,
                CorrelationId::new(),
                Duration::from_secs(5),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert_eq!(error.details.unwrap()["field"], "pageSize");
    }

    #[tokio::test]
    async fn listing_over_budget_becomes_an_operation_that_completes_with_the_page() {
        let mut h = harness();
        let dir = TempDir::new("pending");
        let workspace = h
            .manager
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        slow_listings(&h, Duration::from_millis(150));
        let correlation = CorrelationId::new();
        let started = std::time::Instant::now();
        let response = h
            .manager
            .list(
                list_request(&workspace, "src"),
                Actor::User,
                correlation.clone(),
                SHORT_BUDGET,
            )
            .await
            .unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(140),
            "the command returned within its budget, not after the listing ({:?})",
            started.elapsed()
        );
        let ListDirectoryResponse::Pending { operation } = response else {
            panic!("expected a pending operation, got {response:?}");
        };
        assert_eq!(operation.kind.as_str(), LIST_KIND);
        assert_eq!(operation.workspace_id, Some(workspace.id.clone()));
        assert_eq!(operation.correlation_id, correlation);
        let completed = h.next_named("OperationCompleted").await;
        let CoreEvent::OperationCompleted(payload) = completed.event else {
            panic!("expected OperationCompleted");
        };
        assert_eq!(payload.operation.id, operation.id);
        let page: DirectoryPage = serde_json::from_value(payload.result.unwrap()).unwrap();
        assert_eq!(page.path.as_str(), "src");
        assert_eq!(page.entries[0].name, "main.rs");
    }

    #[tokio::test]
    async fn pending_listing_can_be_cancelled_and_its_page_is_discarded() {
        let mut h = harness();
        let dir = TempDir::new("cancel");
        let workspace = h
            .manager
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        slow_listings(&h, Duration::from_millis(300));
        let response = h
            .manager
            .list(
                list_request(&workspace, ""),
                Actor::User,
                CorrelationId::new(),
                SHORT_BUDGET,
            )
            .await
            .unwrap();
        let ListDirectoryResponse::Pending { operation } = response else {
            panic!("expected a pending operation, got {response:?}");
        };
        h.next_named("OperationStarted").await;
        let cancelled = h.registry.cancel(&operation.id).unwrap();
        assert!(cancelled.cancellation_requested);
        let bound = std::time::Instant::now();
        h.next_named("OperationCancelled").await;
        assert!(
            bound.elapsed() < Duration::from_millis(250),
            "cancellation did not wait for the listing ({:?})",
            bound.elapsed()
        );
        // Let the blocking job run past its delay: it observes the cancelled
        // token and its result never surfaces.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let names = h.sink.names();
        assert!(!names.contains(&"OperationCompleted"), "{names:?}");
        assert_eq!(names.last(), Some(&"OperationCancelled"));
        assert_eq!(
            h.registry.get(&operation.id).unwrap().status,
            OperationStatus::Cancelled
        );

        // Cancelling before the runner even started is immediate as well.
        let response = h
            .manager
            .list(
                list_request(&workspace, ""),
                Actor::User,
                CorrelationId::new(),
                SHORT_BUDGET,
            )
            .await
            .unwrap();
        let ListDirectoryResponse::Pending { operation } = response else {
            panic!("expected a pending operation, got {response:?}");
        };
        let early = h.registry.cancel(&operation.id).unwrap();
        assert_eq!(early.status, OperationStatus::Cancelled);
        tokio::time::sleep(Duration::from_millis(400)).await;
        let completed = h
            .sink
            .names()
            .iter()
            .filter(|n| **n == "OperationCompleted")
            .count();
        assert_eq!(completed, 0);
    }

    #[tokio::test]
    async fn root_removed_while_open_closes_the_workspace_and_fails_with_not_found() {
        let mut h = harness();
        let dir = TempDir::new("removed");
        let workspace = h
            .manager
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        fs::remove_dir_all(&dir.0).unwrap();
        let error = h
            .manager
            .list(
                list_request(&workspace, ""),
                Actor::User,
                CorrelationId::new(),
                Duration::from_secs(5),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::NotFound);
        assert_eq!(error.details.unwrap()["reason"], "ROOT_MISSING");
        let closed = h.next_named("WorkspaceClosed").await;
        assert_eq!(closed.meta.actor, Actor::System);
        let CoreEvent::WorkspaceClosed(payload) = closed.event else {
            panic!("expected WorkspaceClosed");
        };
        assert_eq!(payload.reason, WorkspaceClosedReason::RootMissing);
        assert_eq!(payload.workspace_id, workspace.id);
        assert_eq!(h.manager.current(&CorrelationId::new()), None);
        assert_eq!(h.sink.names().len(), 2, "closed exactly once");

        let dir = TempDir::new("removed-current");
        let workspace = h
            .manager
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        fs::remove_dir_all(&dir.0).unwrap();
        assert_eq!(h.manager.current(&CorrelationId::new()), None);
        let closed = h.next_named("WorkspaceClosed").await;
        assert_eq!(closed.meta.workspace_id, Some(workspace.id));
    }

    /// T070 §11: an external change under the open workspace reaches the bus
    /// as one coalesced batch of directories, attributed to `SYSTEM` and to no
    /// correlation id, because nothing the user asked for caused it.
    #[tokio::test]
    async fn an_external_change_is_announced_as_a_batch_of_directories() {
        let mut h = harness();
        let dir = TempDir::new("watch-change");
        let workspace = h
            .manager
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        // Written from outside Gritty, the way another process would.
        fs::write(dir.0.join("src/added.rs"), "fn added() {}").unwrap();
        let envelope = h.next_named("WorkspaceEntriesChanged").await;
        assert_eq!(envelope.meta.actor, Actor::System);
        assert_eq!(envelope.meta.correlation_id, None);
        assert_eq!(envelope.meta.workspace_id, Some(workspace.id.clone()));
        let CoreEvent::WorkspaceEntriesChanged(payload) = envelope.event else {
            panic!("expected WorkspaceEntriesChanged");
        };
        assert_eq!(payload.workspace_id, workspace.id);
        assert!(!payload.overflowed);
        assert!(
            payload
                .directories
                .contains(&WorkspacePath::parse("src").unwrap()),
            "{:?}",
            payload.directories
        );
    }

    /// T070 §10: the root's disappearance runs the *existing* `RootMissing`
    /// close, without waiting for a command to notice it — the difference from
    /// `a_removed_root_closes_the_workspace_lazily` is that nothing is called.
    #[tokio::test]
    async fn a_removed_root_closes_the_workspace_without_a_command() {
        let mut h = harness();
        let dir = TempDir::new("watch-root-gone");
        let workspace = h
            .manager
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        fs::remove_dir_all(&dir.0).unwrap();
        let closed = h.next_named("WorkspaceClosed").await;
        assert_eq!(closed.meta.actor, Actor::System);
        let CoreEvent::WorkspaceClosed(payload) = closed.event else {
            panic!("expected WorkspaceClosed");
        };
        assert_eq!(payload.reason, WorkspaceClosedReason::RootMissing);
        assert_eq!(payload.workspace_id, workspace.id);
        // And it closed once: the lazy path finds nothing left to close.
        assert_eq!(h.manager.current(&CorrelationId::new()), None);
        assert_eq!(
            h.sink
                .names()
                .iter()
                .filter(|name| **name == "WorkspaceClosed")
                .count(),
            1
        );
    }

    /// A closed workspace has no watch: a batch for a workspace that is gone
    /// would be a change the UI cannot place.
    #[tokio::test]
    async fn closing_the_workspace_stops_the_watch() {
        let h = harness();
        let dir = TempDir::new("watch-stop");
        let workspace = h
            .manager
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        h.manager
            .close(
                &workspace.id,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        for n in 0..20 {
            fs::write(dir.0.join(format!("after-{n}.txt")), "x").unwrap();
        }
        // Well past the watcher's debounce and its ceiling.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(
            !h.sink.names().contains(&"WorkspaceEntriesChanged"),
            "a closed workspace still reported changes: {:?}",
            h.sink.names()
        );
    }

    #[test]
    fn a_trash_refusal_is_io_with_a_reason_the_explorer_can_answer() {
        let error = CoreError::from(WorkspaceError::TrashRefused {
            context: "notes.md could not be moved to the trash".to_string(),
            reason: "no trash on this volume".to_string(),
        });
        assert_eq!(error.code, CoreErrorCode::Io);
        assert_eq!(
            error.message,
            "notes.md could not be moved to the trash: no trash on this volume"
        );
        assert_eq!(error.details.expect("details")["reason"], TRASH_REFUSED);

        // Any other I/O failure carries no such reason, so nothing offers to
        // delete permanently in answer to it.
        let other = CoreError::from(WorkspaceError::Io {
            context: "notes.md".to_string(),
            source: std::io::Error::other("disk"),
        });
        assert!(other.details.expect("details").get("reason").is_none());
    }

    #[test]
    fn wire_types_serialize_as_documented() {
        let response = ListDirectoryResponse::Ready {
            page: DirectoryPage {
                path: WorkspacePath::root(),
                entries: vec![],
                total: 0,
                truncated: false,
                cursor: None,
            },
        };
        assert_eq!(
            serde_json::to_value(&response).unwrap(),
            serde_json::json!({
                "status": "ready",
                "page": { "path": "", "entries": [], "total": 0, "truncated": false }
            })
        );
        assert_eq!(
            serde_json::to_value(WorkspaceClosedReason::RootMissing).unwrap(),
            "ROOT_MISSING"
        );
        let request: ListDirectoryRequest = serde_json::from_value(serde_json::json!({
            "workspaceId": "ws-1", "path": "src/lib"
        }))
        .unwrap();
        assert_eq!(request.path.as_str(), "src/lib");
        assert!(request.cursor.is_none());
        let error = serde_json::from_value::<ListDirectoryRequest>(serde_json::json!({
            "workspaceId": "ws-1", "path": "../x"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("'..'"), "{error}");
        let mapped = CoreError::from(WorkspaceError::PermissionDenied("escape".into()));
        assert_eq!(mapped.code, CoreErrorCode::PermissionDenied);
        assert!(!mapped.retryable);
    }
}
