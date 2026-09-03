//! Documents and buffers (T030, Gate B; addendum §4).
//!
//! A repository file and an open document are different things. The core owns
//! the buffer of every open document: the UI (Monaco later, a textarea now)
//! renders and edits it but is never the source of truth. What the model
//! guarantees:
//!
//! - **Identity**: one document per `(workspace, path)`; opening an open path
//!   returns the same document with its *current* buffer, not the disk.
//! - **Versions**: every accepted `documents.update` increments
//!   `currentVersion`; an update whose `baseVersion` is stale is rejected with
//!   `Conflict`, which protects a reload from being clobbered by an in-flight
//!   edit. `savedRevision` counts the times the buffer was synchronized with
//!   the disk (open, save, reload).
//! - **Dirty** is derived, never stored: `hash(buffer) != savedHash`.
//! - **External modification** is checked on demand (`documents.status`, before
//!   every save, on reload) by hashing the file; the internal disk state
//!   (unchanged / modified / deleted) combines with `dirty` into the wire
//!   `externalModificationState`. Transitions emit
//!   `DocumentExternalChangeDetected`; nothing is ever resolved silently: a
//!   conflicting save fails unless the UI passes `force`, a reload happens only
//!   when asked, a dirty document is closed only with `force`.
//! - **Saves are atomic** (`gritty_workspace::write_file_atomic`); a failed save
//!   leaves the buffer, the dirty flag and the destination untouched.
//! - **Binary** files (NUL bytes or invalid UTF-8) get no buffer and cannot be
//!   edited or saved; **large** files (≥ 10 MiB) open read-only until the UI
//!   asks for editing; files above 64 MiB are refused.
//!
//! Disk work runs on the blocking pool and is bounded by the size limit, so it
//! is not modelled as a cancellable operation: a save must run to completion
//! (the rename is the atomic step) and a bounded read is at most a few hundred
//! milliseconds. Per document, disk work is serialized by an async lock; the
//! in-memory state is protected by a short-held mutex that is never held across
//! an await or an I/O call.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use gritty_workspace::{
    AtomicWriteFault, ContentHash, DiskProbe, DocumentEncoding, FileSnapshot, FileStat,
    LARGE_FILE_THRESHOLD, MAX_DOCUMENT_BYTES, WorkspaceError, WorkspacePath, decode_text,
    encode_text, language_id, probe_file, read_file, write_file_atomic,
};
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use ts_rs::TS;

use crate::event::{
    CoreEvent, DocumentChangedPayload, DocumentClosedPayload, DocumentExternalChangePayload,
    DocumentOpenedPayload, DocumentSavedPayload, EventBus, EventOrigin,
};
use crate::protocol::actor::Actor;
use crate::protocol::error::{CoreError, CoreErrorCode};
use crate::protocol::ids::{CorrelationId, DocumentId, WorkspaceId};
use crate::workspace::{
    UnsavedPolicy, Workspace, WorkspaceCloseGuard, WorkspaceClosedReason, WorkspaceManager,
};

/// Texts longer than this are hashed on the blocking pool instead of inline.
const INLINE_HASH_LIMIT: usize = 256 * 1024;

/// The combined view of the disk and the buffer (addendum §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExternalModificationState {
    /// The file on disk still has `savedHash` (as far as the last check knows).
    Unchanged,
    /// The file changed on disk and the buffer is clean: reload is safe.
    ModifiedExternally,
    /// The file changed on disk *and* the buffer has unsaved edits. Saving
    /// needs `force`; reloading drops the edits.
    Conflicted,
    /// The file is gone. Saving recreates it only with `force`.
    DeletedExternally,
}

/// An open document as the UI sees it. Never carries the text: that travels in
/// [`DocumentContent`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: DocumentId,
    pub workspace_id: WorkspaceId,
    pub path: WorkspacePath,
    pub encoding: DocumentEncoding,
    /// VS Code / Monaco style identifier (`docs/architecture/documents.md`).
    pub language_id: String,
    /// Times the buffer was synchronized with the disk: 1 after open, +1 per
    /// save or reload.
    pub saved_revision: u64,
    /// Hash of the bytes the buffer was last synchronized with.
    pub saved_hash: ContentHash,
    /// 1 after open, +1 per accepted update or reload. Updates must quote it.
    pub current_version: u64,
    /// `hash(buffer) != savedHash`. Always `false` for binary documents.
    pub dirty: bool,
    /// Edits are refused: the file has no write permission, or it is large and
    /// editing was not enabled.
    pub read_only: bool,
    /// NUL bytes or invalid UTF-8: no buffer, nothing to edit or save.
    pub binary: bool,
    /// At or above the large-file threshold; the UI degrades expensive features.
    pub large_file_mode: bool,
    pub external_modification_state: ExternalModificationState,
    /// Byte size on disk when the buffer was last synchronized.
    pub size: u64,
}

/// The document plus its text (absent for binary documents).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DocumentContent {
    pub document: Document,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text: Option<String>,
}

/// `documents.open`: open (or return the already open) document at `path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OpenDocumentRequest {
    pub workspace_id: WorkspaceId,
    pub path: WorkspacePath,
    /// Let a large file be edited (it stays in `largeFileMode`). Also lifts the
    /// read-only flag of an already open large document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub large_file_editing: Option<bool>,
}

/// `documents.close`. A dirty document is closed only with `force`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CloseDocumentRequest {
    pub document_id: DocumentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub force: Option<bool>,
}

/// `documents.update`: replace the buffer with `text`. Full text for now; the
/// upgrade path to deltas keeps `baseVersion` and adds an `edits` array next
/// to `text` (addendum §4, T040 follow-up).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDocumentRequest {
    pub document_id: DocumentId,
    /// The `currentVersion` the text was derived from.
    pub base_version: u64,
    pub text: String,
}

/// `documents.save`. `force` overwrites a file that changed or disappeared
/// since the buffer was synchronized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SaveDocumentRequest {
    pub document_id: DocumentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub force: Option<bool>,
}

/// `documents.reload`: replace the buffer with the disk content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReloadDocumentRequest {
    pub document_id: DocumentId,
}

/// `documents.status`: compare the disk with `savedHash` for one document, or
/// for every open document when `documentId` is absent.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DocumentStatusRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub document_id: Option<DocumentId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DocumentClosedReason {
    User,
    /// The workspace closed (user, replaced, or root missing).
    WorkspaceClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DocumentChangeCause {
    Edit,
    Reload,
    ReadOnlyChanged,
}

/// Size bounds; tests lower them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentLimits {
    pub large_file_threshold: u64,
    pub max_bytes: u64,
}

impl Default for DocumentLimits {
    fn default() -> Self {
        Self {
            large_file_threshold: LARGE_FILE_THRESHOLD,
            max_bytes: MAX_DOCUMENT_BYTES,
        }
    }
}

/// What the last disk check found, independent of the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiskState {
    Unchanged,
    Modified,
    Deleted,
}

impl DiskState {
    fn wire(self, dirty: bool) -> ExternalModificationState {
        match (self, dirty) {
            (Self::Unchanged, _) => ExternalModificationState::Unchanged,
            (Self::Modified, false) => ExternalModificationState::ModifiedExternally,
            (Self::Modified, true) => ExternalModificationState::Conflicted,
            (Self::Deleted, _) => ExternalModificationState::DeletedExternally,
        }
    }

