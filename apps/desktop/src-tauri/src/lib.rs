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
//! - Session persistence (T060): the application data directory is resolved
//!   here (`tauri::path`, never from the WebView), window geometry is
//!   captured from window events and restored before the window shows.

mod commands;
pub mod core;
mod file_drop;
mod shell;
pub mod window_state;

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use tauri::{Manager, Url};

pub use core::CoreOptions;
use tracing_subscriber::EnvFilter;

/// Environment variable controlling log filtering, e.g. `GRITTY_LOG=debug`.
const LOG_ENV: &str = "GRITTY_LOG";

/// Instant [`run`] began. Startup milestones are logged relative to it so the
/// Phase 0 performance baseline (`docs/plans/phase-0-spike.md` §C) reads launch
/// timings out of the log instead of inferring them from line timestamps.
static STARTED: OnceLock<Instant> = OnceLock::new();

/// Log one startup milestone with the time since [`run`] began.
///
/// Silent when the process did not go through [`run`] — the mock runtime in
/// tests has no start instant, and a milestone measured from nothing would be
/// a fabricated number.
pub(crate) fn log_startup_milestone(milestone: &'static str) {
    if let Some(started) = STARTED.get() {
        emit_startup_milestone(milestone, started.elapsed());
    }
}

fn emit_startup_milestone(milestone: &'static str, elapsed: std::time::Duration) {
    tracing::info!(
        milestone,
        elapsed_ms = elapsed.as_millis() as u64,
        "startup milestone"
    );
}

/// Captures rendered log output so a test can assert on the lines the Phase 0
/// measurements parse, rather than on the call that produced them.
#[cfg(test)]
pub(crate) mod log_capture {
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

    impl Captured {
        pub(crate) fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().expect("captured log")).into_owned()
        }
    }

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("captured log").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Runs `body` with every event at DEBUG or above rendered into the
    /// returned text.
    pub(crate) fn capture<T>(body: impl FnOnce() -> T) -> (T, String) {
        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .finish();
        let value = tracing::subscriber::with_default(subscriber, body);
        let text = captured.text();
        (value, text)
    }
}

/// Start the desktop application. Blocks until the app exits.
pub fn run() {
    let _ = STARTED.set(Instant::now());
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
    log_startup_milestone("application built");

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
        .on_window_event(|window, event| {
            window_state::on_window_event(window, event);
            file_drop::on_window_event(window, event);
        })
        .setup(|app| {
            attach_core(app.handle());
            window_state::restore_main_window(app.handle());
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
            commands::session_restore,
            commands::session_candidates,
            commands::session_recover,
            commands::session_discard,
            commands::session_update,
            commands::terminal_spawn,
            commands::terminal_write,
            commands::terminal_resize,
            commands::terminal_kill,
            commands::terminal_list,
        ])
}

/// Build the core (event bus, operation registry, workspace and document
/// managers, shell and session services) and manage it as app state.
pub fn attach_core<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let data_dir = core::resolve_data_dir(app);
    attach_core_with(
        app,
        CoreOptions {
            opener: Arc::new(shell::SystemOpener),
            data_dir,
            pty_host: Arc::new(gritty_terminal::SystemPtyHost::new()),
        },
    );
}

/// [`attach_core`] with injected dependencies: tests use a recording opener
/// so nothing ever opens a browser, and a temporary data directory so no test
/// touches the real session database.
pub fn attach_core_with<R: tauri::Runtime>(app: &tauri::AppHandle<R>, options: CoreOptions) {
    app.manage(window_state::WindowTracker::default());
    app.manage(core::Core::with_options(app.clone(), options));
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
    fn a_startup_milestone_reports_the_elapsed_time_in_milliseconds() {
        let (_, log) = log_capture::capture(|| {
            emit_startup_milestone("window shown", std::time::Duration::from_millis(1234));
        });
        assert!(log.contains("startup milestone"), "{log}");
        assert!(log.contains("milestone=\"window shown\""), "{log}");
        assert!(log.contains("elapsed_ms=1234"), "{log}");
    }

    #[test]
    fn no_milestone_is_logged_when_the_process_never_went_through_run() {
        // `run` is the only place that sets `STARTED`; tests and the mock
        // runtime must not produce a milestone measured from nothing.
        assert!(STARTED.get().is_none());
        let (_, log) = log_capture::capture(|| log_startup_milestone("window shown"));
        assert!(log.is_empty(), "{log}");
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
