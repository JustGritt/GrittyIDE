//! Gritty core: the UI-agnostic application domain.
//!
//! This crate must never depend on Tauri or any UI toolkit. The desktop app
//! (`apps/desktop/src-tauri`) is a thin composition root that wires this crate
//! into Tauri commands and events. See `AGENTS.md` and
//! `docs/architecture/overview.md`.
//!
//! Module map (T010, Gate A):
//! - [`protocol`]: wire conventions shared by every command and event
//!   (identifiers, timestamps, the error envelope, actors, versioning).
//! - [`event`]: the event envelope, the domain event vocabulary and the bus
//!   that assigns sequence numbers.
//! - [`operation`]: the asynchronous operation model (state machine, registry
//!   with cooperative cancellation, progress reporting, demo operation).
//! - [`health`]: the `core.health` report.

pub mod event;
pub mod health;
pub mod operation;
pub mod protocol;

pub use health::{HealthReport, ShellInfo, health};
pub use protocol::error::{CoreError, CoreErrorCode};
pub use protocol::{CORE_EVENT_CHANNEL, PROTOCOL_VERSION};

/// Version of the core crate, taken from `Cargo.toml` at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
