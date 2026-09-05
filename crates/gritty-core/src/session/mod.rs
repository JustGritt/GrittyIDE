//! Session persistence and crash recovery (T060, ADR-004, ADR-014).
//!
//! The core owns every buffer (ADR-011); this module makes that ownership
//! durable. A [`SessionTap`] on the event bus turns document, workspace and
//! durable-operation events into intents; a writer task ([`writer`]) turns
//! intents into debounced database writes, taking dirty buffers from the
//! `DocumentManager` at write time. `session.restore` reopens the last
//! workspace and returns the persisted layout; `session.candidates` compares
//! each snapshot with the file on disk; `session.recover` puts a snapshot back
//! into a buffer (never onto the disk); `session.discard` drops it;
//! `session.update` stores what the UI owns (tabs, view state, preferences,
//! layout). The desktop shell records window geometry through
//! [`SessionService::record_window`] and reads it back at startup.
//!
//! Failure policy: a database that cannot be opened or migrated leaves the
//! service *unavailable* (the editor keeps working, every command reports
//! it); a write that fails is retried with backoff while buffers stay in
//! memory; a file that fails its deferred integrity check is moved aside and
//! replaced, announced through `PersistenceStatusChanged`.

pub mod wire;

mod tap;
mod writer;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gritty_persistence::store::{session, snapshots, tabs, window};
use gritty_persistence::{Database, PersistenceError, now_ms};
use gritty_workspace::{ContentHash, DiskProbe, DocumentEncoding, WorkspacePath, probe_file};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use tap::WriterMessage;
pub use tap::{Intent, SessionTap, TapReceiver};
pub use wire::*;
pub use writer::WriterConfig;
use writer::{Writer, emit_status};

use crate::document::{DocumentContent, DocumentManager, RecoveredBuffer};
use crate::event::EventBus;
use crate::protocol::actor::Actor;
use crate::protocol::error::{CoreError, CoreErrorCode};
use crate::protocol::ids::CorrelationId;
use crate::protocol::time::Timestamp;
use crate::workspace::{
    ForgetWorkspaceRequest, RecentWorkspace, RecentWorkspacesRequest, UnsavedPolicy,
    WorkspaceManager, recents,
};

/// Intents buffered between the bus and the writer before overflow triggers a resync.
pub const TAP_CAPACITY: usize = 1024;
/// Largest JSON blob accepted for `preferences` or `layout`.
pub const MAX_JSON_BYTES: usize = 64 * 1024;
/// Most tabs accepted in one `session.update`.
pub const MAX_TABS: usize = 500;

/// Window geometry in physical pixels, recorded by the desktop shell.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowGeometry {
    pub label: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
    pub maximized: bool,
}

/// Shared availability/health flags, read by commands and written by the
/// writer and the integrity check.
pub(crate) struct Status {
    available: AtomicBool,
    healthy: AtomicBool,
    database_path: Option<String>,
    message: Mutex<Option<String>>,
}

impl Status {
    fn new(available: bool, database_path: Option<String>, message: Option<String>) -> Self {
        Self {
            available: AtomicBool::new(available),
            healthy: AtomicBool::new(available),
            database_path,
            message: Mutex::new(message),
        }
    }

    pub(crate) fn snapshot(&self) -> PersistenceStatus {
        PersistenceStatus {
            available: self.available.load(Ordering::SeqCst),
            healthy: self.healthy.load(Ordering::SeqCst),
            database_path: self.database_path.clone(),
            message: self
                .message
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
    }

    pub(crate) fn set_healthy(&self) {
        self.healthy.store(true, Ordering::SeqCst);
        *self.message.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    pub(crate) fn set_unhealthy(&self, message: String) {
        self.healthy.store(false, Ordering::SeqCst);
        *self.message.lock().unwrap_or_else(PoisonError::into_inner) = Some(message);
    }

    fn set_unavailable(&self, message: String) {
        self.available.store(false, Ordering::SeqCst);
        self.healthy.store(false, Ordering::SeqCst);
        *self.message.lock().unwrap_or_else(PoisonError::into_inner) = Some(message);
    }
}

/// The outcome of opening the database at startup. Never fails: an unusable
/// database becomes an *unavailable* store with the reason attached.
pub struct SessionStore {
    database: Option<Arc<Database>>,
    path: Option<PathBuf>,
    error: Option<String>,
    notices: Vec<SessionNotice>,
}

impl SessionStore {
    /// Open (or create) the database at `path`, migrate it and reconcile
    /// durable operations. Runs synchronously; call it from the composition
    /// root before the window shows.
    pub fn open(path: &Path) -> Self {
        match Database::open(path) {
            Ok(opened) => {
                let mut notices = Vec::new();
                if let Some(aside) = &opened.moved_aside {
                    notices.push(SessionNotice {
                        kind: SessionNoticeKind::DatabaseMovedAside,
                        message: format!(
                            "The session database was unreadable and was moved to {}; a fresh one was created. Recovery snapshots and tab layout from before are in the old file.",
                            aside.display()
                        ),
                        path: Some(aside.display().to_string()),
                    });
                }
                match opened.database.write(|tx| {
                    gritty_persistence::store::operations::mark_interrupted(tx, now_ms())
                }) {
                    Ok(0) => {}
                    Ok(count) => {
                        tracing::warn!(
                            count,
                            "operations from the previous process marked INTERRUPTED"
                        );
                        notices.push(SessionNotice {
                            kind: SessionNoticeKind::OperationsInterrupted,
                            message: format!(
                                "{count} operation(s) were still running when Gritty last exited and are now INTERRUPTED."
                            ),
                            path: None,
                        });
                    }
                    Err(error) => {
                        tracing::warn!(%error, "could not reconcile durable operations");
                    }
                }
                Self {
                    database: Some(Arc::new(opened.database)),
                    path: Some(path.to_path_buf()),
                    error: None,
                    notices,
                }
            }
            Err(error) => {
                tracing::error!(path = %path.display(), %error, "session persistence unavailable");
                Self::unavailable_at(Some(path.to_path_buf()), error.to_string())
            }
        }
    }

    /// A migrated in-memory database (tests).
    pub fn in_memory() -> Self {
        match Database::open_in_memory() {
            Ok(database) => Self {
                database: Some(Arc::new(database)),
                path: None,
                error: None,
                notices: Vec::new(),
            },
            Err(error) => Self::unavailable_at(None, error.to_string()),
        }
    }

    /// A store that never opened (tests and the composition root when no
    /// data directory can be resolved).
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::unavailable_at(None, message.into())
    }

    fn unavailable_at(path: Option<PathBuf>, message: String) -> Self {
        Self {
            database: None,
            path,
            error: Some(message.clone()),
            notices: vec![SessionNotice {
                kind: SessionNoticeKind::PersistenceUnavailable,
                message: format!(
                    "Session persistence is off for this run: {message}. Tabs, layout and unsaved-buffer recovery are not being saved."
                ),
                path: None,
            }],
        }
    }

    pub fn database(&self) -> Option<&Arc<Database>> {
        self.database.as_ref()
    }

    pub fn notices(&self) -> &[SessionNotice] {
        &self.notices
    }
}

pub struct SessionService {
    handle: Handle,
    documents: Arc<DocumentManager>,
    workspaces: Arc<WorkspaceManager>,
    database: Option<Arc<Database>>,
    status: Arc<Status>,
    notices: Arc<Mutex<Vec<SessionNotice>>>,
    /// The first `restore` of the process may reopen the last root; later
    /// calls (renderer reloads) only report what is open.
    restored: AtomicBool,
    sender: mpsc::Sender<WriterMessage>,
}

impl SessionService {
    /// Start the writer (when the store opened) and the deferred integrity
    /// check on `handle`.
    pub fn start(
        handle: Handle,
        events: Arc<EventBus>,
        documents: Arc<DocumentManager>,
        workspaces: Arc<WorkspaceManager>,
        store: SessionStore,
        receiver: TapReceiver,
        config: WriterConfig,
    ) -> Arc<Self> {
        let status = Arc::new(Status::new(
            store.database.is_some(),
            store.path.as_ref().map(|path| path.display().to_string()),
            store.error.clone(),
        ));
        let notices = Arc::new(Mutex::new(store.notices.clone()));
        let TapReceiver {
            rx,
            sender,
            overflowed,
        } = receiver;
        if let Some(database) = &store.database {
            let writer = Writer::new(
                config,
                handle.clone(),
                Arc::clone(database),
                Arc::clone(&documents),
                Arc::clone(&workspaces),
                Arc::clone(&events),
                Arc::clone(&status),
                overflowed,
            );
            handle.spawn(writer.run(rx));
            if store.path.is_some() {
                handle.spawn(Self::integrity_check(
                    handle.clone(),
                    Arc::clone(database),
                    events,
                    Arc::clone(&status),
                    Arc::clone(&notices),
                    sender.clone(),
                ));
            }
        } else {
            // No writer: the tap's sends fail with `Closed` and are ignored.
            drop(rx);
        }
        Arc::new(Self {
            handle,
            documents,
            workspaces,
            database: store.database,
            status,
            notices,
            restored: AtomicBool::new(false),
            sender,
        })
    }

