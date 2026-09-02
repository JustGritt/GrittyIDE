//! Tauri command adapters.
//!
//! Temporary surface for T000. T010 replaces this with the typed core protocol
//! (command/event/error envelope) and the `health` command keeps its name so
//! the protocol client can map onto it without changes here.

use serde::Serialize;

/// Health information returned to the WebView by the `health` command.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppHealth {
    /// Version of the desktop app (Cargo package version).
    pub app_version: String,
    /// Version of the `tauri` runtime crate.
    pub tauri_version: &'static str,
    /// Version string of the system WebView, when detectable (Phase 0 evidence).
    pub webview_version: Option<String>,
    /// Facts reported by `gritty-core`.
    pub core: gritty_core::HealthReport,
}

/// Round-trip probe proving the WebView <-> core IPC path works.
#[tauri::command]
pub fn health(app: tauri::AppHandle) -> AppHealth {
    let report = AppHealth {
        app_version: app.package_info().version.to_string(),
        tauri_version: tauri::VERSION,
        webview_version: tauri::webview_version().ok(),
        core: gritty_core::health(),
    };
    tracing::info!(?report, "health requested by webview");
    report
}
