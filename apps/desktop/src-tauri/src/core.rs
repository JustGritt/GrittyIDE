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
use gritty_core::agent_session::{AgentDiscoveryConfig, AgentSessionManager};
use gritty_core::document::{DocumentLimits, DocumentManager};
use gritty_core::event::{EventBus, EventEnvelope, EventSink, FanOutSink};
use gritty_core::file_drop::FileDropService;
use gritty_core::operation::OperationRegistry;
use gritty_core::ownership::OwnershipManager;
use gritty_core::session::{SessionService, SessionStore, SessionTap, TAP_CAPACITY, WriterConfig};
use gritty_core::shell::{ExternalOpener, ShellService};
use gritty_core::terminal::TerminalManager;
use gritty_core::workspace::WorkspaceManager;
use gritty_core::workstream::WorkstreamManager;
use gritty_persistence::DB_FILE_NAME;
use gritty_terminal::SystemPtyHost;
use gritty_terminal::host::PtyHost;
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
    /// Opens PTY sessions. Tests inject a fake so no test spawns a shell.
    pub pty_host: Arc<dyn PtyHost>,
}

pub struct Core {
    pub registry: Arc<OperationRegistry>,
    pub workspaces: Arc<WorkspaceManager>,
    pub file_drop: Arc<FileDropService>,
    pub documents: Arc<DocumentManager>,
    pub shell: Arc<ShellService>,
    pub session: Arc<SessionService>,
    pub terminals: Arc<TerminalManager>,
    pub workstreams: Arc<WorkstreamManager>,
    pub ownership: Arc<OwnershipManager>,
    pub agents: Arc<AgentSessionManager>,
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
                pty_host: Arc::new(SystemPtyHost::new()),
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
        let terminals = TerminalManager::new(events.clone(), workspaces.clone(), options.pty_host);
        let file_drop = FileDropService::new(events.clone(), workspaces.clone());
        // Synchronous: a small database opens in milliseconds and the window
        // geometry must be known before the window shows.
        let data_dir = options.data_dir;
        let store = match data_dir.as_deref() {
            Some(dir) => SessionStore::open(&dir.join(DB_FILE_NAME)),
            None => SessionStore::unavailable("no application data directory is available"),
        };
        // Workstreams share the session database (T200): one file, one
        // migration chain. Taken before the store is handed to the session
        // service, which consumes it.
        let workstream_db = store.database().cloned();
        let session = SessionService::start(
            handle.clone(),
            events.clone(),
            documents.clone(),
            workspaces.clone(),
            store,
            receiver,
            WriterConfig::default(),
        );
        let workstreams = WorkstreamManager::new(
            handle.clone(),
            events.clone(),
            workspaces.clone(),
            workstream_db.clone(),
        );
        // Ownership shares the workstream database on purpose: a lease belongs
        // to a workstream, and one migration chain keeps them consistent (T325).
        let ownership = OwnershipManager::new(
            handle.clone(),
            events.clone(),
            workspaces.clone(),
            workstream_db,
        );
        // Discovery of *other* agents on this machine (T220). Read-only, and
        // started here rather than lazily so the roster is warm by the time the
        // island first asks for it.
        //
        // The hook reader (T221) is pointed at the same application-data
        // directory the session database lives in, and at this build's own hook
        // binary. Both may be absent — no data directory, or a build whose
        // binary is not beside the executable — and the integration is then
        // simply not offered rather than half-wired.
        let agents = AgentSessionManager::start(
            handle,
            events,
            workspaces.clone(),
            AgentDiscoveryConfig::for_current_home()
                .with_data_dir(data_dir.as_deref())
                .with_hook_binary(resolve_hook_binary()),
        );
        Self {
            registry,
            workspaces,
            file_drop,
            documents,
            shell,
            session,
            terminals,
            workstreams,
            ownership,
            agents,
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
            "shutting down: killing terminals, flushing the session and cancelling operations"
        );
        // Terminals first: a shell left behind outlives the window, and its
        // output would arrive after the session writer has been flushed.
        self.terminals.shutdown();
        // Stops the discovery poll so no scan outlives the window.
        self.agents.shutdown();
        tauri::async_runtime::block_on(async {
            if !self.session.flush(SHUTDOWN_GRACE).await {
                tracing::warn!("session writer did not flush before exit");
            }
            self.registry.shutdown(SHUTDOWN_GRACE).await;
        });
    }
}

/// Environment variable naming this build's `gritty-agent-hook` (dev, tests).
pub const HOOK_BINARY_ENV: &str = "GRITTY_AGENT_HOOK";

/// Where this build's agent hook binary is (T221).
///
/// `GRITTY_AGENT_HOOK` first, then a sibling of the running executable — which
/// is what `cargo run`, `target/debug` and a macOS bundle's `MacOS/` directory
/// all give. `None` when it is not there, and the integration is then reported
/// as unavailable rather than installed as an entry that would point at
/// nothing (ADR-018's uninstall obligation, from the other end).
///
/// This does **not** yet resolve a bundled sidecar resource: bundling the hook
/// into the installer is a packaging change `pnpm check` does not exercise, and
/// claiming it works without having run a bundle would be exactly the kind of
/// claim rule 12 forbids. It is recorded as follow-up work in T221.
pub fn resolve_hook_binary() -> Option<PathBuf> {
    const HOOK_BINARY_NAME: &str = "gritty-agent-hook";
    if let Some(path) = std::env::var_os(HOOK_BINARY_ENV).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(path);
        tracing::info!(path = %path.display(), "agent hook binary from {HOOK_BINARY_ENV}");
        return Some(path);
    }
    let beside = std::env::current_exe()
        .ok()?
        .parent()?
        .join(HOOK_BINARY_NAME);
    if beside.is_file() {
        Some(beside)
    } else {
        tracing::debug!(
            path = %beside.display(),
            "no agent hook binary; the hook integration is not offered"
        );
        None
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
