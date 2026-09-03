//! Gritty desktop shell: a thin Tauri composition root.
//!
//! Rules (see `AGENTS.md`):
//! - This crate only wires `gritty-core` into Tauri commands/events and owns
//!   process-level concerns (tracing, window setup).
//! - Domain logic lives in `crates/*`, never here.
//! - The WebView is never handed raw filesystem or process APIs.

mod commands;
mod core;

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

    let app = tauri::Builder::default()
        .setup(|app| {
            app.manage(core::Core::new(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::core_health,
            commands::operations_list,
            commands::operations_cancel,
            commands::operations_start_demo,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Gritty");

    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            app.state::<core::Core>().shutdown();
        }
    });
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