    /// `PRAGMA quick_check` off the startup path; a damaged file is replaced
    /// and announced, a failure to replace it turns persistence off.
    async fn integrity_check(
        handle: Handle,
        database: Arc<Database>,
        events: Arc<EventBus>,
        status: Arc<Status>,
        notices: Arc<Mutex<Vec<SessionNotice>>>,
        sender: mpsc::Sender<WriterMessage>,
    ) {
        let checked = {
            let database = Arc::clone(&database);
            handle.spawn_blocking(move || database.quick_check()).await
        };
        match checked {
            Ok(Ok(())) => tracing::debug!("session database passed quick_check"),
            Ok(Err(error)) if error.is_corrupt() => {
                tracing::warn!(%error, "session database failed quick_check; replacing it");
                let replaced = {
                    let database = Arc::clone(&database);
                    handle
                        .spawn_blocking(move || database.replace_corrupt())
                        .await
                };
                match replaced {
                    Ok(Ok((aside, _))) => {
                        let notice = SessionNotice {
                            kind: SessionNoticeKind::DatabaseMovedAside,
                            message: format!(
                                "The session database failed an integrity check and was moved to {}; a fresh one was created.",
                                aside.display()
                            ),
                            path: Some(aside.display().to_string()),
                        };
                        notices
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(notice.clone());
                        status.set_healthy();
                        let _ = sender.try_send(WriterMessage::Resync);
                        emit_status(&events, &status, Some(notice));
                    }
                    Ok(Err(error)) => {
                        tracing::error!(%error, "could not replace the damaged session database");
                        status.set_unavailable(error.to_string());
                        let notice = SessionNotice {
                            kind: SessionNoticeKind::PersistenceUnavailable,
                            message: format!(
                                "The session database is damaged and could not be replaced: {error}. Nothing is being persisted."
                            ),
                            path: Some(database.path().display().to_string()),
                        };
                        notices
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(notice.clone());
                        emit_status(&events, &status, Some(notice));
                    }
                    Err(join) => tracing::error!(%join, "integrity recovery task failed"),
                }
            }
            Ok(Err(error)) => tracing::warn!(%error, "quick_check could not run"),
            Err(join) => tracing::error!(%join, "integrity check task failed"),
        }
    }

    pub fn status(&self) -> PersistenceStatus {
        self.status.snapshot()
    }

