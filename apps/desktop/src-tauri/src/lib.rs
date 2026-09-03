//! Gritty desktop shell: a thin Tauri composition root.
//!
//! Rules (see `AGENTS.md`):
//! - This crate only wires `gritty-core` into Tauri commands/events and owns
//!   process-level concerns (tracing, window setup).
//! - Domain logic lives in `crates/*`, never here.
//! - The WebView is never handed raw filesystem or process APIs.

mod commands;
pub mod core;

use tauri::Manager;
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

/// Register every command adapter and schedule [`attach_core`] for setup.
/// Generic over the runtime so tests can drive the same wiring on Tauri's mock
/// runtime (which never runs `setup`; tests call [`attach_core`] themselves).
pub fn configure<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .setup(|app| {
            attach_core(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::core_health,
            commands::operations_list,
            commands::operations_cancel,
            commands::operations_start_demo,
        ])
}

/// Build the core (event bus + operation registry) and manage it as app state.
pub fn attach_core<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    app.manage(core::Core::new(app.clone()));
}

fn init_tracing() {
    let filter = EnvFilter::try_from_env(LOG_ENV)
        .unwrap_or_else(|_| EnvFilter::new("info,gritty_desktop_lib=debug,gritty_core=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_writer(std::io::stderr)
        .init();
}
