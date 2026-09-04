//! Crash simulation (T060, addendum §11.1): a process writes a recovery
//! snapshot, its state is dropped without any shutdown (no close, no flush,
//! no checkpoint), and a new process on the same database is offered the
//! unsaved text and recovers it in each of the three reconciliation states.
//! The real `kill -9` of the running application is the manual half of this
//! test; this half proves the persistence path with public core APIs only.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gritty_core::CoreError;
use gritty_core::document::{
    DocumentContent, DocumentLimits, DocumentManager, ExternalModificationState,
    OpenDocumentRequest, UpdateDocumentRequest,
};
use gritty_core::event::{EventBus, EventEnvelope, EventSink, FanOutSink};
use gritty_core::operation::OperationRegistry;
use gritty_core::protocol::actor::Actor;
use gritty_core::protocol::ids::CorrelationId;
use gritty_core::session::{
    CandidatesRequest, RecoverSnapshotRequest, RecoveryState, SessionService, SessionStore,
    SessionTap, TAP_CAPACITY, WriterConfig,
};
use gritty_core::workspace::{UnsavedPolicy, Workspace, WorkspaceManager};
use gritty_persistence::store::snapshots;
use gritty_persistence::testing::TempDir;
use gritty_persistence::{DB_FILE_NAME, Database};
use gritty_workspace::WorkspacePath;
use tokio::runtime::Handle;

const UNSAVED: &str = "fn main() {\n    // unsaved work\n}\n";

/// The WebView of these tests: nobody listens.
struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _envelope: &EventEnvelope) -> Result<(), CoreError> {
        Ok(())
    }
}

/// Everything a Gritty process holds; dropped without ceremony to simulate death.
struct Process {
    workspaces: Arc<WorkspaceManager>,
    documents: Arc<DocumentManager>,
    session: Arc<SessionService>,
}

fn boot(db: &Path) -> Process {
    let (tap, receiver) = SessionTap::new(TAP_CAPACITY);
    let events = EventBus::new(FanOutSink::new(vec![Arc::new(NullSink), tap]));
    let registry = OperationRegistry::new(Handle::current(), events.clone());
    let workspaces = WorkspaceManager::new(Handle::current(), events.clone(), registry);
    let documents = DocumentManager::new(
        Handle::current(),
        events.clone(),
        workspaces.clone(),
        DocumentLimits::default(),
    );
    let session = SessionService::start(
        Handle::current(),
        events,
        documents.clone(),
        workspaces.clone(),
        SessionStore::open(db),
        receiver,
        WriterConfig {
            debounce: Duration::from_millis(50),
            max_wait: Duration::from_millis(100),
            ..WriterConfig::default()
        },
    );
    Process {
        workspaces,
        documents,
        session,
    }
}

