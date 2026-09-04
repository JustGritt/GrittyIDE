//! Gritty terminal (T090): pseudo-terminal sessions behind a small adapter.
//!
//! This crate is the only place in the tree that opens a PTY. It knows nothing
//! about workspaces, documents, ids or events: `gritty-core` owns the session
//! registry and the protocol, and reaches the operating system through
//! [`PtyHost`]. That split is what lets the core's tests run without spawning a
//! shell, and keeps `portable-pty` out of every other crate (`AGENTS.md`:
//! platform libraries stay behind small adapters).
//!
//! Module map:
//! - [`host`]: the traits and value types the core depends on.
//! - [`system`]: the real host — `portable-pty`, a reader thread, a flusher
//!   thread and escalating process-group termination.
//! - [`batch`]: output coalescing, the byte cap and UTF-8 decoding.
//! - [`shell`]: which shell runs, how, and with what environment.
//! - [`testing`]: a deterministic fake host for downstream crates.
//!
//! Terminal output is never persisted and never logged: it is user data and can
//! contain anything the user typed or a program printed, secrets included
//! (`docs/architecture/security.md`).

pub mod batch;
pub mod error;
pub mod host;
pub mod shell;
pub mod system;
pub mod testing;

pub use batch::{FLUSH_INTERVAL, MAX_PENDING_BYTES, OutputBatch};
pub use error::TerminalError;
pub use host::{PtyGrid, PtyHost, PtySession, SpawnRequest, TerminalExit, TerminalSink};
pub use shell::{ShellProfile, discover_shells, system_shells};
pub use system::SystemPtyHost;
