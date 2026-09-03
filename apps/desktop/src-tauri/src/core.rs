//! Composition of the core inside the Tauri process: the event sink that
//! forwards envelopes to the WebView, the event bus, the operation registry,
//! the workspace manager, the document manager and the shell service, all
//! running on Tauri's tokio runtime. Managed as Tauri state.

use std::sync::Arc;
use std::time::Duration;

use gritty_core::CORE_EVENT_CHANNEL;
use gritty_core::CoreError;
use gritty_core::document::{DocumentLimits, DocumentManager};
use gritty_core::event::{EventBus, EventEnvelope, EventSink};
use gritty_core::operation::OperationRegistry;
use gritty_core::shell::{ExternalOpener, ShellService};
use gritty_core::workspace::WorkspaceManager;
use tauri::{AppHandle, Emitter, Runtime};

use crate::shell::SystemOpener;

/// How long shutdown waits for running operations to observe cancellation.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

pub struct Core {
    pub registry: Arc<OperationRegistry>,
    pub workspaces: Arc<WorkspaceManager>,
    pub documents: Arc<DocumentManager>,
    pub shell: Arc<ShellService>,
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
    /// The production composition: external URLs go to the system handler.
    pub fn new<R: Runtime>(app: AppHandle<R>) -> Self {
        Self::with_opener(app, Arc::new(SystemOpener))
    }

    /// Same wiring with an injected opener (tests never open a browser).
    pub fn with_opener<R: Runtime>(app: AppHandle<R>, opener: Arc<dyn ExternalOpener>) -> Self {
        let events = EventBus::new(Arc::new(TauriEventSink { app }));
        // Tauri already runs a tokio runtime; the core builds on it (no second runtime).
        let handle = tauri::async_runtime::handle().inner().clone();
        let registry = OperationRegistry::new(handle.clone(), events.clone());
        let workspaces = WorkspaceManager::new(handle.clone(), events.clone(), registry.clone());
        // Installs itself as the workspace close guard (unsaved buffers).
        let documents = DocumentManager::new(
            handle.clone(),
            events,
            workspaces.clone(),
            DocumentLimits::default(),
        );
        let shell = ShellService::new(handle, opener);
        Self {
            registry,
            workspaces,
            documents,
            shell,
        }
    }

    /// Cancel every running operation; called from the `RunEvent::Exit` handler.
    pub fn shutdown(&self) {
        let running = self
            .registry
            .list()
            .into_iter()
            .filter(|op| !op.status.is_terminal())
            .count();
        tracing::info!(running, "shutting down: cancelling running operations");
        tauri::async_runtime::block_on(self.registry.shutdown(SHUTDOWN_GRACE));
    }
}