impl Process {
    fn open_workspace(&self, dir: &Path) -> Workspace {
        self.workspaces
            .open(
                dir,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap()
    }

    async fn open(&self, ws: &Workspace, path: &str) -> DocumentContent {
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

    async fn recover(&self, ws: &Workspace, path: &str) -> DocumentContent {
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
            .unwrap()
    }

    async fn candidates(&self, ws: &Workspace) -> Vec<(String, RecoveryState)> {
        self.session
            .candidates(
                CandidatesRequest {
                    workspace_id: ws.id.clone(),
                },
                CorrelationId::new(),
            )
            .await
            .unwrap()
            .into_iter()
            .map(|c| (c.path.to_string(), c.state))
            .collect()
    }
}

fn snapshot_text(db: &Path, root: &Path, path: &str) -> Option<String> {
    Database::open(db)
        .unwrap()
        .database
        .read(|c| snapshots::get(c, &root.display().to_string(), path))
        .unwrap()
        .map(|row| row.text)
}

/// Process 1: open a workspace, edit a file, never save, wait until the
/// writer has the snapshot on disk, then die. Returns the data dir, the
/// workspace dir and the canonical root.
async fn crash_while_dirty(label: &str) -> (TempDir, TempDir, PathBuf) {
    let data = TempDir::new(&format!("crash-{label}"));
    let dir = TempDir::new(&format!("crash-ws-{label}"));
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let db = data.join(DB_FILE_NAME);

    let process = boot(&db);
    let ws = process.open_workspace(dir.path());
    let doc = process.open(&ws, "src/main.rs").await;
    process
        .documents
        .update(
            UpdateDocumentRequest {
                document_id: doc.document.id.clone(),
                base_version: doc.document.current_version,
                text: UNSAVED.to_owned(),
            },
            Actor::User,
            CorrelationId::new(),
        )
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while snapshot_text(&db, &root, "src/main.rs").is_none() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "snapshot written within the debounce"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Death: no close, no flush, no checkpoint. The Arcs leak on purpose.
    std::mem::forget(process);
    assert_eq!(
        fs::read_to_string(dir.join("src/main.rs")).unwrap(),
        "fn main() {}\n",
        "the disk was never touched"
    );
    (data, dir, root)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsaved_text_survives_the_process_and_is_recovered_when_the_disk_is_unchanged() {
    let (data, dir, root) = crash_while_dirty("same").await;
    let db = data.join(DB_FILE_NAME);

    let process = boot(&db);
    let restored = process.session.restore(CorrelationId::new()).await;
    let ws = restored.workspace.expect("last workspace reopened");
    assert_eq!(ws.root, root.display().to_string());
    assert!(restored.persistence.available);
    assert_eq!(
        process.candidates(&ws).await,
        vec![("src/main.rs".to_owned(), RecoveryState::SameAsDisk)]
    );
    let recovered = process.recover(&ws, "src/main.rs").await;
    assert_eq!(recovered.text.as_deref(), Some(UNSAVED));
    assert!(recovered.document.dirty);
    assert_eq!(
        recovered.document.external_modification_state,
        ExternalModificationState::Unchanged
    );
    assert_eq!(
        fs::read_to_string(dir.join("src/main.rs")).unwrap(),
        "fn main() {}\n"
    );
    assert!(
        process.candidates(&ws).await.is_empty(),
        "a recovered (dirty) document is no longer a candidate"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_changed_after_the_crash_is_recovered_as_conflicted() {
    let (data, dir, _root) = crash_while_dirty("changed").await;
    fs::write(
        dir.join("src/main.rs"),
        "fn main() { changed_by_git_pull() }\n",
    )
    .unwrap();

    let process = boot(&data.join(DB_FILE_NAME));
    let ws = process
        .session
        .restore(CorrelationId::new())
        .await
        .workspace
        .unwrap();
    assert_eq!(
        process.candidates(&ws).await,
        vec![("src/main.rs".to_owned(), RecoveryState::DiskChanged)]
    );
    let recovered = process.recover(&ws, "src/main.rs").await;
    assert_eq!(recovered.text.as_deref(), Some(UNSAVED));
    assert_eq!(
        recovered.document.external_modification_state,
        ExternalModificationState::Conflicted
    );
    assert_eq!(
        fs::read_to_string(dir.join("src/main.rs")).unwrap(),
        "fn main() { changed_by_git_pull() }\n",
        "both versions exist: the buffer and the disk"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_deleted_after_the_crash_is_recovered_as_deleted_externally() {
    let (data, dir, _root) = crash_while_dirty("missing").await;
    fs::remove_file(dir.join("src/main.rs")).unwrap();

    let process = boot(&data.join(DB_FILE_NAME));
    let ws = process
        .session
        .restore(CorrelationId::new())
        .await
        .workspace
        .unwrap();
    assert_eq!(
        process.candidates(&ws).await,
        vec![("src/main.rs".to_owned(), RecoveryState::FileMissing)]
    );
    let recovered = process.recover(&ws, "src/main.rs").await;
    assert_eq!(recovered.text.as_deref(), Some(UNSAVED));
    assert_eq!(
        recovered.document.external_modification_state,
        ExternalModificationState::DeletedExternally
    );
    assert!(
        !dir.join("src/main.rs").exists(),
        "nothing recreated the file"
    );
}