    fn from_probe(probe: &DiskProbe, saved_hash: &ContentHash) -> Self {
        match probe {
            DiskProbe::Missing => Self::Deleted,
            DiskProbe::Present { hash, .. } if hash.as_ref() == Some(saved_hash) => Self::Unchanged,
            DiskProbe::Present { .. } => Self::Modified,
        }
    }
}

struct Buffer {
    text: String,
    /// Hash of `encode_text(text, encoding)`.
    hash: ContentHash,
}

struct Record {
    document: Document,
    buffer: Option<Buffer>,
    disk: DiskState,
    file_read_only: bool,
    large_editing_enabled: bool,
}

impl Record {
    fn new(
        id: DocumentId,
        workspace_id: WorkspaceId,
        path: WorkspacePath,
        snapshot: FileSnapshot,
        limits: &DocumentLimits,
        large_editing_enabled: bool,
    ) -> Self {
        let language = language_id(&path).to_owned();
        let mut record = Self {
            document: Document {
                id,
                workspace_id,
                path,
                encoding: DocumentEncoding::Utf8,
                language_id: language,
                saved_revision: 0,
                saved_hash: snapshot.hash.clone(),
                current_version: 0,
                dirty: false,
                read_only: false,
                binary: false,
                large_file_mode: false,
                external_modification_state: ExternalModificationState::Unchanged,
                size: 0,
            },
            buffer: None,
            disk: DiskState::Unchanged,
            file_read_only: false,
            large_editing_enabled,
        };
        record.synchronize(snapshot, limits);
        record
    }

    /// Replace everything the disk determines from a fresh read (open, reload).
    fn synchronize(&mut self, snapshot: FileSnapshot, limits: &DocumentLimits) {
        let FileSnapshot { bytes, hash, stat } = snapshot;
        match decode_text(&bytes) {
            Some((text, encoding)) => {
                self.document.encoding = encoding;
                self.document.binary = false;
                self.buffer = Some(Buffer {
                    text,
                    hash: hash.clone(),
                });
            }
            None => {
                self.document.encoding = DocumentEncoding::Binary;
                self.document.binary = true;
                self.buffer = None;
            }
        }
        self.document.large_file_mode = stat.size >= limits.large_file_threshold;
        self.document.saved_hash = hash;
        self.document.saved_revision += 1;
        self.document.current_version += 1;
        self.document.size = stat.size;
        self.file_read_only = stat.read_only;
        self.disk = DiskState::Unchanged;
        self.refresh();
    }

    /// Record a completed write.
    fn saved(&mut self, hash: ContentHash, stat: FileStat) {
        self.document.saved_hash = hash;
        self.document.saved_revision += 1;
        self.document.size = stat.size;
        self.file_read_only = stat.read_only;
        self.disk = DiskState::Unchanged;
        self.refresh();
    }

    /// Recompute the derived wire fields.
    fn refresh(&mut self) {
        self.document.dirty = self
            .buffer
            .as_ref()
            .is_some_and(|buffer| buffer.hash != self.document.saved_hash);
        self.document.read_only =
            self.file_read_only || (self.document.large_file_mode && !self.large_editing_enabled);
        self.document.external_modification_state = self.disk.wire(self.document.dirty);
    }

    fn content(&self) -> DocumentContent {
        DocumentContent {
            document: self.document.clone(),
            text: self.buffer.as_ref().map(|buffer| buffer.text.clone()),
        }
    }

    /// Apply a disk check; returns the previous wire state when it changed.
    fn observe_disk(&mut self, disk: DiskState) -> Option<ExternalModificationState> {
        let previous = self.document.external_modification_state;
        self.disk = disk;
        self.refresh();
        (self.document.external_modification_state != previous).then_some(previous)
    }
}

struct Entry {
    record: Mutex<Record>,
    /// Serializes disk work (save, reload, status) per document.
    io: tokio::sync::Mutex<()>,
}

impl Entry {
    fn lock(&self) -> MutexGuard<'_, Record> {
        self.record.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Default)]
struct Table {
    by_id: HashMap<DocumentId, Arc<Entry>>,
    by_path: HashMap<(WorkspaceId, WorkspacePath), DocumentId>,
}

impl Table {
    fn remove(&mut self, id: &DocumentId) -> Option<Arc<Entry>> {
        let entry = self.by_id.remove(id)?;
        let record = entry.lock();
        self.by_path.remove(&(
            record.document.workspace_id.clone(),
            record.document.path.clone(),
        ));
        drop(record);
        Some(entry)
    }
}

pub struct DocumentManager {
    table: Mutex<Table>,
    events: Arc<EventBus>,
    workspaces: Arc<WorkspaceManager>,
    handle: Handle,
    limits: DocumentLimits,
    #[cfg(test)]
    write_fault: Mutex<AtomicWriteFault>,
}

enum SaveOutcome {
    /// The disk differs from `savedHash` and `force` was not given.
    Conflict(DiskState),
    /// Nothing to write: clean buffer, unchanged disk.
    NothingToDo,
    Written {
        hash: ContentHash,
        stat: FileStat,
    },
}

impl DocumentManager {
    /// Build the manager and install it as the workspace close guard.
    pub fn new(
        handle: Handle,
        events: Arc<EventBus>,
        workspaces: Arc<WorkspaceManager>,
        limits: DocumentLimits,
    ) -> Arc<Self> {
        let manager = Arc::new(Self {
            table: Mutex::new(Table::default()),
            events,
            workspaces: Arc::clone(&workspaces),
            handle,
            limits,
            #[cfg(test)]
            write_fault: Mutex::new(AtomicWriteFault::None),
        });
        let as_guard: Arc<dyn WorkspaceCloseGuard> = manager.clone();
        let guard: Weak<dyn WorkspaceCloseGuard> = Arc::downgrade(&as_guard);
        drop(as_guard);
        workspaces.set_close_guard(guard);
        manager
    }

    pub fn limits(&self) -> DocumentLimits {
        self.limits
    }

    /// Every open document, for rehydration after a renderer reload (the text
    /// then comes from `open`, which returns the current buffer).
    pub fn list(&self) -> Vec<Document> {
        let table = self.lock_table();
        let mut documents: Vec<Document> = table
            .by_id
            .values()
            .map(|entry| entry.lock().document.clone())
            .collect();
        documents.sort_by(|a, b| a.id.cmp(&b.id));
        documents
    }

