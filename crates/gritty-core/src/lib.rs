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
//! - [`workspace`]: workspace lifecycle and directory listing (T020) over
//!   the `gritty-workspace` crate.
//! - [`document`]: open documents and their buffers (T030, Gate B): dirty
//!   state, atomic saves, external modification detection.
//! - [`git`]: read-only repository status and structured diffs (T100) over
//!   the `gritty-git` adapter. Never a `ChangeSet`, and never linked to a
//!   workstream: `git` and `workstream` do not import each other.
//! - [`shell`]: opening external URLs through an injected opener (T050).
//! - [`terminal`]: terminal sessions (T090) over the `gritty-terminal` crate:
//!   the registry, the workspace-relative working directory and the
//!   `TerminalOutput` / `TerminalExited` events.
//! - [`session`]: session persistence and crash recovery over the
//!   `gritty-persistence` database (T060): tabs and layout, dirty-buffer
//!   snapshots, startup reconciliation, durable operation records.
//! - [`workstream`]: the durable model of work (T200, Gate C): the workstream
//!   state machine and the decisions that block it, over the same database.
//! - [`agent_session`]: discovery of the coding-agent sessions already running
//!   on this machine (T220), over the `gritty-agents` adapter. Read-only, and
//!   not the agent run protocol (T300) — an observation, not something Gritty
//!   owns.

pub mod agent_session;
pub mod document;
pub mod event;
pub mod file_drop;
pub mod git;
pub mod health;
pub mod operation;
pub mod ownership;
pub mod protocol;
pub mod session;
pub mod shell;
pub mod terminal;
pub mod workspace;
pub mod workstream;

pub use health::{HealthReport, ShellInfo, health};
pub use protocol::error::{CoreError, CoreErrorCode};
pub use protocol::{CORE_EVENT_CHANNEL, PROTOCOL_VERSION};

/// Version of the core crate, taken from `Cargo.toml` at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