    pub fn notices(&self) -> Vec<SessionNotice> {
        self.notices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// `session.restore`. Infallible: persistence trouble is reported in the
    /// response, never as a rejection, so the UI's rehydration always runs.
    /// `session.lastSession` (T051): the workspace the launcher offers to pick
    /// up again, and what was open in it.
    ///
    /// Deliberately *not* part of `restore`: restore runs once per WebView and
    /// reopens the last workspace by itself, so by the time the launcher is on
    /// screen the interesting question is a different one — "what did not come
    /// back, and why". This answers that, and it answers it again after the
    /// user closes a workspace mid-run, when restore will never run again.
    ///
    /// `None` when a workspace is open (the launcher is not showing), when
    /// there is no database (the notices already say so), or when the last
    /// workspace had no tabs — the history row is then the whole story.
    pub async fn last_session(
        &self,
        correlation: CorrelationId,
    ) -> Result<Option<LastSession>, CoreError> {
        if self.workspaces.current(&correlation).is_some() {
            return Ok(None);
        }
        let Some(database) = self.database.clone() else {
            return Ok(None);
        };
        let home = home_directory();
        self.blocking(database, move |db| {
            let Some(workspace) = recents::load(db, 1, home.as_deref())?.into_iter().next() else {
                return Ok(None);
            };
            let root = workspace.root.clone();
            let rows = db.read(|conn| tabs::list(conn, &root))?;
            let tabs: Vec<SessionTab> = rows.iter().filter_map(tab_from_row).collect();
            if tabs.is_empty() {
                return Ok(None);
            }
            let unsaved = db.read(|conn| snapshots::list_meta(conn, &root))?.len();
            Ok(Some(LastSession {
                workspace,
                tabs,
                unsaved: u32::try_from(unsaved).unwrap_or(u32::MAX),
            }))
        })
        .await
    }

    pub async fn restore(&self, correlation: CorrelationId) -> SessionRestoreResponse {
        let mut notices = self.notices();
        let current = self.workspaces.current(&correlation);
        let Some(database) = self.database.clone() else {
            return SessionRestoreResponse {
                workspace: current,
                tabs: Vec::new(),
                preferences: None,
                layout: None,
                persistence: self.status.snapshot(),
                notices,
            };
        };
        let row = match self
            .blocking(Arc::clone(&database), |db| db.read(session::load))
            .await
        {
            Ok(row) => row.unwrap_or_default(),
            Err(error) => {
                tracing::warn!(%error, "could not read the persisted session");
                return SessionRestoreResponse {
                    workspace: current,
                    tabs: Vec::new(),
                    preferences: None,
                    layout: None,
                    persistence: PersistenceStatus {
                        message: Some(error.message.clone()),
                        healthy: false,
                        ..self.status.snapshot()
                    },
                    notices,
                };
            }
        };
        let first = !self.restored.swap(true, Ordering::SeqCst);
        let workspace = match current {
            Some(workspace) => Some(workspace),
            None if first => match row.workspace_root.as_deref() {
                Some(root) => match self.workspaces.open(
                    Path::new(root),
                    UnsavedPolicy::Refuse,
                    Actor::System,
                    correlation.clone(),
                ) {
                    Ok(workspace) => {
                        tracing::info!(root, "session restored the last workspace");
                        Some(workspace)
                    }
                    Err(error) => {
                        tracing::warn!(root, code = %error.code, message = %error.message, "the last workspace could not be reopened");
                        notices.push(SessionNotice {
                            kind: SessionNoticeKind::WorkspaceRootMissing,
                            message: format!(
                                "The last workspace ({root}) could not be reopened: {}",
                                error.message
                            ),
                            path: Some(root.to_owned()),
                        });
                        None
                    }
                },
                None => None,
            },
            None => None,
        };
        let tabs = match &workspace {
            Some(workspace) => {
                let root = workspace.root.clone();
                match self
                    .blocking(database, move |db| db.read(|c| tabs::list(c, &root)))
                    .await
                {
                    Ok(rows) => rows.iter().filter_map(tab_from_row).collect(),
                    Err(error) => {
                        tracing::warn!(%error, "could not read the persisted tabs");
                        Vec::new()
                    }
                }
            }
            None => Vec::new(),
        };
        SessionRestoreResponse {
            workspace,
            tabs,
            preferences: parse_json(row.editor_preferences.as_deref(), "preferences"),
            layout: parse_json(row.layout.as_deref(), "layout"),
            persistence: self.status.snapshot(),
            notices,
        }
    }

    /// `workspace.recents`: the workspace history behind the startup screen
    /// (T049). Lives here rather than on `WorkspaceManager` because this is
    /// where the database handle is; the logic is `workspace::recents`.
    ///
    /// **Never fails for persistence reasons.** With the store unavailable the
    /// history is empty, not an error — the same rule `restore` follows, and
    /// the reason the launcher can render before it knows whether the database
    /// opened. `PersistenceUnavailable` already says why in the notices.
    pub async fn recents(
        &self,
        request: RecentWorkspacesRequest,
        _correlation: CorrelationId,
    ) -> Result<Vec<RecentWorkspace>, CoreError> {
        let Some(database) = self.database.clone() else {
            return Ok(Vec::new());
        };
        let limit = request.limit.unwrap_or(recents::DEFAULT_RECENTS);
        // `$HOME` is resolved per call rather than cached: it is one `getenv`
        // and a process whose environment changed under it should not keep
        // rendering the old `~`.
        let home = home_directory();
        let entries = self
            .blocking(database, move |db| {
                recents::load(db, limit, home.as_deref())
            })
            .await?;
        tracing::debug!(entries = entries.len(), "workspace history read");
        Ok(entries)
    }

    /// `workspace.forget`: drop one root from the history, and the
    /// last-workspace pointer with it when they are the same root.
    ///
    /// Idempotent, and narrow on purpose: tabs and recovery snapshots stored
    /// under the root stay (ADR-014).
    pub async fn forget(
        &self,
        request: ForgetWorkspaceRequest,
        _correlation: CorrelationId,
    ) -> Result<(), CoreError> {
        let database = self.database()?;
        let root = request.root.clone();
        self.blocking(database, move |db| recents::forget(db, &root))
            .await?;
        tracing::info!(root = %request.root, "workspace forgotten");
        Ok(())
    }

    /// `session.candidates`: every snapshot of the workspace's root compared
    /// with the disk. Documents that are open and dirty are the live truth
    /// and are skipped.
    pub async fn candidates(
        &self,
        request: CandidatesRequest,
        correlation: CorrelationId,
    ) -> Result<Vec<RecoveryCandidate>, CoreError> {
        let database = self.database()?;
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        let key = workspace.root.clone();
        let metas = self
            .blocking(database, move |db| {
                db.read(|c| snapshots::list_meta(c, &key))
            })
            .await?;
        let max = self.documents.limits().max_bytes;
        let mut candidates = Vec::with_capacity(metas.len());
        for meta in metas {
            let Ok(path) = WorkspacePath::parse(&meta.path) else {
                tracing::warn!(path = %meta.path, "ignoring a snapshot with an invalid path");
                continue;
            };
            if self
                .documents
                .find_by_path(&workspace.id, &path)
                .is_some_and(|document| document.dirty)
            {
                continue;
            }
            let probe = {
                let root = Arc::clone(&root);
                let path = path.clone();
                self.handle
                    .spawn_blocking(move || probe_file(&root, &path, max))
                    .await
                    .map_err(|join| CoreError::internal(format!("probe task failed: {join}")))?
            };
            let state = match probe {
                Ok(DiskProbe::Missing) => RecoveryState::FileMissing,
                Ok(DiskProbe::Present {
                    hash: Some(hash), ..
                }) if hash.as_str() == meta.saved_hash => RecoveryState::SameAsDisk,
                Ok(DiskProbe::Present { .. }) => RecoveryState::DiskChanged,
                Err(error) => {
                    tracing::warn!(%path, %error, "could not probe a snapshot's file; treating it as changed");
                    RecoveryState::DiskChanged
                }
            };
            candidates.push(RecoveryCandidate {
                path,
                state,
                updated_at: Timestamp::from_millis(u64::try_from(meta.updated_at).unwrap_or(0)),
                size: u64::try_from(meta.size).unwrap_or(0),
            });
        }
        Ok(candidates)
    }

    /// `session.recover`: put the snapshot back into the document's buffer.
    /// The row stays until the normal save/close path removes it.
    pub async fn recover(
        &self,
        request: RecoverSnapshotRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<DocumentContent, CoreError> {
        let database = self.database()?;
        let (workspace, _) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        let key = workspace.root.clone();
        let path_key = request.path.to_string();
        let row = self
            .blocking(database, move |db| {
                db.read(|c| snapshots::get(c, &key, &path_key))
            })
            .await?
            .ok_or_else(|| {
                CoreError::not_found(format!("no recovery snapshot for {}", request.path))
            })?;
        let saved_hash = ContentHash::parse(&row.saved_hash).ok_or_else(|| {
            CoreError::internal(format!(
                "snapshot for {} has an invalid saved hash",
                request.path
            ))
        })?;
        let encoding = DocumentEncoding::parse(&row.encoding).unwrap_or_else(|| {
            tracing::warn!(path = %request.path, encoding = %row.encoding, "snapshot has an unknown encoding; assuming utf-8");
            DocumentEncoding::Utf8
        });
        tracing::info!(path = %request.path, bytes = row.text.len(), "recovering a snapshot into the buffer");
        self.documents
            .open_recovered(
                workspace.id,
                request.path,
                RecoveredBuffer {
                    text: row.text,
                    encoding,
                    saved_hash,
                },
                actor,
                correlation,
            )
            .await
    }

    /// `session.discard`: drop the snapshot. Idempotent.
    pub async fn discard(
        &self,
        request: DiscardSnapshotRequest,
        correlation: CorrelationId,
    ) -> Result<(), CoreError> {
        let database = self.database()?;
        let (workspace, _) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        let key = workspace.root.clone();
        let path = request.path.to_string();
        let removed = self
            .blocking(database, move |db| {
                db.write(|tx| snapshots::delete(tx, &key, &path))
            })
            .await?;
        tracing::info!(path = %request.path, removed, "recovery snapshot discarded");
        Ok(())
    }

    /// `session.update`: persist what the UI owns, in one transaction. A
    /// no-op while persistence is unavailable (the UI knows from `restore`).
    pub async fn update(
        &self,
        request: UpdateSessionRequest,
        correlation: CorrelationId,
    ) -> Result<(), CoreError> {
        let Some(database) = self.database.clone() else {
            return Ok(());
        };
        let preferences = encode_json(request.preferences.as_ref(), "preferences")?;
        let layout = encode_json(request.layout.as_ref(), "layout")?;
        let tabs = match request.tabs {
            Some(tabs) if tabs.len() > MAX_TABS => {
                return Err(CoreError::invalid_input(
                    "tabs",
                    format!("at most {MAX_TABS} tabs"),
                ));
            }
            Some(tabs) => {
                let Some(workspace_id) = &request.workspace_id else {
                    return Err(CoreError::invalid_input(
                        "workspaceId",
                        "required when tabs are given",
                    ));
                };
                let (workspace, _) = self.workspaces.require(workspace_id, &correlation)?;
                Some((
                    workspace.root,
                    tabs.iter().map(tab_to_row).collect::<Vec<_>>(),
                ))
            }
            None => None,
        };
        self.blocking(database, move |db| {
            db.write(|tx| {
                let now = now_ms();
                if let Some((root, rows)) = &tabs {
                    tabs::replace(tx, root, rows, now)?;
                }
                if let Some(json) = &preferences {
                    session::set_editor_preferences(tx, Some(json), now)?;
                }
                if let Some(json) = &layout {
                    session::set_layout(tx, Some(json), now)?;
                }
                Ok(())
            })
        })
        .await
    }

    /// Debounced through the writer; safe to call on every resize/move event.
    pub fn record_window(&self, geometry: WindowGeometry) {
        let _ = self.sender.try_send(WriterMessage::Window(geometry));
    }

    /// The persisted geometry of a window, read synchronously (startup, before
    /// the window shows).
    pub fn window_state(&self, label: &str) -> Option<WindowGeometry> {
        let database = self.database.as_ref()?;
        match database.read(|c| window::load(c, label)) {
            Ok(Some(row)) => Some(WindowGeometry {
                label: row.label,
                x: row.x.and_then(|x| i32::try_from(x).ok()),
                y: row.y.and_then(|y| i32::try_from(y).ok()),
                width: u32::try_from(row.width).ok()?,
                height: u32::try_from(row.height).ok()?,
                scale_factor: row.scale_factor,
                maximized: row.maximized,
            }),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(%error, "could not read the window state");
                None
            }
        }
    }

    /// Write everything pending now; `false` when the writer did not answer
    /// within `timeout` (or persistence is unavailable).
    pub async fn flush(&self, timeout: Duration) -> bool {
        let (tx, rx) = oneshot::channel();
        if self.sender.send(WriterMessage::Flush(tx)).await.is_err() {
            return false;
        }
        tokio::time::timeout(timeout, rx)
            .await
            .is_ok_and(|r| r.is_ok())
    }

    fn database(&self) -> Result<Arc<Database>, CoreError> {
        self.database.clone().ok_or_else(|| {
            CoreError::new(
                CoreErrorCode::Io,
                "session persistence is unavailable for this run",
            )
            .with_details(serde_json::json!({ "persistence": self.status.snapshot() }))
        })
    }

    async fn blocking<T: Send + 'static>(
        &self,
        database: Arc<Database>,
        work: impl FnOnce(&Database) -> Result<T, PersistenceError> + Send + 'static,
    ) -> Result<T, CoreError> {
        self.handle
            .spawn_blocking(move || work(&database))
            .await
            .map_err(|join| CoreError::internal(format!("session task failed: {join}")))?
            .map_err(persistence_error)
    }
}

/// The user's home directory, for shortening a displayed path (T049).
///
/// Reuses the terminal crate's resolver — `$HOME` first, then the account's
/// own home from the password database — because it is the one place that
/// already answers this question correctly for an app launched without an
/// environment. A failure is not an error here: a path that could not be
/// shortened is shown in full.
fn home_directory() -> Option<std::path::PathBuf> {
    match gritty_terminal::shell::home_directory(|key| std::env::var(key).ok()) {
        Ok(home) => Some(home),
        Err(error) => {
            tracing::debug!(%error, "no home directory; recent paths stay absolute");
            None
        }
    }
}

fn persistence_error(error: PersistenceError) -> CoreError {
    CoreError::new(CoreErrorCode::Io, format!("session database: {error}"))
}

fn parse_json(raw: Option<&str>, field: &str) -> Option<serde_json::Value> {
    let raw = raw?;
    match serde_json::from_str(raw) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(field, %error, "ignoring an unparseable persisted blob");
            None
        }
    }
}

