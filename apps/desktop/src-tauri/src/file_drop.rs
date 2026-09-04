//! Platform drag and drop, handed to the core.
//!
//! The window receives the OS drop; the core decides what the paths mean and
//! announces them; the WebView applies policy through the commands it already
//! has. Nothing here is domain logic, and the WebView never sees Tauri's own
//! `tauri://drag-drop` events — going through the core event keeps the one
//! transport rule (`AGENTS.md` rule 1) and keeps the classification testable
//! without a window.

use tauri::{DragDropEvent, Manager, Runtime, Window, WindowEvent};

use crate::core::Core;

pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    let WindowEvent::DragDrop(drag) = event else {
        return;
    };
    let app = window.app_handle();
    let Some(core) = app.try_state::<Core>() else {
        return;
    };
    let service = core.file_drop.clone();
    match drag {
        DragDropEvent::Enter { .. } => service.hover(true),
        DragDropEvent::Leave => service.hover(false),
        DragDropEvent::Drop { paths, .. } => {
            service.hover(false);
            // Classification stats every path. That is filesystem I/O, and
            // this handler runs on the UI thread, so it goes to the blocking
            // pool: a drop of a directory tree must never freeze the window.
            let paths = paths.clone();
            tauri::async_runtime::spawn_blocking(move || service.dropped(&paths));
        }
        // Fires continuously while the cursor moves over the window. Never
        // forwarded: that is the unbounded stream rule 8 forbids, and the
        // overlay only needs enter and leave.
        DragDropEvent::Over { .. } => {}
        _ => {}
    }
}
