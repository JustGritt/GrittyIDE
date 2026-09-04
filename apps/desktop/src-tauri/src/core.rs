//! Composition of the core inside the Tauri process: the event sink that
//! forwards envelopes to the WebView, the session tap next to it, the event
//! bus, the operation registry, the workspace and document managers, the
//! shell service and the session service, all running on Tauri's tokio
//! runtime. Managed as Tauri state.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gritty_core::CORE_EVENT_CHANNEL;
use gritty_core::CoreError;
use gritty_core::document::{DocumentLimits, DocumentManager};
use gritty_core::event::{EventBus, EventEnvelope, EventSink, FanOutSink};
use gritty_core::operation::OperationRegistry;
use gritty_core::session::{SessionService, SessionStore, SessionTap, TAP_CAPACITY, WriterConfig};
use gritty_core::shell::{ExternalOpener, ShellService};
use gritty_core::workspace::WorkspaceManager;
use gritty_persistence::DB_FILE_NAME;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::shell::SystemOpener;

/// How long shutdown waits for the session writer and for running operations.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// Environment variable overriding the application data directory (dev,
/// manual crash tests): the session database lives there.
pub const DATA_DIR_ENV: &str = "GRITTY_DATA_DIR";

/// What the composition root injects; tests replace both.
pub struct CoreOptions {
    pub opener: Arc<dyn ExternalOpener>,
    /// Directory of the session database; `None` runs without persistence.
    pub data_dir: Option<PathBuf>,
}

pub struct Core {
    pub registry: Arc<OperationRegistry>,
    pub workspaces: Arc<WorkspaceManager>,
    pub documents: Arc<DocumentManager>,
    pub shell: Arc<ShellService>,
    pub session: Arc<SessionService>,
}

/// Delivers every envelope on the single core event channel. Failures are
/// reported back to the bus, which logs them with the correlation id.
struct TauriEventSink<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> EventSink for TauriEventSink<R> {
    fn emit(&self, envelope: &EventEnvelope) -> Result<(), CoreError> {
        self.app
            .emit(CORE_EVENT_CHANNEL, envelope)
            .map_err(|error| CoreError::internal(format!("event delivery failed: {error}")))
    }
}

impl Core {
    /// The production composition: system browser, platform app-data directory.
    pub fn new<R: Runtime>(app: AppHandle<R>) -> Self {
        let data_dir = resolve_data_dir(&app);
        Self::with_options(
            app,
            CoreOptions {
                opener: Arc::new(SystemOpener),
                data_dir,
            },
        )
    }

    /// Same wiring with injected dependencies (tests never open a browser
    /// and never touch the real database).
    pub fn with_options<R: Runtime>(app: AppHandle<R>, options: CoreOptions) -> Self {
        let (tap, receiver) = SessionTap::new(TAP_CAPACITY);
        let events = EventBus::new(FanOutSink::new(vec![Arc::new(TauriEventSink { app }), tap]));
        // Tauri already runs a tokio runtime; the core builds on it (no second runtime).
        let handle = tauri::async_runtime::handle().inner().clone();
        let registry = OperationRegistry::new(handle.clone(), events.clone());
        let workspaces = WorkspaceManager::new(handle.clone(), events.clone(), registry.clone());
        // Installs itself as the workspace close guard (unsaved buffers).
        let documents = DocumentManager::new(
            handle.clone(),
            events.clone(),
            workspaces.clone(),
            DocumentLimits::default(),
        );
        let shell = ShellService::new(handle.clone(), options.opener);
        // Synchronous: a small database opens in milliseconds and the window
        // geometry must be known before the window shows.
        let store = match options.data_dir {
            Some(dir) => SessionStore::open(&dir.join(DB_FILE_NAME)),
            None => SessionStore::unavailable("no application data directory is available"),
        };
        let session = SessionService::start(
            handle,
            events,
            documents.clone(),
            workspaces.clone(),
            store,
            receiver,
            WriterConfig::default(),
        );
        Self {
            registry,
            workspaces,
            documents,
            shell,
            session,
        }
    }

    /// Persist what is pending and cancel every running operation; called
    /// from the `RunEvent::Exit` handler.
    pub fn shutdown(&self) {
        let running = self
            .registry
            .list()
            .into_iter()
            .filter(|op| !op.status.is_terminal())
            .count();
        tracing::info!(
            running,
            "shutting down: flushing the session and cancelling running operations"
        );
        tauri::async_runtime::block_on(async {
            if !self.session.flush(SHUTDOWN_GRACE).await {
                tracing::warn!("session writer did not flush before exit");
            }
            self.registry.shutdown(SHUTDOWN_GRACE).await;
        });
    }
}

/// `GRITTY_DATA_DIR`, else the platform application data directory for the
/// bundle identifier (macOS: `~/Library/Application Support/io.github.justgritt.gritty`).
pub fn resolve_data_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV).filter(|value| !value.is_empty()) {
        let dir = PathBuf::from(dir);
        tracing::info!(dir = %dir.display(), "data directory from {DATA_DIR_ENV}");
        return Some(dir);
    }
    match app.path().app_data_dir() {
        Ok(dir) => Some(dir),
        Err(error) => {
            tracing::error!(%error, "no application data directory; running without persistence");
            None
        }
    }
}