fn encode_json(
    value: Option<&serde_json::Value>,
    field: &str,
) -> Result<Option<String>, CoreError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let json = serde_json::to_string(value)?;
    if json.len() > MAX_JSON_BYTES {
        return Err(CoreError::invalid_input(
            field,
            format!("must be at most {MAX_JSON_BYTES} bytes as JSON"),
        ));
    }
    Ok(Some(json))
}

fn tab_to_row(tab: &SessionTab) -> tabs::TabRow {
    let clamp = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    tabs::TabRow {
        path: tab.path.to_string(),
        position: 0,
        active: tab.active,
        view_mode: tab.view_mode.clone(),
        cursor_line: tab.cursor.map(|p| clamp(p.line)),
        cursor_column: tab.cursor.map(|p| clamp(p.column)),
        anchor_line: tab.anchor.map(|p| clamp(p.line)),
        anchor_column: tab.anchor.map(|p| clamp(p.column)),
        scroll_line: tab.scroll_line.map(clamp),
        updated_at: 0,
    }
}

fn tab_from_row(row: &tabs::TabRow) -> Option<SessionTab> {
    let path = match WorkspacePath::parse(&row.path) {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(path = %row.path, %error, "ignoring a persisted tab with an invalid path");
            return None;
        }
    };
    let position = |line: Option<i64>, column: Option<i64>| {
        Some(TextPosition {
            line: u64::try_from(line?).ok()?,
            column: u64::try_from(column?).ok()?,
        })
    };
    Some(SessionTab {
        path,
        active: row.active,
        view_mode: row.view_mode.clone(),
        cursor: position(row.cursor_line, row.cursor_column),
        anchor: position(row.anchor_line, row.anchor_column),
        scroll_line: row.scroll_line.and_then(|line| u64::try_from(line).ok()),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    use gritty_persistence::store::{operations, snapshots};
    use gritty_persistence::testing::TempDir;
    use gritty_persistence::{DB_FILE_NAME, Database};
    use tokio::runtime::Handle;

    use super::*;
    use crate::document::{
        CloseDocumentRequest, Document, DocumentLimits, ExternalModificationState,
        OpenDocumentRequest, ReloadDocumentRequest, SaveDocumentRequest, UpdateDocumentRequest,
    };
    use crate::event::FanOutSink;
    use crate::event::testing::RecordingSink;
    use crate::operation::{OperationKind, OperationRegistry, OperationSpec};
    use crate::protocol::ids::DocumentId;
    use crate::workspace::Workspace;

    const FAST: WriterConfig = WriterConfig {
        debounce: Duration::from_millis(50),
        max_wait: Duration::from_millis(150),
        window_debounce: Duration::from_millis(20),
        backoff_base: Duration::from_millis(50),
        backoff_cap: Duration::from_millis(200),
    };
    const LIMITS: DocumentLimits = DocumentLimits {
        large_file_threshold: 4096,
        max_bytes: 64 * 1024,
    };
    const WAIT: Duration = Duration::from_secs(5);

    struct Harness {
        workspaces: Arc<WorkspaceManager>,
        documents: Arc<DocumentManager>,
        registry: Arc<OperationRegistry>,
        session: Arc<SessionService>,
        sink: Arc<RecordingSink>,
        database: Option<Arc<Database>>,
        db_path: PathBuf,
    }

    fn start(db_path: &Path, config: WriterConfig) -> Harness {
        start_with(db_path, config, SessionStore::open(db_path))
    }

    fn start_with(db_path: &Path, config: WriterConfig, store: SessionStore) -> Harness {
        let (tap, receiver) = SessionTap::new(TAP_CAPACITY);
        let sink = RecordingSink::new();
        let events = EventBus::new(FanOutSink::new(vec![sink.clone(), tap]));
        let registry = OperationRegistry::new(Handle::current(), events.clone());
        let workspaces = WorkspaceManager::new(Handle::current(), events.clone(), registry.clone());
        let documents = DocumentManager::new(
            Handle::current(),
            events.clone(),
            workspaces.clone(),
            LIMITS,
        );
        let database = store.database().cloned();
        let session = SessionService::start(
            Handle::current(),
            events,
            documents.clone(),
            workspaces.clone(),
            store,
            receiver,
            config,
        );
        Harness {
            workspaces,
            documents,
            registry,
            session,
            sink,
            database,
            db_path: db_path.to_path_buf(),
        }
    }

    fn workspace_dir(label: &str) -> TempDir {
        let dir = TempDir::new(&format!("ws-{label}"));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("README.md"), "# hi\n").unwrap();
        fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.join("notes.txt"), "notes\n").unwrap();
        dir
    }

    fn root_of(dir: &TempDir) -> PathBuf {
        fs::canonicalize(dir.path()).unwrap()
    }

    async fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + WAIT;
        while !condition() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    impl Harness {
        fn open_ws(&self, dir: &TempDir) -> Workspace {
            self.workspaces
                .open(
                    dir.path(),
                    UnsavedPolicy::Refuse,
                    Actor::User,
                    CorrelationId::new(),
                )
                .unwrap()
        }

        async fn open_doc(&self, ws: &Workspace, path: &str) -> DocumentContent {
            self.documents
                .open(
                    OpenDocumentRequest {
                        workspace_id: ws.id.clone(),
                        path: WorkspacePath::parse(path).unwrap(),
                        large_file_editing: None,
                    },
                    Actor::User,
                    CorrelationId::new(),
                )
                .await
                .unwrap()
        }

        async fn update(&self, id: &DocumentId, base: u64, text: &str) -> Document {
            self.documents
                .update(
                    UpdateDocumentRequest {
                        document_id: id.clone(),
                        base_version: base,
                        text: text.to_owned(),
                    },
                    Actor::User,
                    CorrelationId::new(),
                )
                .await
                .unwrap()
        }

        async fn save(&self, id: &DocumentId, force: bool) -> Result<Document, CoreError> {
            self.documents
                .save(
                    SaveDocumentRequest {
                        document_id: id.clone(),
                        force: force.then_some(true),
                    },
                    Actor::User,
                    CorrelationId::new(),
                )
                .await
        }

        async fn reload(&self, id: &DocumentId) -> DocumentContent {
            self.documents
                .reload(
                    ReloadDocumentRequest {
                        document_id: id.clone(),
                    },
                    Actor::User,
                    CorrelationId::new(),
                )
                .await
                .unwrap()
        }

        fn close(&self, id: &DocumentId, force: bool) -> Result<Document, CoreError> {
            self.documents.close(
                CloseDocumentRequest {
                    document_id: id.clone(),
                    force: force.then_some(true),
                },
                Actor::User,
                CorrelationId::new(),
            )
        }

        /// A separate handle on the same file, like the next process would open.
        fn reader(&self) -> Database {
            Database::open(&self.db_path).unwrap().database
        }

        fn snapshot_row(&self, root: &Path, path: &str) -> Option<snapshots::SnapshotRow> {
            self.reader()
                .read(|c| snapshots::get(c, &root.display().to_string(), path))
                .unwrap()
        }

        fn last_root(&self) -> Option<String> {
            self.reader()
                .read(session::load)
                .unwrap()
                .and_then(|row| row.workspace_root)
        }

        async fn flush(&self) {
            assert!(self.session.flush(WAIT).await, "writer flushed");
        }

        async fn candidates(&self, ws: &Workspace) -> Vec<RecoveryCandidate> {
            self.session
                .candidates(
                    CandidatesRequest {
                        workspace_id: ws.id.clone(),
                    },
                    CorrelationId::new(),
                )
                .await
                .unwrap()
        }

        async fn recover(&self, ws: &Workspace, path: &str) -> Result<DocumentContent, CoreError> {
            self.session
                .recover(
                    RecoverSnapshotRequest {
                        workspace_id: ws.id.clone(),
                        path: WorkspacePath::parse(path).unwrap(),
                    },
                    Actor::User,
                    CorrelationId::new(),
                )
                .await
        }

        fn status_events(&self) -> Vec<(bool, bool, Option<SessionNoticeKind>)> {
            self.sink
                .recorded()
                .iter()
                .filter_map(|envelope| match &envelope.event {
                    CoreEvent::PersistenceStatusChanged(p) => Some((
                        p.status.available,
                        p.status.healthy,
                        p.notice.as_ref().map(|n| n.kind),
                    )),
                    _ => None,
                })
                .collect()
        }
    }

    use crate::event::CoreEvent;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dirty_buffer_is_snapshotted_after_the_debounce_and_removed_on_save() {
        let data = TempDir::new("snapshot");
        let dir = workspace_dir("snapshot");
        let h = start(&data.join(DB_FILE_NAME), FAST);
        let ws = h.open_ws(&dir);
        let root = root_of(&dir);
        let doc = h.open_doc(&ws, "src/main.rs").await;
        // A clean open never produces a row.
        tokio::time::sleep(FAST.max_wait * 2).await;
        assert!(h.snapshot_row(&root, "src/main.rs").is_none());

        let updated = h
            .update(&doc.document.id, 1, "fn main() { edited }\n")
            .await;
        assert!(updated.dirty);
        wait_for("snapshot row", || {
            h.snapshot_row(&root, "src/main.rs").is_some()
        })
        .await;
        let row = h.snapshot_row(&root, "src/main.rs").unwrap();
        assert_eq!(row.text, "fn main() { edited }\n");
        assert_eq!(row.saved_hash, doc.document.saved_hash.to_string());
        assert_eq!(row.current_version, 2);
        assert_eq!(row.encoding, "utf-8");
        assert_eq!(h.last_root().as_deref(), Some(root.to_str().unwrap()));

        h.save(&doc.document.id, false).await.unwrap();
        wait_for("row removed after save", || {
            h.snapshot_row(&root, "src/main.rs").is_none()
        })
        .await;
        assert!(h.session.status().healthy);
        assert!(
            h.status_events().is_empty(),
            "no status transitions on the happy path"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn continuous_edits_are_bounded_by_max_wait() {
        let data = TempDir::new("maxwait");
        let dir = workspace_dir("maxwait");
        let h = start(&data.join(DB_FILE_NAME), FAST);
        let ws = h.open_ws(&dir);
        let root = root_of(&dir);
        let doc = h.open_doc(&ws, "notes.txt").await;
        let started = tokio::time::Instant::now();
        let mut version = 1;
        let mut first_row_at = None;
        // Type faster than the debounce for longer than max_wait.
        while started.elapsed() < FAST.max_wait * 4 {
            version = h
                .update(&doc.document.id, version, &format!("notes {version}\n"))
                .await
                .current_version;
            if first_row_at.is_none() && h.snapshot_row(&root, "notes.txt").is_some() {
                first_row_at = Some(started.elapsed());
            }
            tokio::time::sleep(FAST.debounce / 2).await;
        }
        let first = first_row_at.expect("a snapshot was written while typing continued");
        assert!(
            first < FAST.max_wait * 2,
            "first snapshot after {first:?}, max_wait {:?}",
            FAST.max_wait
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn force_closing_a_dirty_document_removes_its_snapshot() {
        let data = TempDir::new("close");
        let dir = workspace_dir("close");
        let h = start(&data.join(DB_FILE_NAME), FAST);
        let ws = h.open_ws(&dir);
        let root = root_of(&dir);
        let doc = h.open_doc(&ws, "notes.txt").await;
        h.update(&doc.document.id, 1, "unsaved\n").await;
        wait_for("row", || h.snapshot_row(&root, "notes.txt").is_some()).await;
        assert!(
            h.close(&doc.document.id, false).is_err(),
            "dirty close refused"
        );
        assert!(h.snapshot_row(&root, "notes.txt").is_some());
        h.close(&doc.document.id, true).unwrap();
        wait_for("row removed after discard", || {
            h.snapshot_row(&root, "notes.txt").is_none()
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_root_that_disappears_keeps_snapshots_and_the_last_workspace() {
        let data = TempDir::new("rootgone");
        let dir = workspace_dir("rootgone");
        let h = start(&data.join(DB_FILE_NAME), FAST);
        let ws = h.open_ws(&dir);
        let root = root_of(&dir);
        let doc = h.open_doc(&ws, "notes.txt").await;
        h.update(&doc.document.id, 1, "unsaved\n").await;
        wait_for("row", || h.snapshot_row(&root, "notes.txt").is_some()).await;

        fs::remove_dir_all(dir.path()).unwrap();
        assert!(h.workspaces.current(&CorrelationId::new()).is_none());
        h.flush().await;
        assert!(
            h.snapshot_row(&root, "notes.txt").is_some(),
            "kept for recovery"
        );
        assert_eq!(h.last_root().as_deref(), Some(root.to_str().unwrap()));

        // A user close, by contrast, clears the last root.
        let dir2 = workspace_dir("rootgone-2");
        let ws2 = h.open_ws(&dir2);
        h.flush().await;
        assert_eq!(h.last_root().as_deref(), root_of(&dir2).to_str());
        h.workspaces
            .close(
                &ws2.id,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        h.flush().await;
        assert_eq!(h.last_root(), None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_snapshot_from_a_previous_run_survives_a_clean_open_of_the_same_path() {
        let data = TempDir::new("survive");
        let dir = workspace_dir("survive");
        let db = data.join(DB_FILE_NAME);
        let root = root_of(&dir);
        {
            let previous = start(&db, FAST);
            let ws = previous.open_ws(&dir);
            let doc = previous.open_doc(&ws, "notes.txt").await;
            previous.update(&doc.document.id, 1, "from before\n").await;
            previous.flush().await;
            std::mem::forget(previous);
        }
        let h = start(&db, FAST);
        let ws = h.open_ws(&dir);
        let doc = h.open_doc(&ws, "notes.txt").await;
        assert!(!doc.document.dirty);
        tokio::time::sleep(FAST.max_wait * 2).await;
        h.flush().await;
        assert!(
            h.snapshot_row(&root, "notes.txt").is_some(),
            "a clean open never deletes an inherited row"
        );
        // Closing the clean tab keeps it too; only a discard or our own write removes it.
        h.close(&doc.document.id, false).unwrap();
        h.flush().await;
        assert!(h.snapshot_row(&root, "notes.txt").is_some());
        assert_eq!(h.candidates(&ws).await.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_last_session_is_offered_only_once_there_is_something_to_offer() {
        let data = TempDir::new("last-session");
        let db = data.join(DB_FILE_NAME);
        let dir = workspace_dir("last-session");
        let h = start(&db, FAST);
        let correlation = CorrelationId::new();

        // Nothing has ever been opened.
        assert!(
            h.session
                .last_session(correlation.clone())
                .await
                .unwrap()
                .is_none()
        );

        let ws = h.open_ws(&dir);
        h.flush().await;
        assert!(
            h.session
                .last_session(correlation.clone())
                .await
                .unwrap()
                .is_none(),
            "a workspace is open, so the launcher is not showing"
        );

        // Opened, but nothing was ever in it: the history row is the whole story.
        h.workspaces
            .close(
                &ws.id,
                UnsavedPolicy::Refuse,
                Actor::User,
                correlation.clone(),
            )
            .unwrap();
        h.flush().await;
        assert!(
            h.session
                .last_session(correlation.clone())
                .await
                .unwrap()
                .is_none(),
            "a rail that only repeats the recents list has not earned its space"
        );

        // With tabs, it has something to say.
        let ws = h.open_ws(&dir);
        h.session
            .update(
                UpdateSessionRequest {
                    workspace_id: Some(ws.id.clone()),
                    tabs: Some(vec![
                        SessionTab {
                            path: WorkspacePath::parse("README.md").unwrap(),
                            active: true,
                            view_mode: None,
                            cursor: None,
                            anchor: None,
                            scroll_line: None,
                        },
                        SessionTab {
                            path: WorkspacePath::parse("src/main.rs").unwrap(),
                            active: false,
                            view_mode: None,
                            cursor: None,
                            anchor: None,
                            scroll_line: None,
                        },
                    ]),
                    preferences: None,
                    layout: None,
                },
                correlation.clone(),
            )
            .await
            .unwrap();
        h.workspaces
            .close(
                &ws.id,
                UnsavedPolicy::Refuse,
                Actor::User,
                correlation.clone(),
            )
            .unwrap();
        h.flush().await;

        let last = h
            .session
            .last_session(correlation.clone())
            .await
            .unwrap()
            .expect("the workspace that was just closed");
        assert_eq!(last.workspace.root, root_of(&dir).display().to_string());
        assert!(last.workspace.exists);
        assert_eq!(
            last.tabs
                .iter()
                .map(|tab| tab.path.as_str())
                .collect::<Vec<_>>(),
            ["README.md", "src/main.rs"]
        );
        assert_eq!(last.unsaved, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_last_session_counts_unsaved_buffers_and_reports_a_missing_root() {
        let data = TempDir::new("last-session-unsaved");
        let db = data.join(DB_FILE_NAME);
        let dir = workspace_dir("last-session-unsaved");
        let correlation = CorrelationId::new();

        // A window that closed with unsaved work in it — a crash or a quit,
        // not a user closing the workspace, which would have discarded it.
        {
            let previous = start(&db, FAST);
            let ws = previous.open_ws(&dir);
            let opened = previous.open_doc(&ws, "README.md").await;
            previous
                .update(
                    &opened.document.id,
                    opened.document.current_version,
                    "# edited\n",
                )
                .await;
            previous
                .session
                .update(
                    UpdateSessionRequest {
                        workspace_id: Some(ws.id.clone()),
                        tabs: Some(vec![SessionTab {
                            path: WorkspacePath::parse("README.md").unwrap(),
                            active: true,
                            view_mode: None,
                            cursor: None,
                            anchor: None,
                            scroll_line: None,
                        }]),
                        preferences: None,
                        layout: None,
                    },
                    correlation.clone(),
                )
                .await
                .unwrap();
            previous.flush().await;
            wait_for("the dirty buffer to be snapshotted", || {
                previous
                    .database
                    .as_ref()
                    .is_some_and(|db| db.read(snapshots::count).unwrap_or(0) > 0)
            })
            .await;
            std::mem::forget(previous);
        }

        let h = start(&db, FAST);
        let last = h
            .session
            .last_session(correlation.clone())
            .await
            .unwrap()
            .expect("the workspace the window closed on");
        assert_eq!(last.tabs.len(), 1);
        assert_eq!(
            last.unsaved, 1,
            "restoring keeps the buffer dirty, so the rail has to say so"
        );
        assert!(last.workspace.exists);

        // The directory disappears: the rail still knows about it, and says the
        // root is gone rather than offering to reopen nothing.
        fs::remove_dir_all(dir.path()).unwrap();
        let gone = h
            .session
            .last_session(correlation)
            .await
            .unwrap()
            .expect("still known, just not there");
        assert!(!gone.workspace.exists);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn without_a_database_there_is_no_last_session_to_offer() {
        let data = TempDir::new("last-session-nodb");
        let h = start_with(
            &data.join(DB_FILE_NAME),
            FAST,
            SessionStore::unavailable("no data directory"),
        );
        assert!(
            h.session
                .last_session(CorrelationId::new())
                .await
                .unwrap()
                .is_none(),
            "the notices already explain that nothing is being saved"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn restore_reopens_the_last_workspace_once_and_returns_the_layout() {
        let data = TempDir::new("restore");
        let dir = workspace_dir("restore");
        let db = data.join(DB_FILE_NAME);
        let root = root_of(&dir);
        {
            let previous = start(&db, FAST);
            let ws = previous.open_ws(&dir);
            previous
                .session
                .update(
                    UpdateSessionRequest {
                        workspace_id: Some(ws.id.clone()),
                        tabs: Some(vec![
                            SessionTab {
                                path: WorkspacePath::parse("README.md").unwrap(),
                                active: false,
                                view_mode: Some("preview".into()),
                                cursor: Some(TextPosition { line: 2, column: 1 }),
                                anchor: Some(TextPosition { line: 1, column: 1 }),
                                scroll_line: Some(1),
                            },
                            SessionTab {
                                path: WorkspacePath::parse("src/main.rs").unwrap(),
                                active: true,
                                view_mode: None,
                                cursor: Some(TextPosition { line: 1, column: 4 }),
                                anchor: None,
                                scroll_line: None,
                            },
                        ]),
                        preferences: Some(serde_json::json!({ "version": 1, "fontSize": 15 })),
                        layout: Some(
                            serde_json::json!({ "version": 1, "explorer": { "showHidden": true } }),
                        ),
                    },
                    CorrelationId::new(),
                )
                .await
                .unwrap();
            previous.flush().await;
            std::mem::forget(previous);
        }

        let h = start(&db, FAST);
        assert!(h.workspaces.current(&CorrelationId::new()).is_none());
        let restored = h.session.restore(CorrelationId::new()).await;
        let workspace = restored.workspace.clone().expect("last workspace reopened");
        assert_eq!(workspace.root, root.display().to_string());
        assert_eq!(restored.tabs.len(), 2);
        assert_eq!(restored.tabs[0].path.as_str(), "README.md");
        assert_eq!(restored.tabs[0].view_mode.as_deref(), Some("preview"));
        assert_eq!(
            restored.tabs[0].cursor,
            Some(TextPosition { line: 2, column: 1 })
        );
        assert!(restored.tabs[1].active);
        assert_eq!(restored.tabs[1].anchor, None);
        assert_eq!(restored.preferences.unwrap()["fontSize"], 15);
        assert_eq!(restored.layout.unwrap()["explorer"]["showHidden"], true);
        assert!(restored.persistence.available && restored.persistence.healthy);
        assert!(restored.notices.is_empty(), "{:?}", restored.notices);
        assert!(
            h.sink.names().contains(&"WorkspaceOpened"),
            "the explorer learns about the reopened workspace from the event"
        );

        // A second call (renderer reload) reports the same workspace without reopening.
        let again = h.session.restore(CorrelationId::new()).await;
        assert_eq!(again.workspace.unwrap().id, workspace.id);
        assert_eq!(again.tabs.len(), 2);
        assert_eq!(
            h.sink
                .names()
                .iter()
                .filter(|n| **n == "WorkspaceOpened")
                .count(),
            1
        );

        // After the user closes it, a reload does not bring it back.
        h.workspaces
            .close(
                &workspace.id,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        let after_close = h.session.restore(CorrelationId::new()).await;
        assert!(after_close.workspace.is_none());
        assert!(after_close.tabs.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn restore_reports_a_missing_root_instead_of_failing() {
        let data = TempDir::new("restore-missing");
        let db = data.join(DB_FILE_NAME);
        {
            let previous = start(&db, FAST);
            let dir = workspace_dir("restore-missing");
            previous.open_ws(&dir);
            previous.flush().await;
            std::mem::forget(previous);
            // `dir` is removed here.
        }
        let h = start(&db, FAST);
        let restored = h.session.restore(CorrelationId::new()).await;
        assert!(restored.workspace.is_none());
        assert_eq!(restored.notices.len(), 1);
        assert_eq!(
            restored.notices[0].kind,
            SessionNoticeKind::WorkspaceRootMissing
        );
        assert!(restored.notices[0].path.is_some());
    }

    /// Writes one snapshot with a previous "process", then hands back a fresh
    /// harness on the same database with the workspace reopened.
    async fn with_inherited_snapshot(
        label: &str,
    ) -> (TempDir, TempDir, Harness, Workspace, String) {
        let data = TempDir::new(label);
        let dir = workspace_dir(label);
        let db = data.join(DB_FILE_NAME);
        let saved_hash = {
            let previous = start(&db, FAST);
            let ws = previous.open_ws(&dir);
            let doc = previous.open_doc(&ws, "notes.txt").await;
            previous
                .update(&doc.document.id, 1, "recovered text\n")
                .await;
            previous.flush().await;
            std::mem::forget(previous);
            doc.document.saved_hash.to_string()
        };
        let h = start(&db, FAST);
        let restored = h.session.restore(CorrelationId::new()).await;
        let ws = restored.workspace.unwrap();
        (data, dir, h, ws, saved_hash)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn candidates_reflect_the_disk_and_skip_dirty_open_documents() {
        let (_data, dir, h, ws, _) = with_inherited_snapshot("candidates").await;
        let same = h.candidates(&ws).await;
        assert_eq!(same.len(), 1);
        assert_eq!(same[0].path.as_str(), "notes.txt");
        assert_eq!(same[0].state, RecoveryState::SameAsDisk);
        assert_eq!(same[0].size, "recovered text\n".len() as u64);
        assert!(same[0].updated_at.as_millis() > 0);

        fs::write(dir.join("notes.txt"), "changed on disk\n").unwrap();
        assert_eq!(h.candidates(&ws).await[0].state, RecoveryState::DiskChanged);

        fs::remove_file(dir.join("notes.txt")).unwrap();
        assert_eq!(h.candidates(&ws).await[0].state, RecoveryState::FileMissing);

        // A dirty open document is the live truth: the row is not a candidate.
        fs::write(dir.join("notes.txt"), "notes\n").unwrap();
        let doc = h.open_doc(&ws, "notes.txt").await;
        assert_eq!(
            h.candidates(&ws).await.len(),
            1,
            "clean open still offers it"
        );
        h.update(&doc.document.id, 1, "typed since launch\n").await;
        assert!(h.candidates(&ws).await.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recover_same_as_disk_yields_a_dirty_unchanged_document() {
        let (_data, dir, h, ws, saved_hash) = with_inherited_snapshot("recover-same").await;
        let root = root_of(&dir);
        let content = h.recover(&ws, "notes.txt").await.unwrap();
        assert_eq!(content.text.as_deref(), Some("recovered text\n"));
        assert!(content.document.dirty);
        assert_eq!(
            content.document.external_modification_state,
            ExternalModificationState::Unchanged
        );
        assert_eq!(content.document.saved_hash.to_string(), saved_hash);
        assert_eq!(content.document.current_version, 2);
        assert_eq!(
            fs::read_to_string(dir.join("notes.txt")).unwrap(),
            "notes\n",
            "disk untouched"
        );
        assert!(h.sink.recorded().iter().any(|e| matches!(
            &e.event,
            CoreEvent::DocumentChanged(p) if p.cause == crate::document::DocumentChangeCause::Recovered
        )));
        // The row stays until saved; the writer rewrites it for the new version.
        h.flush().await;
        assert!(h.snapshot_row(&root, "notes.txt").is_some());
        // Recovering twice is refused: the buffer now holds unsaved work.
        let again = h.recover(&ws, "notes.txt").await.unwrap_err();
        assert_eq!(again.code, CoreErrorCode::Conflict);
        h.save(&content.document.id, false).await.unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("notes.txt")).unwrap(),
            "recovered text\n"
        );
        wait_for("row removed after save", || {
            h.snapshot_row(&root, "notes.txt").is_none()
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recover_disk_changed_yields_a_conflicted_document_that_reload_resolves() {
        let (_data, dir, h, ws, saved_hash) = with_inherited_snapshot("recover-changed").await;
        fs::write(dir.join("notes.txt"), "changed on disk\n").unwrap();
        let content = h.recover(&ws, "notes.txt").await.unwrap();
        assert_eq!(content.text.as_deref(), Some("recovered text\n"));
        assert!(content.document.dirty);
        assert_eq!(
            content.document.external_modification_state,
            ExternalModificationState::Conflicted
        );
        assert_eq!(
            content.document.saved_hash.to_string(),
            saved_hash,
            "base of the snapshot"
        );
        let id = content.document.id.clone();
        // Saving without force is refused: the disk moved.
        let refused = h.save(&id, false).await.unwrap_err();
        assert_eq!(refused.code, CoreErrorCode::Conflict);
        assert_eq!(
            fs::read_to_string(dir.join("notes.txt")).unwrap(),
            "changed on disk\n"
        );
        // Reload drops the snapshot and shows the disk.
        let reloaded = h.reload(&id).await;
        assert_eq!(reloaded.text.as_deref(), Some("changed on disk\n"));
        assert!(!reloaded.document.dirty);
        assert_eq!(
            reloaded.document.external_modification_state,
            ExternalModificationState::Unchanged
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recover_disk_changed_can_overwrite_with_force() {
        let (_data, dir, h, ws, _) = with_inherited_snapshot("recover-overwrite").await;
        fs::write(dir.join("notes.txt"), "changed on disk\n").unwrap();
        let content = h.recover(&ws, "notes.txt").await.unwrap();
        let saved = h.save(&content.document.id, true).await.unwrap();
        assert!(!saved.dirty);
        assert_eq!(
            fs::read_to_string(dir.join("notes.txt")).unwrap(),
            "recovered text\n"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recover_file_missing_yields_a_deleted_document_that_force_save_recreates() {
        let (_data, dir, h, ws, saved_hash) = with_inherited_snapshot("recover-missing").await;
        let root = root_of(&dir);
        fs::remove_file(dir.join("notes.txt")).unwrap();
        let content = h.recover(&ws, "notes.txt").await.unwrap();
        assert_eq!(content.text.as_deref(), Some("recovered text\n"));
        assert!(content.document.dirty);
        assert_eq!(
            content.document.external_modification_state,
            ExternalModificationState::DeletedExternally
        );
        assert_eq!(content.document.saved_hash.to_string(), saved_hash);
        assert_eq!(content.document.language_id, "plaintext");
        assert_eq!(content.document.size, 0);
        assert!(!content.document.large_file_mode);
        let id = content.document.id.clone();
        assert!(
            h.documents.list().iter().any(|d| d.id == id),
            "registered like any document"
        );
        assert!(h.sink.names().contains(&"DocumentOpened"));
        assert_eq!(
            h.save(&id, false).await.unwrap_err().code,
            CoreErrorCode::Conflict
        );
        assert!(!dir.join("notes.txt").exists());
        let saved = h.save(&id, true).await.unwrap();
        assert!(!saved.dirty);
        assert_eq!(
            saved.external_modification_state,
            ExternalModificationState::Unchanged
        );
        assert_eq!(
            fs::read_to_string(dir.join("notes.txt")).unwrap(),
            "recovered text\n"
        );
        wait_for("row removed after recreate", || {
            h.snapshot_row(&root, "notes.txt").is_none()
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recover_refuses_a_dirty_open_document_and_discard_removes_the_row() {
        let (_data, dir, h, ws, _) = with_inherited_snapshot("recover-dirty").await;
        let root = root_of(&dir);
        let doc = h.open_doc(&ws, "notes.txt").await;
        h.update(&doc.document.id, 1, "typed since launch\n").await;
        let refused = h.recover(&ws, "notes.txt").await.unwrap_err();
        assert_eq!(refused.code, CoreErrorCode::Conflict);
        assert_eq!(
            h.documents
                .snapshot(&doc.document.id)
                .unwrap()
                .text
                .as_deref(),
            Some("typed since launch\n")
        );

        h.session
            .discard(
                DiscardSnapshotRequest {
                    workspace_id: ws.id.clone(),
                    path: WorkspacePath::parse("notes.txt").unwrap(),
                },
                CorrelationId::new(),
            )
            .await
            .unwrap();
        // Our own newer snapshot may have been written meanwhile; wait for a settled state.
        h.flush().await;
        let row = h.snapshot_row(&root, "notes.txt");
        assert!(row.is_none_or(|row| row.text == "typed since launch\n"));
        let missing = h.recover(&ws, "README.md").await.unwrap_err();
        assert_eq!(missing.code, CoreErrorCode::NotFound);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn write_failures_back_off_and_report_status_transitions() {
        let data = TempDir::new("backoff");
        let dir = workspace_dir("backoff");
        let h = start(&data.join(DB_FILE_NAME), FAST);
        let ws = h.open_ws(&dir);
        let root = root_of(&dir);
        h.flush().await;
        let doc = h.open_doc(&ws, "notes.txt").await;
        h.database.as_ref().unwrap().inject_write_failures(2);
        h.update(&doc.document.id, 1, "unsaved\n").await;
        wait_for("unhealthy status", || {
            h.status_events().iter().any(|(_, healthy, _)| !healthy)
        })
        .await;
        assert!(!h.session.status().healthy);
        assert!(h.session.status().message.is_some());
        wait_for("row after retries", || {
            h.snapshot_row(&root, "notes.txt").is_some()
        })
        .await;
        wait_for("healthy again", || {
            h.status_events().iter().any(|(_, healthy, _)| *healthy)
        })
        .await;
        assert_eq!(
            h.status_events(),
            vec![(true, false, None), (true, true, None)],
            "one transition per streak"
        );
        assert!(h.session.status().healthy);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unavailable_store_reports_itself_and_keeps_commands_harmless() {
        let dir = workspace_dir("unavailable");
        let h = start_with(
            Path::new("/nonexistent/gritty.sqlite"),
            FAST,
            SessionStore::unavailable("disk on fire"),
        );
        let restored = h.session.restore(CorrelationId::new()).await;
        assert!(!restored.persistence.available);
        assert_eq!(
            restored.notices[0].kind,
            SessionNoticeKind::PersistenceUnavailable
        );
        assert!(restored.notices[0].message.contains("disk on fire"));
        let ws = h.open_ws(&dir);
        let doc = h.open_doc(&ws, "notes.txt").await;
        h.update(&doc.document.id, 1, "typing works\n").await;
        h.session
            .update(
                UpdateSessionRequest {
                    workspace_id: Some(ws.id.clone()),
                    tabs: Some(vec![]),
                    ..Default::default()
                },
                CorrelationId::new(),
            )
            .await
            .unwrap();
        let error = h
            .session
            .candidates(
                CandidatesRequest {
                    workspace_id: ws.id.clone(),
                },
                CorrelationId::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::Io);
        assert_eq!(error.details.unwrap()["persistence"]["available"], false);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_workspace_history_follows_what_was_opened() {
        let data = TempDir::new("recents-service");
        let db = data.join(DB_FILE_NAME);
        let h = start(&db, FAST);
        let first = workspace_dir("recents-one");
        let second = workspace_dir("recents-two");

        assert!(
            h.session
                .recents(RecentWorkspacesRequest::default(), CorrelationId::new())
                .await
                .unwrap()
                .is_empty(),
            "nothing has been opened yet"
        );

        // The roots the history records are the canonical ones the manager
        // resolved, not the paths handed to it.
        let one = h.open_ws(&first);
        let two = h.open_ws(&second);
        assert!(h.session.flush(Duration::from_secs(2)).await);

        let entries = h
            .session
            .recents(RecentWorkspacesRequest::default(), CorrelationId::new())
            .await
            .unwrap();
        // Set, not sequence: two opens this close together share a millisecond
        // on a fast machine and the root breaks the tie. Recency ordering is
        // pinned deterministically in `workspace::recents`; what this test
        // owns is that opening a workspace records its *canonical* root at all.
        let mut roots = entries.iter().map(|e| e.root.as_str()).collect::<Vec<_>>();
        roots.sort_unstable();
        let mut expected = vec![one.root.as_str(), two.root.as_str()];
        expected.sort_unstable();
        assert_eq!(roots, expected);
        assert!(entries.iter().all(|entry| entry.exists));
        assert!(entries.iter().any(|entry| entry.name == two.name));

        h.session
            .forget(
                ForgetWorkspaceRequest {
                    root: two.root.clone(),
                },
                CorrelationId::new(),
            )
            .await
            .unwrap();
        let after = h
            .session
            .recents(RecentWorkspacesRequest::default(), CorrelationId::new())
            .await
            .unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].root, one.root);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unavailable_store_has_an_empty_history_rather_than_an_error() {
        let h = start_with(
            Path::new("/nonexistent/gritty.sqlite"),
            FAST,
            SessionStore::unavailable("disk on fire"),
        );
        // The startup screen renders before it knows whether the database
        // opened; an error here would replace the launcher with a stack trace.
        assert!(
            h.session
                .recents(RecentWorkspacesRequest::default(), CorrelationId::new())
                .await
                .unwrap()
                .is_empty()
        );
        // Forgetting, by contrast, is an action the user took and must not
        // silently appear to succeed.
        let error = h
            .session
            .forget(
                ForgetWorkspaceRequest {
                    root: "/anything".into(),
                },
                CorrelationId::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::Io);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unavailable_store_rejects_recovery_commands_with_io() {
        let dir = workspace_dir("unavailable-io");
        let h = start_with(
            Path::new("/nonexistent/gritty.sqlite"),
            FAST,
            SessionStore::unavailable("disk on fire"),
        );
        let ws = h.open_ws(&dir);
        let error = h.recover(&ws, "notes.txt").await.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::Io);
        assert!(!h.session.flush(Duration::from_millis(50)).await);
        assert!(h.session.window_state("main").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn durable_operations_are_recorded_and_interrupted_on_the_next_start() {
        let data = TempDir::new("durable");
        let db = data.join(DB_FILE_NAME);
        let h = start(&db, FAST);
        let operation = h.registry.start(
            OperationSpec {
                kind: OperationKind::new("test.durable"),
                actor: Actor::User,
                workspace_id: None,
                correlation_id: CorrelationId::new(),
                durable: true,
            },
            |ctx| async move {
                ctx.cancelled().await;
                Err(CoreError::cancelled("stopped"))
            },
        );
        let id = operation.id.to_string();
        let running = |h: &Harness| {
            h.reader()
                .read(|c| operations::get(c, &id))
                .unwrap()
                .map(|row| row.status)
        };
        wait_for("RUNNING row", || running(&h).as_deref() == Some("RUNNING")).await;
        // A non-durable operation leaves no row.
        let ephemeral = h.registry.start(
            OperationSpec {
                kind: OperationKind::new("test.ephemeral"),
                actor: Actor::User,
                workspace_id: None,
                correlation_id: CorrelationId::new(),
                durable: false,
            },
            |_ctx| async move { Ok(None) },
        );
        h.flush().await;
        assert!(
            h.reader()
                .read(|c| operations::get(c, &ephemeral.id.to_string()))
                .unwrap()
                .is_none()
        );

        // "Next start": the store reconciles before any writer runs.
        std::mem::forget(h);
        let next = SessionStore::open(&db);
        assert_eq!(next.notices().len(), 1);
        assert_eq!(
            next.notices()[0].kind,
            SessionNoticeKind::OperationsInterrupted
        );
        let row = next
            .database()
            .unwrap()
            .read(|c| operations::get(c, &id))
            .unwrap()
            .unwrap();
        assert_eq!(row.status, operations::INTERRUPTED);
        assert!(row.finished_at.is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_garbage_database_is_moved_aside_and_reported_by_restore() {
        let data = TempDir::new("garbage");
        let db = data.join(DB_FILE_NAME);
        fs::write(&db, b"not a database").unwrap();
        let h = start(&db, FAST);
        let restored = h.session.restore(CorrelationId::new()).await;
        assert!(restored.persistence.available);
        assert_eq!(restored.notices.len(), 1);
        assert_eq!(
            restored.notices[0].kind,
            SessionNoticeKind::DatabaseMovedAside
        );
        let aside = restored.notices[0].path.clone().unwrap();
        assert!(Path::new(&aside).exists(), "{aside}");
        assert!(aside.contains("gritty.sqlite.corrupt-"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_damaged_database_is_replaced_by_the_integrity_check_and_announced() {
        use std::io::{Seek, SeekFrom, Write};
        let data = TempDir::new("damaged");
        let db = data.join(DB_FILE_NAME);
        {
            let opened = Database::open(&db).unwrap();
            opened
                .database
                .write(|tx| {
                    snapshots::upsert(
                        tx,
                        "/repo",
                        &snapshots::SnapshotRow {
                            path: "big.txt".into(),
                            text: "x".repeat(256 * 1024),
                            encoding: "utf-8".into(),
                            saved_hash: "sha256:0".into(),
                            current_version: 1,
                            updated_at: 1,
                        },
                    )
                })
                .unwrap();
        }
        let mut file = fs::OpenOptions::new().write(true).open(&db).unwrap();
        file.seek(SeekFrom::Start(4096 * 12)).unwrap();
        file.write_all(&[0xFF; 4096]).unwrap();
        drop(file);

        let h = start(&db, FAST);
        wait_for("replacement announced", || {
            h.status_events()
                .iter()
                .any(|(_, _, notice)| *notice == Some(SessionNoticeKind::DatabaseMovedAside))
        })
        .await;
        assert!(h.session.status().available && h.session.status().healthy);
        assert!(
            h.session
                .notices()
                .iter()
                .any(|n| n.kind == SessionNoticeKind::DatabaseMovedAside)
        );
        assert_eq!(
            h.reader().read(snapshots::count).unwrap(),
            0,
            "fresh database"
        );
        // The replacement is usable: a snapshot lands in the new file.
        let dir = workspace_dir("damaged");
        let ws = h.open_ws(&dir);
        let doc = h.open_doc(&ws, "notes.txt").await;
        h.update(&doc.document.id, 1, "after replacement\n").await;
        wait_for("row in the new database", || {
            h.snapshot_row(&root_of(&dir), "notes.txt").is_some()
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn window_geometry_round_trips_through_the_writer() {
        let data = TempDir::new("window");
        let h = start(&data.join(DB_FILE_NAME), FAST);
        assert!(h.session.window_state("main").is_none());
        h.session.record_window(WindowGeometry {
            label: "main".into(),
            x: Some(10),
            y: Some(20),
            width: 1280,
            height: 800,
            scale_factor: 2.0,
            maximized: false,
        });
        // The last geometry within the debounce wins.
        let geometry = WindowGeometry {
            label: "main".into(),
            x: Some(30),
            y: Some(40),
            width: 1000,
            height: 700,
            scale_factor: 2.0,
            maximized: true,
        };
        h.session.record_window(geometry.clone());
        h.flush().await;
        assert_eq!(h.session.window_state("main"), Some(geometry));
        assert!(h.session.window_state("other").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn update_validates_its_input() {
        let data = TempDir::new("validate");
        let dir = workspace_dir("validate");
        let h = start(&data.join(DB_FILE_NAME), FAST);
        let ws = h.open_ws(&dir);
        let tab = SessionTab {
            path: WorkspacePath::parse("README.md").unwrap(),
            active: true,
            view_mode: None,
            cursor: None,
            anchor: None,
            scroll_line: None,
        };
        let without_workspace = h
            .session
            .update(
                UpdateSessionRequest {
                    tabs: Some(vec![tab.clone()]),
                    ..Default::default()
                },
                CorrelationId::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(without_workspace.code, CoreErrorCode::InvalidInput);
        let unknown = h
            .session
            .update(
                UpdateSessionRequest {
                    workspace_id: Some(crate::protocol::ids::WorkspaceId::from_raw("nope")),
                    tabs: Some(vec![tab.clone()]),
                    ..Default::default()
                },
                CorrelationId::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(unknown.code, CoreErrorCode::NotFound);
        let huge = h
            .session
            .update(
                UpdateSessionRequest {
                    layout: Some(serde_json::json!({ "blob": "x".repeat(MAX_JSON_BYTES) })),
                    ..Default::default()
                },
                CorrelationId::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(huge.code, CoreErrorCode::InvalidInput);
        let too_many = h
            .session
            .update(
                UpdateSessionRequest {
                    workspace_id: Some(ws.id.clone()),
                    tabs: Some(vec![tab.clone(); MAX_TABS + 1]),
                    ..Default::default()
                },
                CorrelationId::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(too_many.code, CoreErrorCode::InvalidInput);
        // A valid update only touches what it carries.
        h.session
            .update(
                UpdateSessionRequest {
                    workspace_id: Some(ws.id.clone()),
                    tabs: Some(vec![tab]),
                    preferences: Some(serde_json::json!({ "version": 1 })),
                    layout: None,
                },
                CorrelationId::new(),
            )
            .await
            .unwrap();
        let restored = h.session.restore(CorrelationId::new()).await;
        assert_eq!(restored.tabs.len(), 1);
        assert_eq!(restored.preferences.unwrap()["version"], 1);
        assert!(restored.layout.is_none());
    }
}
