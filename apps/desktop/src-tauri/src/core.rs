//! Composition of the core inside the Tauri process: the event sink that
//! forwards envelopes to the WebView, the event bus and the operation
//! registry running on Tauri's tokio runtime. Managed as Tauri state.

use std::sync::Arc;
use std::time::Duration;

use gritty_core::CORE_EVENT_CHANNEL;
use gritty_core::CoreError;
use gritty_core::event::{EventBus, EventEnvelope, EventSink};
use gritty_core::operation::OperationRegistry;
use tauri::{AppHandle, Emitter};

/// How long shutdown waits for running operations to observe cancellation.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

pub struct Core {
    pub registry: Arc<OperationRegistry>,
}

/// Delivers every envelope on the single core event channel. Failures are
/// reported back to the bus, which logs them with the correlation id.
struct TauriEventSink {
    app: AppHandle,
}

impl EventSink for TauriEventSink {
    fn emit(&self, envelope: &EventEnvelope) -> Result<(), CoreError> {
        self.app
            .emit(CORE_EVENT_CHANNEL, envelope)
            .map_err(|error| CoreError::internal(format!("event delivery failed: {error}")))
    }
}

impl Core {
    pub fn new(app: AppHandle) -> Self {
        let events = EventBus::new(Arc::new(TauriEventSink { app }));
        // Tauri already runs a tokio runtime; the core builds on it (no second runtime).
        let handle = tauri::async_runtime::handle().inner().clone();
        let registry = OperationRegistry::new(handle, events);
        Self { registry }
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