    pub async fn open(
        &self,
        request: OpenDocumentRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<DocumentContent, CoreError> {
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        if request.path.is_root() {
            return Err(CoreError::invalid_input("path", "must name a file"));
        }
        let enable_large = request.large_file_editing == Some(true);
        if let Some(entry) = self.lookup_path(&workspace.id, &request.path) {
            return Ok(self.reopen(&entry, enable_large, &actor, &correlation));
        }

        let snapshot = {
            let root = Arc::clone(&root);
            let path = request.path.clone();
            let max = self.limits.max_bytes;
            self.blocking(move || read_file(&root, &path, max)).await?
        };
        let record = Record::new(
            DocumentId::new(),
            workspace.id.clone(),
            request.path.clone(),
            snapshot,
            &self.limits,
            enable_large,
        );
        let entry = Arc::new(Entry {
            record: Mutex::new(record),
            io: tokio::sync::Mutex::new(()),
        });
        let content = {
            let mut table = self.lock_table();
            let key = (workspace.id.clone(), request.path.clone());
            if let Some(existing) = table.by_path.get(&key).and_then(|id| table.by_id.get(id)) {
                // A concurrent open of the same path won; keep one document.
                let existing = Arc::clone(existing);
                drop(table);
                return Ok(self.reopen(&existing, enable_large, &actor, &correlation));
            }
            let record = entry.lock();
            table.by_path.insert(key, record.document.id.clone());
            table
                .by_id
                .insert(record.document.id.clone(), Arc::clone(&entry));
            record.content()
        };
        tracing::info!(
            document = %content.document.id,
            workspace = %workspace.id,
            path = %content.document.path,
            encoding = %content.document.encoding,
            size = content.document.size,
            large = content.document.large_file_mode,
            "document opened"
        );
        self.emit(
            &workspace.id,
            &actor,
            &correlation,
            CoreEvent::DocumentOpened(DocumentOpenedPayload {
                document: content.document.clone(),
            }),
        );
        Ok(content)
    }

    fn reopen(
        &self,
        entry: &Entry,
        enable_large: bool,
        actor: &Actor,
        correlation: &CorrelationId,
    ) -> DocumentContent {
        let mut record = entry.lock();
        let lifted =
            enable_large && record.document.large_file_mode && !record.large_editing_enabled;
        if lifted {
            record.large_editing_enabled = true;
            record.refresh();
        }
        let content = record.content();
        drop(record);
        if lifted {
            self.emit(
                &content.document.workspace_id,
                actor,
                correlation,
                CoreEvent::DocumentChanged(DocumentChangedPayload {
                    document: content.document.clone(),
                    cause: DocumentChangeCause::ReadOnlyChanged,
                }),
            );
        }
        content
    }

    pub fn close(
        &self,
        request: CloseDocumentRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Document, CoreError> {
        let force = request.force == Some(true);
        let document = {
            let mut table = self.lock_table();
            let entry = table
                .by_id
                .get(&request.document_id)
                .ok_or_else(|| not_open(&request.document_id))?;
            let record = entry.lock();
            if record.document.dirty && !force {
                return Err(CoreError::conflict(format!(
                    "{} has unsaved changes; save it or close with force",
                    record.document.path
                ))
                .with_details(serde_json::json!({
                    "documentId": record.document.id,
                    "dirty": true,
                })));
            }
            let document = record.document.clone();
            drop(record);
            table.remove(&request.document_id);
            document
        };
        self.emit_closed(
            &document,
            DocumentClosedReason::User,
            &actor,
            Some(&correlation),
        );
        Ok(document)
    }

    pub async fn update(
        &self,
        request: UpdateDocumentRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Document, CoreError> {
        let entry = self.require_entry(&request.document_id)?;
        if request.text.len() as u64 > self.limits.max_bytes {
            return Err(CoreError::invalid_input(
                "text",
                format!(
                    "is {} bytes, above the {}-byte document limit",
                    request.text.len(),
                    self.limits.max_bytes
                ),
            ));
        }
        let encoding = {
            let record = entry.lock();
            check_editable(&record.document, request.base_version)?;
            record.document.encoding
        };
        let (text, hash) = self.hash_text(request.text, encoding).await?;
        let document = {
            let mut record = entry.lock();
            // The version may have moved while the text was being hashed.
            check_editable(&record.document, request.base_version)?;
            record.buffer = Some(Buffer { text, hash });
            record.document.current_version += 1;
            record.refresh();
            record.document.clone()
        };
        tracing::debug!(
            document = %document.id,
            version = document.current_version,
            dirty = document.dirty,
            "document updated"
        );
        self.emit(
            &document.workspace_id,
            &actor,
            &correlation,
            CoreEvent::DocumentChanged(DocumentChangedPayload {
                document: document.clone(),
                cause: DocumentChangeCause::Edit,
            }),
        );
        Ok(document)
    }

    pub async fn save(
        &self,
        request: SaveDocumentRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Document, CoreError> {
        let entry = self.require_entry(&request.document_id)?;
        let _io = entry.io.lock().await;
        let (workspace_id, path, bytes, saved_hash, dirty) = {
            let record = entry.lock();
            let Some(buffer) = &record.buffer else {
                return Err(CoreError::invalid_input(
                    "documentId",
                    format!(
                        "{} is binary and has no buffer to save",
                        record.document.path
                    ),
                ));
            };
            (
                record.document.workspace_id.clone(),
                record.document.path.clone(),
                encode_text(&buffer.text, record.document.encoding),
                record.document.saved_hash.clone(),
                record.document.dirty,
            )
        };
        let (_, root) = self.workspaces.require(&workspace_id, &correlation)?;
        let force = request.force == Some(true);
        let max = self.limits.max_bytes;
        let fault = self.write_fault();
        let outcome = self
            .blocking(move || -> Result<SaveOutcome, WorkspaceError> {
                let probe = probe_file(&root, &path, max)?;
                let disk = DiskState::from_probe(&probe, &saved_hash);
                if disk != DiskState::Unchanged && !force {
                    return Ok(SaveOutcome::Conflict(disk));
                }
                if disk == DiskState::Unchanged && !dirty {
                    return Ok(SaveOutcome::NothingToDo);
                }
                let target = root.resolve_for_write(&path)?;
                let stat = write_file_atomic(target.canonical(), &bytes, fault)?;
                Ok(SaveOutcome::Written {
                    hash: ContentHash::of(&bytes),
                    stat,
                })
            })
            .await?;
        match outcome {
            SaveOutcome::Conflict(disk) => {
                let document = self.observe(&entry, disk, &actor, &correlation);
                Err(CoreError::conflict(format!(
                    "{} changed on disk since it was opened ({}); reload it or save with force",
                    document.path,
                    document.external_modification_state.as_str()
                ))
                .with_details(serde_json::json!({
                    "documentId": document.id,
                    "externalModificationState": document.external_modification_state,
                })))
            }
            SaveOutcome::NothingToDo => Ok(entry.lock().document.clone()),
            SaveOutcome::Written { hash, stat } => {
                let document = {
                    let mut record = entry.lock();
                    record.saved(hash, stat);
                    record.document.clone()
                };
                tracing::info!(
                    document = %document.id,
                    path = %document.path,
                    revision = document.saved_revision,
                    size = document.size,
                    force,
                    "document saved"
                );
                self.emit(
                    &document.workspace_id,
                    &actor,
                    &correlation,
                    CoreEvent::DocumentSaved(DocumentSavedPayload {
                        document: document.clone(),
                    }),
                );
                Ok(document)
            }
        }
    }

    pub async fn reload(
        &self,
        request: ReloadDocumentRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<DocumentContent, CoreError> {
        let entry = self.require_entry(&request.document_id)?;
        let _io = entry.io.lock().await;
        let (workspace_id, path) = {
            let record = entry.lock();
            (
                record.document.workspace_id.clone(),
                record.document.path.clone(),
            )
        };
        let (_, root) = self.workspaces.require(&workspace_id, &correlation)?;
        let max = self.limits.max_bytes;
        let read = {
            let path = path.clone();
            self.handle
                .spawn_blocking(move || read_file(&root, &path, max))
                .await
                .map_err(|join| CoreError::internal(format!("document read task failed: {join}")))?
        };
        let snapshot = match read {
            Ok(snapshot) => snapshot,
            Err(WorkspaceError::NotFound(message)) => {
                self.observe(&entry, DiskState::Deleted, &actor, &correlation);
                return Err(CoreError::not_found(message));
            }
            Err(error) => return Err(error.into()),
        };
        let content = {
            let mut record = entry.lock();
            record.synchronize(snapshot, &self.limits);
            record.content()
        };
        tracing::info!(document = %content.document.id, path = %path, "document reloaded from disk");
        self.emit(
            &workspace_id,
            &actor,
            &correlation,
            CoreEvent::DocumentChanged(DocumentChangedPayload {
                document: content.document.clone(),
                cause: DocumentChangeCause::Reload,
            }),
        );
        Ok(content)
    }

    pub async fn status(
        &self,
        request: DocumentStatusRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Vec<Document>, CoreError> {
        let entries: Vec<Arc<Entry>> = match &request.document_id {
            Some(id) => vec![self.require_entry(id)?],
            None => {
                let table = self.lock_table();
                let mut entries: Vec<_> = table.by_id.values().cloned().collect();
                entries.sort_by_key(|entry| entry.lock().document.id.clone());
                entries
            }
        };
        let mut documents = Vec::with_capacity(entries.len());
        for entry in entries {
            match self.check_disk(&entry, &actor, &correlation).await {
                Ok(document) => documents.push(document),
                // Checking everything: a document whose workspace vanished was
                // closed by the guard while we iterated; skip it.
                Err(error)
                    if request.document_id.is_none() && error.code == CoreErrorCode::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(documents)
    }

    async fn check_disk(
        &self,
        entry: &Arc<Entry>,
        actor: &Actor,
        correlation: &CorrelationId,
    ) -> Result<Document, CoreError> {
        let _io = entry.io.lock().await;
        let (workspace_id, path, saved_hash) = {
            let record = entry.lock();
            (
                record.document.workspace_id.clone(),
                record.document.path.clone(),
                record.document.saved_hash.clone(),
            )
        };
        let (_, root) = self.workspaces.require(&workspace_id, correlation)?;
        let max = self.limits.max_bytes;
        let probe = self.blocking(move || probe_file(&root, &path, max)).await?;
        Ok(self.observe(
            entry,
            DiskState::from_probe(&probe, &saved_hash),
            actor,
            correlation,
        ))
    }

    /// Apply a disk observation and announce a state transition.
    fn observe(
        &self,
        entry: &Entry,
        disk: DiskState,
        actor: &Actor,
        correlation: &CorrelationId,
    ) -> Document {
        let (document, previous) = {
            let mut record = entry.lock();
            let previous = record.observe_disk(disk);
            (record.document.clone(), previous)
        };
        if let Some(previous_state) = previous {
            tracing::info!(
                document = %document.id,
                path = %document.path,
                from = previous_state.as_str(),
                to = document.external_modification_state.as_str(),
                "external change detected"
            );
            self.emit(
                &document.workspace_id,
                // The change itself came from outside Gritty; the check may
                // have been triggered by the user.
                &Actor::External,
                correlation,
                CoreEvent::DocumentExternalChangeDetected(DocumentExternalChangePayload {
                    document: document.clone(),
                    previous_state,
                }),
            );
            let _ = actor;
        }
        document
    }

    async fn hash_text(
        &self,
        text: String,
        encoding: DocumentEncoding,
    ) -> Result<(String, ContentHash), CoreError> {
        if text.len() <= INLINE_HASH_LIMIT {
            let hash = ContentHash::of_text(&text, encoding);
            return Ok((text, hash));
        }
        self.handle
            .spawn_blocking(move || {
                let hash = ContentHash::of_text(&text, encoding);
                (text, hash)
            })
            .await
            .map_err(|join| CoreError::internal(format!("hashing task failed: {join}")))
    }

    async fn blocking<T, F>(&self, work: F) -> Result<T, CoreError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, WorkspaceError> + Send + 'static,
    {
        self.handle
            .spawn_blocking(work)
            .await
            .map_err(|join| CoreError::internal(format!("document task failed: {join}")))?
            .map_err(CoreError::from)
    }

    fn require_entry(&self, id: &DocumentId) -> Result<Arc<Entry>, CoreError> {
        self.lock_table()
            .by_id
            .get(id)
            .cloned()
            .ok_or_else(|| not_open(id))
    }

    fn lookup_path(&self, workspace_id: &WorkspaceId, path: &WorkspacePath) -> Option<Arc<Entry>> {
        let table = self.lock_table();
        let id = table.by_path.get(&(workspace_id.clone(), path.clone()))?;
        table.by_id.get(id).cloned()
    }

    fn lock_table(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(
        &self,
        workspace_id: &WorkspaceId,
        actor: &Actor,
        correlation: &CorrelationId,
        event: CoreEvent,
    ) {
        self.events.emit(
            &EventOrigin {
                workspace_id: Some(workspace_id.clone()),
                correlation_id: Some(correlation.clone()),
                actor: actor.clone(),
            },
            event,
        );
    }

    fn emit_closed(
        &self,
        document: &Document,
        reason: DocumentClosedReason,
        actor: &Actor,
        correlation: Option<&CorrelationId>,
    ) {
        if document.dirty {
            tracing::warn!(
                document = %document.id,
                path = %document.path,
                ?reason,
                "document closed with unsaved changes"
            );
        } else {
            tracing::info!(document = %document.id, path = %document.path, ?reason, "document closed");
        }
        self.events.emit(
            &EventOrigin {
                workspace_id: Some(document.workspace_id.clone()),
                correlation_id: correlation.cloned(),
                actor: actor.clone(),
            },
            CoreEvent::DocumentClosed(DocumentClosedPayload {
                document_id: document.id.clone(),
                workspace_id: document.workspace_id.clone(),
                path: document.path.clone(),
                reason,
                dirty: document.dirty,
            }),
        );
    }

    #[cfg(test)]
    fn write_fault(&self) -> AtomicWriteFault {
        *self
            .write_fault
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(not(test))]
    fn write_fault(&self) -> AtomicWriteFault {
        AtomicWriteFault::None
    }
}

impl WorkspaceCloseGuard for DocumentManager {
    fn before_close(
        &self,
        workspace: &Workspace,
        reason: WorkspaceClosedReason,
        policy: UnsavedPolicy,
    ) -> Result<(), CoreError> {
        if policy == UnsavedPolicy::Discard {
            return Ok(());
        }
        let table = self.lock_table();
        let dirty: Vec<serde_json::Value> = table
            .by_id
            .values()
            .filter_map(|entry| {
                let record = entry.lock();
                (record.document.workspace_id == workspace.id && record.document.dirty).then(|| {
                    serde_json::json!({
                        "documentId": record.document.id,
                        "path": record.document.path,
                    })
                })
            })
            .collect();
        if dirty.is_empty() {
            return Ok(());
        }
        Err(CoreError::conflict(format!(
            "{} document(s) have unsaved changes; save or close them, or pass force",
            dirty.len()
        ))
        .with_details(serde_json::json!({
            "workspaceId": workspace.id,
            "reason": reason,
            "dirtyDocuments": dirty,
        })))
    }

    fn after_close(
        &self,
        workspace: &Workspace,
        _reason: WorkspaceClosedReason,
        actor: &Actor,
        correlation: Option<&CorrelationId>,
    ) {
        let closed: Vec<Document> = {
            let mut table = self.lock_table();
            let ids: Vec<DocumentId> = table
                .by_id
                .iter()
                .filter(|(_, entry)| entry.lock().document.workspace_id == workspace.id)
                .map(|(id, _)| id.clone())
                .collect();
            let mut closed: Vec<Document> = ids
                .iter()
                .filter_map(|id| table.remove(id))
                .map(|entry| entry.lock().document.clone())
                .collect();
            closed.sort_by(|a, b| a.id.cmp(&b.id));
            closed
        };
        for document in &closed {
            self.emit_closed(
                document,
                DocumentClosedReason::WorkspaceClosed,
                actor,
                correlation,
            );
        }
    }
}

impl ExternalModificationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unchanged => "UNCHANGED",
            Self::ModifiedExternally => "MODIFIED_EXTERNALLY",
            Self::Conflicted => "CONFLICTED",
            Self::DeletedExternally => "DELETED_EXTERNALLY",
        }
    }
}

fn not_open(id: &DocumentId) -> CoreError {
    CoreError::not_found(format!("document {id} is not open"))
}

/// Whether an update with `base_version` may be applied to `document`.
fn check_editable(document: &Document, base_version: u64) -> Result<(), CoreError> {
    if document.binary {
        return Err(CoreError::invalid_input(
            "documentId",
            format!("{} is binary and cannot be edited", document.path),
        ));
    }
    if document.read_only {
        let reason = if document.large_file_mode {
            "large file; reopen with largeFileEditing to edit it"
        } else {
            "the file is not writable"
        };
        return Err(CoreError::new(
            CoreErrorCode::PermissionDenied,
            format!("{} is read-only: {reason}", document.path),
        )
        .with_details(serde_json::json!({
            "documentId": document.id,
            "largeFileMode": document.large_file_mode,
        })));
    }
    if base_version != document.current_version {
        return Err(CoreError::conflict(format!(
            "update is based on version {base_version} but the document is at version {}",
            document.current_version
        ))
        .with_details(serde_json::json!({
            "documentId": document.id,
            "baseVersion": base_version,
            "currentVersion": document.current_version,
        })));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use gritty_workspace::is_temp_file_name;
    use tokio::sync::mpsc;
    use tokio::time::timeout;

    use super::*;
    use crate::event::EventEnvelope;
    use crate::event::testing::RecordingSink;
    use crate::operation::OperationRegistry;

    /// Small bounds so the large-file paths are exercised with tiny fixtures.
    const LIMITS: DocumentLimits = DocumentLimits {
        large_file_threshold: 4096,
        max_bytes: 64 * 1024,
    };

    struct Harness {
        workspaces: Arc<WorkspaceManager>,
        documents: Arc<DocumentManager>,
        sink: Arc<RecordingSink>,
        rx: mpsc::UnboundedReceiver<EventEnvelope>,
    }

    fn harness() -> Harness {
        let (sink, rx) = RecordingSink::with_channel();
        let events = EventBus::new(sink.clone());
        let registry = OperationRegistry::new(Handle::current(), events.clone());
        let workspaces = WorkspaceManager::new(Handle::current(), events.clone(), registry);
        let documents = DocumentManager::new(Handle::current(), events, workspaces.clone(), LIMITS);
        Harness {
            workspaces,
            documents,
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

        fn open_ws(&self, dir: &TempDir) -> Workspace {
            self.workspaces
                .open(
                    &dir.0,
                    UnsavedPolicy::Refuse,
                    Actor::User,
                    CorrelationId::new(),
                )
                .unwrap()
        }

        async fn open_doc(&self, ws: &Workspace, path: &str) -> Result<DocumentContent, CoreError> {
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
        }

        async fn update(
            &self,
            id: &DocumentId,
            base: u64,
            text: &str,
        ) -> Result<Document, CoreError> {
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

        async fn status(&self, id: &DocumentId) -> Result<Document, CoreError> {
            let mut documents = self
                .documents
                .status(
                    DocumentStatusRequest {
                        document_id: Some(id.clone()),
                    },
                    Actor::User,
                    CorrelationId::new(),
                )
                .await?;
            Ok(documents.remove(0))
        }

        async fn reload(&self, id: &DocumentId) -> Result<DocumentContent, CoreError> {
            self.documents
                .reload(
                    ReloadDocumentRequest {
                        document_id: id.clone(),
                    },
                    Actor::User,
                    CorrelationId::new(),
                )
                .await
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
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "gritty-core-doc-{label}-{}-{n}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(path.join("src")).unwrap();
            fs::write(path.join("README.md"), "# hi\n").unwrap();
            fs::write(path.join("src/main.rs"), "fn main() {}\n").unwrap();
            fs::write(
                path.join("bom.txt"),
                [0xEF, 0xBB, 0xBF, b'b', b'o', b'm', b'\n'],
            )
            .unwrap();
            fs::write(path.join("image.bin"), b"\x89PNG\0\0\0").unwrap();
            fs::write(path.join("big.txt"), "x".repeat(5000)).unwrap();
            Self(fs::canonicalize(&path).unwrap())
        }

        fn read(&self, relative: &str) -> Vec<u8> {
            fs::read(self.0.join(relative)).unwrap()
        }

        fn write(&self, relative: &str, content: &[u8]) {
            fs::write(self.0.join(relative), content).unwrap();
        }

        fn temp_files(&self) -> Vec<String> {
            fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| is_temp_file_name(name))
                .collect()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn open_returns_the_text_and_announces_the_document() {
        let mut h = harness();
        let dir = TempDir::new("open");
        let ws = h.open_ws(&dir);
        let content = h.open_doc(&ws, "src/main.rs").await.unwrap();
        let doc = &content.document;
        assert!(doc.id.is_well_formed());
        assert_eq!(doc.workspace_id, ws.id);
        assert_eq!(doc.path.as_str(), "src/main.rs");
        assert_eq!(doc.language_id, "rust");
        assert_eq!(doc.encoding, DocumentEncoding::Utf8);
        assert_eq!(doc.saved_revision, 1);
        assert_eq!(doc.current_version, 1);
        assert_eq!(doc.saved_hash, ContentHash::of(b"fn main() {}\n"));
        assert!(!doc.dirty && !doc.read_only && !doc.binary && !doc.large_file_mode);
        assert_eq!(
            doc.external_modification_state,
            ExternalModificationState::Unchanged
        );
        assert_eq!(doc.size, 13);
        assert_eq!(content.text.as_deref(), Some("fn main() {}\n"));
        let opened = h.next_named("DocumentOpened").await;
        assert_eq!(opened.meta.workspace_id, Some(ws.id.clone()));
        let CoreEvent::DocumentOpened(payload) = opened.event else {
            panic!("expected DocumentOpened");
        };
        assert_eq!(&payload.document, doc);
        assert_eq!(h.documents.list(), vec![doc.clone()]);

        // Opening the same path again is the same document with its buffer.
        h.update(&doc.id, 1, "fn main() { edited }\n")
            .await
            .unwrap();
        let again = h.open_doc(&ws, "src/main.rs").await.unwrap();
        assert_eq!(again.document.id, doc.id);
        assert_eq!(again.document.current_version, 2);
        assert!(again.document.dirty);
        assert_eq!(again.text.as_deref(), Some("fn main() { edited }\n"));
        assert_eq!(h.documents.list().len(), 1);
        assert_eq!(
            h.sink.names(),
            vec!["WorkspaceOpened", "DocumentOpened", "DocumentChanged"],
            "reopening emits nothing"
        );
    }

    #[tokio::test]
    async fn update_versions_the_buffer_and_derives_dirty_from_hashes() {
        let mut h = harness();
        let dir = TempDir::new("update");
        let ws = h.open_ws(&dir);
        let doc = h.open_doc(&ws, "README.md").await.unwrap().document;

        let stale = h.update(&doc.id, 7, "x").await.unwrap_err();
        assert_eq!(stale.code, CoreErrorCode::Conflict);
        assert_eq!(stale.details.as_ref().unwrap()["currentVersion"], 1);
        assert_eq!(stale.details.as_ref().unwrap()["baseVersion"], 7);

        let v2 = h.update(&doc.id, 1, "# hi\nmore\n").await.unwrap();
        assert_eq!(v2.current_version, 2);
        assert!(v2.dirty);
        assert_eq!(v2.saved_revision, 1);
        assert_eq!(v2.saved_hash, doc.saved_hash, "saving state untouched");
        let changed = h.next_named("DocumentChanged").await;
        let CoreEvent::DocumentChanged(payload) = changed.event else {
            panic!("expected DocumentChanged");
        };
        assert_eq!(payload.cause, DocumentChangeCause::Edit);
        assert!(payload.document.dirty);

        // Typing the original text back makes the document clean again.
        let v3 = h.update(&doc.id, 2, "# hi\n").await.unwrap();
        assert_eq!(v3.current_version, 3);
        assert!(!v3.dirty);

        // Nothing touched the disk.
        assert_eq!(dir.read("README.md"), b"# hi\n");
        let unknown = h.update(&DocumentId::new(), 1, "x").await.unwrap_err();
        assert_eq!(unknown.code, CoreErrorCode::NotFound);
        let huge = h
            .update(&doc.id, 3, &"y".repeat(LIMITS.max_bytes as usize + 1))
            .await
            .unwrap_err();
        assert_eq!(huge.code, CoreErrorCode::InvalidInput);
        assert_eq!(huge.details.unwrap()["field"], "text");
    }

    #[tokio::test]
    async fn save_writes_atomically_and_preserves_the_bom() {
        let mut h = harness();
        let dir = TempDir::new("save");
        let ws = h.open_ws(&dir);
        let doc = h.open_doc(&ws, "bom.txt").await.unwrap();
        assert_eq!(doc.document.encoding, DocumentEncoding::Utf8Bom);
        assert_eq!(doc.text.as_deref(), Some("bom\n"));
        let id = doc.document.id.clone();

        // Saving a clean document writes nothing and emits nothing.
        let same = h.save(&id, false).await.unwrap();
        assert_eq!(same, doc.document);

        h.update(&id, 1, "bom\nsecond\r\n").await.unwrap();
        let saved = h.save(&id, false).await.unwrap();
        assert!(!saved.dirty);
        assert_eq!(saved.saved_revision, 2);
        assert_eq!(saved.current_version, 2);
        assert_eq!(saved.size, 3 + "bom\nsecond\r\n".len() as u64);
        assert_eq!(
            saved.external_modification_state,
            ExternalModificationState::Unchanged
        );
        let on_disk = dir.read("bom.txt");
        assert_eq!(&on_disk[..3], [0xEF, 0xBB, 0xBF]);
        assert_eq!(&on_disk[3..], b"bom\nsecond\r\n");
        assert_eq!(saved.saved_hash, ContentHash::of(&on_disk));
        assert!(dir.temp_files().is_empty());
        let event = h.next_named("DocumentSaved").await;
        let CoreEvent::DocumentSaved(payload) = event.event else {
            panic!("expected DocumentSaved");
        };
        assert_eq!(payload.document, saved);
        assert_eq!(h.status(&id).await.unwrap(), saved);
    }

    #[tokio::test]
    async fn external_modification_matrix_and_forced_save() {
        let mut h = harness();
        let dir = TempDir::new("external");
        let ws = h.open_ws(&dir);
        let id = h.open_doc(&ws, "README.md").await.unwrap().document.id;

        let unchanged = h.status(&id).await.unwrap();
        assert_eq!(
            unchanged.external_modification_state,
            ExternalModificationState::Unchanged
        );

        // Clean buffer + external write → MODIFIED_EXTERNALLY (same size, so
        // only the hash can tell).
        dir.write("README.md", b"# HI\n");
        let modified = h.status(&id).await.unwrap();
        assert_eq!(
            modified.external_modification_state,
            ExternalModificationState::ModifiedExternally
        );
        assert!(!modified.dirty);
        let detected = h.next_named("DocumentExternalChangeDetected").await;
        assert_eq!(detected.meta.actor, Actor::External);
        let CoreEvent::DocumentExternalChangeDetected(payload) = detected.event else {
            panic!("expected DocumentExternalChangeDetected");
        };
        assert_eq!(payload.previous_state, ExternalModificationState::Unchanged);
        // A second identical check is silent.
        h.status(&id).await.unwrap();
        assert_eq!(
            h.sink
                .names()
                .iter()
                .filter(|n| **n == "DocumentExternalChangeDetected")
                .count(),
            1
        );

        // Editing the buffer now makes it a conflict; saving needs force.
        let dirty = h.update(&id, 1, "# hi\nlocal edit\n").await.unwrap();
        assert_eq!(
            dirty.external_modification_state,
            ExternalModificationState::Conflicted
        );
        let refused = h.save(&id, false).await.unwrap_err();
        assert_eq!(refused.code, CoreErrorCode::Conflict);
        assert_eq!(
            refused.details.unwrap()["externalModificationState"],
            "CONFLICTED"
        );
        assert_eq!(
            dir.read("README.md"),
            b"# HI\n",
            "the external version survived"
        );
        let still = h.status(&id).await.unwrap();
        assert!(still.dirty);
        assert_eq!(
            still.external_modification_state,
            ExternalModificationState::Conflicted
        );

        // Deleted while dirty → DELETED_EXTERNALLY; save recreates only with force.
        fs::remove_file(dir.0.join("README.md")).unwrap();
        let deleted = h.status(&id).await.unwrap();
        assert_eq!(
            deleted.external_modification_state,
            ExternalModificationState::DeletedExternally
        );
        let refused = h.save(&id, false).await.unwrap_err();
        assert_eq!(refused.code, CoreErrorCode::Conflict);
        assert!(!dir.0.join("README.md").exists());
        let forced = h.save(&id, true).await.unwrap();
        assert_eq!(
            forced.external_modification_state,
            ExternalModificationState::Unchanged
        );
        assert!(!forced.dirty);
        assert_eq!(forced.saved_revision, 2);
        assert_eq!(dir.read("README.md"), b"# hi\nlocal edit\n");
        assert!(dir.temp_files().is_empty());

        // Forced save over an externally modified file overwrites it.
        dir.write("README.md", b"someone else\n");
        h.update(&id, 2, "mine\n").await.unwrap();
        let forced = h.save(&id, true).await.unwrap();
        assert_eq!(dir.read("README.md"), b"mine\n");
        assert_eq!(forced.saved_revision, 3);
    }

    #[tokio::test]
    async fn reload_replaces_the_buffer_only_when_asked_and_rejects_stale_edits() {
        let mut h = harness();
        let dir = TempDir::new("reload");
        let ws = h.open_ws(&dir);
        let id = h.open_doc(&ws, "src/main.rs").await.unwrap().document.id;
        h.update(&id, 1, "fn main() { local }\n").await.unwrap();
        dir.write("src/main.rs", b"fn main() { external }\n");
        let conflicted = h.status(&id).await.unwrap();
        assert_eq!(
            conflicted.external_modification_state,
            ExternalModificationState::Conflicted
        );

        let reloaded = h.reload(&id).await.unwrap();
        assert_eq!(reloaded.text.as_deref(), Some("fn main() { external }\n"));
        assert_eq!(reloaded.document.current_version, 3);
        assert_eq!(reloaded.document.saved_revision, 2);
        assert!(!reloaded.document.dirty);
        assert_eq!(
            reloaded.document.external_modification_state,
            ExternalModificationState::Unchanged
        );
        let changed = h.next_named("DocumentChanged").await;
        let CoreEvent::DocumentChanged(payload) = changed.event else {
            panic!("expected DocumentChanged");
        };
        assert_eq!(payload.cause, DocumentChangeCause::Edit);
        let changed = h.next_named("DocumentChanged").await;
        let CoreEvent::DocumentChanged(payload) = changed.event else {
            panic!("expected DocumentChanged");
        };
        assert_eq!(payload.cause, DocumentChangeCause::Reload);

        // An edit that was in flight against the pre-reload buffer is refused.
        let stale = h.update(&id, 2, "fn main() { stale }\n").await.unwrap_err();
        assert_eq!(stale.code, CoreErrorCode::Conflict);
        let fresh = h.update(&id, 3, "fn main() { fresh }\n").await.unwrap();
        assert_eq!(fresh.current_version, 4);

        // Reloading a deleted file fails and marks it deleted; the buffer stays.
        fs::remove_file(dir.0.join("src/main.rs")).unwrap();
        let error = h.reload(&id).await.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::NotFound);
        let after = h.open_doc(&ws, "src/main.rs").await.unwrap();
        assert_eq!(after.document.id, id);
        assert_eq!(after.text.as_deref(), Some("fn main() { fresh }\n"));
        assert_eq!(
            after.document.external_modification_state,
            ExternalModificationState::DeletedExternally
        );
    }

    #[tokio::test]
    async fn binary_and_large_files_are_guarded() {
        let mut h = harness();
        let dir = TempDir::new("guards");
        let ws = h.open_ws(&dir);

        let binary = h.open_doc(&ws, "image.bin").await.unwrap();
        assert!(binary.document.binary);
        assert_eq!(binary.document.encoding, DocumentEncoding::Binary);
        assert_eq!(binary.text, None);
        assert!(!binary.document.dirty);
        let error = h.update(&binary.document.id, 1, "x").await.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        let error = h.save(&binary.document.id, true).await.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert_eq!(dir.read("image.bin"), b"\x89PNG\0\0\0", "never rewritten");
        assert_eq!(
            h.status(&binary.document.id)
                .await
                .unwrap()
                .external_modification_state,
            ExternalModificationState::Unchanged
        );

        let large = h.open_doc(&ws, "big.txt").await.unwrap();
        assert!(large.document.large_file_mode);
        assert!(large.document.read_only);
        assert_eq!(large.text.as_deref().map(str::len), Some(5000));
        let error = h.update(&large.document.id, 1, "x").await.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::PermissionDenied);
        assert_eq!(error.details.unwrap()["largeFileMode"], true);

        let enabled = h
            .documents
            .open(
                OpenDocumentRequest {
                    workspace_id: ws.id.clone(),
                    path: WorkspacePath::parse("big.txt").unwrap(),
                    large_file_editing: Some(true),
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
            .unwrap();
        assert_eq!(enabled.document.id, large.document.id);
        assert!(enabled.document.large_file_mode);
        assert!(!enabled.document.read_only);
        let changed = h.next_named("DocumentChanged").await;
        let CoreEvent::DocumentChanged(payload) = changed.event else {
            panic!("expected DocumentChanged");
        };
        assert_eq!(payload.cause, DocumentChangeCause::ReadOnlyChanged);
        let edited = h
            .update(&large.document.id, 1, "small now\n")
            .await
            .unwrap();
        assert!(edited.dirty);
        let saved = h.save(&large.document.id, false).await.unwrap();
        assert!(
            saved.large_file_mode && !saved.read_only,
            "the mode is decided when the buffer is synchronized from disk, not per save"
        );
        assert_eq!(dir.read("big.txt"), b"small now\n");
        let reloaded = h.reload(&large.document.id).await.unwrap();
        assert!(
            !reloaded.document.large_file_mode,
            "a reload re-evaluates the size"
        );
        assert!(!reloaded.document.read_only);
    }

    #[tokio::test]
    async fn dirty_documents_are_never_dropped_silently() {
        let mut h = harness();
        let dir = TempDir::new("dirty");
        let other = TempDir::new("dirty-other");
        let ws = h.open_ws(&dir);
        let id = h.open_doc(&ws, "README.md").await.unwrap().document.id;
        let clean_id = h.open_doc(&ws, "src/main.rs").await.unwrap().document.id;
        h.update(&id, 1, "unsaved\n").await.unwrap();

        // documents.close refuses without force.
        let refused = h.close(&id, false).unwrap_err();
        assert_eq!(refused.code, CoreErrorCode::Conflict);
        assert_eq!(h.documents.list().len(), 2);

        // workspace.close and a replacing workspace.open refuse as well.
        let refused = h
            .workspaces
            .close(
                &ws.id,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap_err();
        assert_eq!(refused.code, CoreErrorCode::Conflict);
        let details = refused.details.unwrap();
        assert_eq!(details["dirtyDocuments"].as_array().unwrap().len(), 1);
        assert_eq!(details["dirtyDocuments"][0]["path"], "README.md");
        assert_eq!(details["reason"], "USER");
        let refused = h
            .workspaces
            .open(
                &other.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap_err();
        assert_eq!(refused.code, CoreErrorCode::Conflict);
        assert_eq!(refused.details.unwrap()["reason"], "REPLACED");
        assert_eq!(
            h.workspaces.current(&CorrelationId::new()),
            Some(ws.clone())
        );
        assert_eq!(h.documents.list().len(), 2);

        // A clean document closes without ceremony.
        let closed = h.close(&clean_id, false).unwrap();
        assert!(!closed.dirty);
        let event = h.next_named("DocumentClosed").await;
        let CoreEvent::DocumentClosed(payload) = event.event else {
            panic!("expected DocumentClosed");
        };
        assert_eq!(payload.reason, DocumentClosedReason::User);
        assert!(!payload.dirty);

        // Explicit discard closes the workspace and its documents, flagging the loss.
        let closed = h
            .workspaces
            .close(
                &ws.id,
                UnsavedPolicy::Discard,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        assert_eq!(closed, ws);
        h.next_named("WorkspaceClosed").await;
        let event = h.next_named("DocumentClosed").await;
        let CoreEvent::DocumentClosed(payload) = event.event else {
            panic!("expected DocumentClosed");
        };
        assert_eq!(payload.document_id, id);
        assert_eq!(payload.reason, DocumentClosedReason::WorkspaceClosed);
        assert!(payload.dirty);
        assert!(h.documents.list().is_empty());
        let error = h.update(&id, 2, "x").await.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::NotFound);
        assert_eq!(
            dir.read("README.md"),
            b"# hi\n",
            "the disk was never touched"
        );
    }

    #[tokio::test]
    async fn root_missing_and_replacement_close_documents() {
        let mut h = harness();
        let dir = TempDir::new("replace");
        let second = TempDir::new("replace-second");
        let ws = h.open_ws(&dir);
        let id = h.open_doc(&ws, "README.md").await.unwrap().document.id;
        let ws2 = h
            .workspaces
            .open(
                &second.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        let closed = h.next_named("DocumentClosed").await;
        let CoreEvent::DocumentClosed(payload) = closed.event else {
            panic!("expected DocumentClosed");
        };
        assert_eq!(payload.document_id, id);
        assert_eq!(payload.reason, DocumentClosedReason::WorkspaceClosed);
        assert!(!payload.dirty);
        assert!(h.documents.list().is_empty());

        let id2 = h.open_doc(&ws2, "README.md").await.unwrap().document.id;
        h.update(&id2, 1, "dirty\n").await.unwrap();
        fs::remove_dir_all(&second.0).unwrap();
        let error = h.status(&id2).await.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::NotFound);
        assert_eq!(error.details.unwrap()["reason"], "ROOT_MISSING");
        let closed = h.next_named("DocumentClosed").await;
        assert_eq!(closed.meta.actor, Actor::System);
        let CoreEvent::DocumentClosed(payload) = closed.event else {
            panic!("expected DocumentClosed");
        };
        assert_eq!(payload.document_id, id2);
        assert!(payload.dirty, "the loss is reported, not hidden");
        assert!(h.documents.list().is_empty());

        // Checking every document when nothing is open is fine.
        let all = h
            .documents
            .status(
                DocumentStatusRequest::default(),
                Actor::User,
                CorrelationId::new(),
            )
            .await
            .unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn write_failure_preserves_buffer_dirty_state_and_disk() {
        let h = harness();
        let dir = TempDir::new("write-failure");
        let ws = h.open_ws(&dir);
        let id = h.open_doc(&ws, "README.md").await.unwrap().document.id;
        h.update(&id, 1, "new content\n").await.unwrap();
        *h.documents.write_fault.lock().unwrap() = AtomicWriteFault::FailBeforeRename;
        let error = h.save(&id, false).await.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::Io);
        assert_eq!(dir.read("README.md"), b"# hi\n");
        assert!(dir.temp_files().is_empty(), "no temporary file left behind");
        let doc = h.status(&id).await.unwrap();
        assert!(doc.dirty);
        assert_eq!(doc.saved_revision, 1);
        assert_eq!(
            doc.external_modification_state,
            ExternalModificationState::Unchanged
        );
        assert_eq!(
            h.open_doc(&ws, "README.md").await.unwrap().text.as_deref(),
            Some("new content\n")
        );
        *h.documents.write_fault.lock().unwrap() = AtomicWriteFault::None;
        let saved = h.save(&id, false).await.unwrap();
        assert!(!saved.dirty);
        assert_eq!(dir.read("README.md"), b"new content\n");
        assert!(!h.sink.names().contains(&"DocumentSaved") || saved.saved_revision == 2);
    }

    #[tokio::test]
    async fn open_failures_create_no_document() {
        let h = harness();
        let dir = TempDir::new("open-failures");
        let ws = h.open_ws(&dir);
        let error = h.open_doc(&ws, "src").await.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        let error = h.open_doc(&ws, "").await.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        let error = h.open_doc(&ws, "nope.txt").await.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::NotFound);
        dir.write("huge.txt", &vec![b'z'; LIMITS.max_bytes as usize + 1]);
        let error = h.open_doc(&ws, "huge.txt").await.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert_eq!(error.details.unwrap()["reason"], "too large");
        let error = h
            .documents
            .open(
                OpenDocumentRequest {
                    workspace_id: WorkspaceId::new(),
                    path: WorkspacePath::parse("README.md").unwrap(),
                    large_file_editing: None,
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::NotFound);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            dir.write("denied.txt", b"secret");
            fs::set_permissions(dir.0.join("denied.txt"), fs::Permissions::from_mode(0o000))
                .unwrap();
            if fs::read(dir.0.join("denied.txt")).is_err() {
                let error = h.open_doc(&ws, "denied.txt").await.unwrap_err();
                assert_eq!(error.code, CoreErrorCode::PermissionDenied);
            }
        }
        assert!(h.documents.list().is_empty());
        assert_eq!(h.sink.names(), vec!["WorkspaceOpened"]);
    }

    #[test]
    fn wire_types_serialize_as_documented() {
        let document = Document {
            id: DocumentId::from_raw("doc-1"),
            workspace_id: WorkspaceId::from_raw("ws-1"),
            path: WorkspacePath::parse("src/main.rs").unwrap(),
            encoding: DocumentEncoding::Utf8Bom,
            language_id: "rust".into(),
            saved_revision: 1,
            saved_hash: ContentHash::of(b""),
            current_version: 2,
            dirty: true,
            read_only: false,
            binary: false,
            large_file_mode: false,
            external_modification_state: ExternalModificationState::Conflicted,
            size: 0,
        };
        let json = serde_json::to_value(DocumentContent {
            document: document.clone(),
            text: None,
        })
        .unwrap();
        assert_eq!(json["document"]["encoding"], "utf-8-bom");
        assert_eq!(json["document"]["externalModificationState"], "CONFLICTED");
        assert_eq!(json["document"]["savedHash"], ContentHash::of(b"").as_str());
        assert_eq!(json["document"]["path"], "src/main.rs");
        assert!(json.get("text").is_none());
        let back: DocumentContent = serde_json::from_value(json).unwrap();
        assert_eq!(back.document, document);

        let request: UpdateDocumentRequest = serde_json::from_value(serde_json::json!({
            "documentId": "doc-1", "baseVersion": 2, "text": "x"
        }))
        .unwrap();
        assert_eq!(request.base_version, 2);
        let request: SaveDocumentRequest =
            serde_json::from_value(serde_json::json!({ "documentId": "doc-1" })).unwrap();
        assert_eq!(request.force, None);
        let request: DocumentStatusRequest = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(request.document_id, None);
        let error = serde_json::from_value::<OpenDocumentRequest>(serde_json::json!({
            "workspaceId": "ws-1", "path": "../x"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("'..'"), "{error}");
        assert_eq!(
            serde_json::to_value(DocumentClosedReason::WorkspaceClosed).unwrap(),
            "WORKSPACE_CLOSED"
        );
        assert_eq!(
            serde_json::to_value(DocumentChangeCause::ReadOnlyChanged).unwrap(),
            "READ_ONLY_CHANGED"
        );
        assert_eq!(
            DiskState::Modified.wire(true),
            ExternalModificationState::Conflicted
        );
        assert_eq!(
            DiskState::Modified.wire(false),
            ExternalModificationState::ModifiedExternally
        );
        assert_eq!(
            DiskState::Deleted.wire(true),
            ExternalModificationState::DeletedExternally
        );
        assert_eq!(
            DiskState::Unchanged.wire(true),
            ExternalModificationState::Unchanged
        );
    }
}
