//! Gritty desktop shell: a thin Tauri composition root.
//!
//! Rules (see `AGENTS.md`):
//! - This crate only wires `gritty-core` into Tauri commands/events and owns
//!   process-level concerns (tracing, window setup).
//! - Domain logic lives in `crates/*`, never here.
//! - The WebView is never handed raw filesystem or process APIs. The dialog
//!   plugin is registered so the core can show the native directory picker
//!   from Rust; the WebView is granted none of its commands. External URLs
//!   are opened by the core's shell service (T050) through [`shell::SystemOpener`],
//!   and a navigation guard keeps the WebView on the application origin.

mod commands;
pub mod core;
mod shell;

use std::sync::Arc;

use gritty_core::shell::ExternalOpener;
use tauri::{Manager, Url};
use tracing_subscriber::EnvFilter;

/// Environment variable controlling log filtering, e.g. `GRITTY_LOG=debug`.
const LOG_ENV: &str = "GRITTY_LOG";

/// Start the desktop application. Blocks until the app exits.
pub fn run() {
    init_tracing();
    tracing::info!(
        app_version = env!("CARGO_PKG_VERSION"),
        core_version = gritty_core::VERSION,
        protocol_version = gritty_core::PROTOCOL_VERSION,
        "starting Gritty"
    );

    let app = configure(tauri::Builder::default())
        .build(tauri::generate_context!())
        .expect("error while building Gritty");

    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            app.state::<core::Core>().shutdown();
        }
    });
}

/// Register plugins and every command adapter, and schedule [`attach_core`]
/// for setup. Generic over the runtime so tests can drive the same wiring on
/// Tauri's mock runtime (which never runs `setup`; tests call [`attach_core`]
/// themselves).
pub fn configure<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(navigation_guard())
        .setup(|app| {
            attach_core(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::core_health,
            commands::operations_list,
            commands::operations_cancel,
            commands::operations_start_demo,
            commands::workspace_open,
            commands::workspace_close,
            commands::workspace_current,
            commands::workspace_list,
            commands::documents_open,
            commands::documents_close,
            commands::documents_update,
            commands::documents_save,
            commands::documents_reload,
            commands::documents_status,
            commands::documents_list,
            commands::shell_open_external,
        ])
}

/// Build the core (event bus, operation registry, workspace and document
/// managers, shell service) and manage it as app state.
pub fn attach_core<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    attach_core_with(app, Arc::new(shell::SystemOpener));
}

/// [`attach_core`] with an injected opener for external URLs; tests use a
/// recording fake so nothing ever opens a browser.
pub fn attach_core_with<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    opener: Arc<dyn ExternalOpener>,
) {
    app.manage(core::Core::with_opener(app.clone(), opener));
}

/// A link the WebView tries to follow (a click the preview did not intercept,
/// a context-menu "open link", a script) must never navigate the app away
/// from its own origin: external URLs go through `shell.openExternal`, which
/// validates them and opens the system handler instead.
fn navigation_guard<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("gritty-navigation-guard")
        .on_navigation(|_webview, url| {
            let allowed = is_app_navigation(url, cfg!(debug_assertions));
            if !allowed {
                tracing::warn!(
                    scheme = url.scheme(),
                    host = url.host_str().unwrap_or_default(),
                    "blocked webview navigation outside the app origin"
                );
            }
            allowed
        })
        .build()
}

/// The application's own origins: `tauri://localhost` (macOS, Linux),
/// `http://tauri.localhost` (Windows), and the Vite dev server in debug builds.
pub fn is_app_navigation(url: &Url, allow_dev_server: bool) -> bool {
    match url.scheme() {
        "tauri" => true,
        "http" | "https" => {
            let host = url.host_str().unwrap_or_default();
            host == "tauri.localhost"
                || (allow_dev_server && matches!(host, "localhost" | "127.0.0.1"))
        }
        _ => false,
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_env(LOG_ENV).unwrap_or_else(|_| {
        EnvFilter::new("info,gritty_desktop_lib=debug,gritty_core=debug,gritty_workspace=debug")
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_writer(std::io::stderr)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("test url")
    }

    #[test]
    fn navigation_stays_on_the_app_origin() {
        assert!(is_app_navigation(
            &url("tauri://localhost/index.html"),
            false
        ));
        assert!(is_app_navigation(&url("http://tauri.localhost/"), false));
        assert!(is_app_navigation(&url("http://localhost:1420/"), true));
        assert!(!is_app_navigation(&url("http://localhost:1420/"), false));
        assert!(!is_app_navigation(&url("https://example.com/"), true));
        assert!(!is_app_navigation(
            &url("https://tauri.localhost.evil.com/"),
            true
        ));
        assert!(!is_app_navigation(&url("file:///etc/passwd"), true));
        assert!(!is_app_navigation(&url("javascript:alert(1)"), true));
        assert!(!is_app_navigation(&url("data:text/html,hi"), true));
    }
}
