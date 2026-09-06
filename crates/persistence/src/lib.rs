//! Gritty persistence (T060, ADR-004, ADR-014): the SQLite database that holds
//! application metadata (workspaces, session layout, window geometry, tab
//! state), dirty-buffer recovery snapshots and durable operation records.
//!
//! This crate knows nothing about the core's types: repositories take and
//! return plain rows (strings, integers), the core maps them onto its domain.
//! It never depends on `gritty-core` or Tauri. Repository files stay the
//! source of truth for project content; a snapshot here never replaces a file
//! on disk by itself.

pub mod db;
pub mod error;
pub mod migrations;
pub mod store;
pub mod testing;

pub use db::{DB_FILE_NAME, Database, Opened, now_ms};
pub use error::PersistenceError;
pub use migrations::{MIGRATIONS, Migration, MigrationReport, SCHEMA_VERSION};
/// Re-exported so a caller can name the handle a `store` function takes
/// without depending on `rusqlite` itself.
pub use rusqlite::Connection;
