//! Repositories: one module per table, plain rows in and out, every function
//! takes a `&Connection` (a `Transaction` derefs to one) so the core can group
//! several writes into one `Database::write`.

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::PersistenceError;

type Result<T> = std::result::Result<T, PersistenceError>;

/// `session` (single row): the last workspace and UI-owned JSON blobs.
pub mod session {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub struct SessionRow {
        pub workspace_root: Option<String>,
        pub editor_preferences: Option<String>,
        pub layout: Option<String>,
        pub updated_at: i64,
    }

    pub fn load(conn: &Connection) -> Result<Option<SessionRow>> {
        conn.query_row(
            "SELECT workspace_root, editor_preferences, layout, updated_at FROM session WHERE id = 1",
            [],
            |row| {
                Ok(SessionRow {
                    workspace_root: row.get(0)?,
                    editor_preferences: row.get(1)?,
                    layout: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn set_workspace_root(conn: &Connection, root: Option<&str>, now: i64) -> Result<()> {
        conn.execute(
            "INSERT INTO session (id, workspace_root, updated_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET workspace_root = excluded.workspace_root,
                                           updated_at = excluded.updated_at",
            params![root, now],
        )?;
        Ok(())
    }

    pub fn set_editor_preferences(conn: &Connection, json: Option<&str>, now: i64) -> Result<()> {
        conn.execute(
            "INSERT INTO session (id, editor_preferences, updated_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET editor_preferences = excluded.editor_preferences,
                                           updated_at = excluded.updated_at",
            params![json, now],
        )?;
        Ok(())
    }

    pub fn set_layout(conn: &Connection, json: Option<&str>, now: i64) -> Result<()> {
        conn.execute(
            "INSERT INTO session (id, layout, updated_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET layout = excluded.layout,
                                           updated_at = excluded.updated_at",
            params![json, now],
        )?;
        Ok(())
    }
}

/// `workspace`: every root ever opened, for a later "recent workspaces" list.
pub mod workspace {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct WorkspaceRow {
        pub root: String,
        pub last_opened_at: i64,
    }

    pub fn touch(conn: &Connection, root: &str, now: i64) -> Result<()> {
        conn.execute(
            "INSERT INTO workspace (root, last_opened_at) VALUES (?1, ?2)
             ON CONFLICT(root) DO UPDATE SET last_opened_at = excluded.last_opened_at",
            params![root, now],
        )?;
        Ok(())
    }

    /// Most recently opened first.
    pub fn list(conn: &Connection) -> Result<Vec<WorkspaceRow>> {
        let mut statement = conn.prepare(
            "SELECT root, last_opened_at FROM workspace ORDER BY last_opened_at DESC, root",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(WorkspaceRow {
                root: row.get(0)?,
                last_opened_at: row.get(1)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

/// `workspace_tab`: the open tabs of a root with their per-document view state.
pub mod tabs {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub struct TabRow {
        pub path: String,
        pub position: i64,
        pub active: bool,
        pub view_mode: Option<String>,
        pub cursor_line: Option<i64>,
        pub cursor_column: Option<i64>,
        pub anchor_line: Option<i64>,
        pub anchor_column: Option<i64>,
        pub scroll_line: Option<i64>,
        pub updated_at: i64,
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<TabRow> {
        Ok(TabRow {
            path: row.get(0)?,
            position: row.get(1)?,
            active: row.get::<_, i64>(2)? != 0,
            view_mode: row.get(3)?,
            cursor_line: row.get(4)?,
            cursor_column: row.get(5)?,
            anchor_line: row.get(6)?,
            anchor_column: row.get(7)?,
            scroll_line: row.get(8)?,
            updated_at: row.get(9)?,
        })
    }

    /// Replace the tab set of `root` (positions are taken from the slice order).
    pub fn replace(conn: &Connection, root: &str, tabs: &[TabRow], now: i64) -> Result<()> {
        conn.execute(
            "DELETE FROM workspace_tab WHERE workspace_root = ?1",
            [root],
        )?;
        let mut statement = conn.prepare_cached(
            "INSERT INTO workspace_tab (workspace_root, path, position, active, view_mode,
                cursor_line, cursor_column, anchor_line, anchor_column, scroll_line, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for (position, tab) in tabs.iter().enumerate() {
            statement.execute(params![
                root,
                tab.path,
                position as i64,
                i64::from(tab.active),
                tab.view_mode,
                tab.cursor_line,
                tab.cursor_column,
                tab.anchor_line,
                tab.anchor_column,
                tab.scroll_line,
                now,
            ])?;
        }
        Ok(())
    }

    /// In position order.
    pub fn list(conn: &Connection, root: &str) -> Result<Vec<TabRow>> {
        let mut statement = conn.prepare(
            "SELECT path, position, active, view_mode, cursor_line, cursor_column, anchor_line,
                    anchor_column, scroll_line, updated_at
             FROM workspace_tab WHERE workspace_root = ?1 ORDER BY position",
        )?;
        let rows = statement.query_map([root], from_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

/// `buffer_snapshot`: the full text of every dirty document, keyed by root and path.
pub mod snapshots {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SnapshotRow {
        pub path: String,
        pub text: String,
        pub encoding: String,
        pub saved_hash: String,
        pub current_version: i64,
        pub updated_at: i64,
    }

    /// Everything but the text, for listing candidates without loading them.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SnapshotMeta {
        pub path: String,
        pub saved_hash: String,
        pub current_version: i64,
        pub updated_at: i64,
        /// UTF-8 byte length of the stored text.
        pub size: i64,
    }

    pub fn upsert(conn: &Connection, root: &str, row: &SnapshotRow) -> Result<()> {
        conn.execute(
            "INSERT INTO buffer_snapshot (workspace_root, path, text, encoding, saved_hash,
                                          current_version, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(workspace_root, path) DO UPDATE SET
                text = excluded.text, encoding = excluded.encoding,
                saved_hash = excluded.saved_hash, current_version = excluded.current_version,
                updated_at = excluded.updated_at",
            params![
                root,
                row.path,
                row.text,
                row.encoding,
                row.saved_hash,
                row.current_version,
                row.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn get(conn: &Connection, root: &str, path: &str) -> Result<Option<SnapshotRow>> {
        conn.query_row(
            "SELECT path, text, encoding, saved_hash, current_version, updated_at
             FROM buffer_snapshot WHERE workspace_root = ?1 AND path = ?2",
            params![root, path],
            |row| {
                Ok(SnapshotRow {
                    path: row.get(0)?,
                    text: row.get(1)?,
                    encoding: row.get(2)?,
                    saved_hash: row.get(3)?,
                    current_version: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    /// `true` when a row was removed.
    pub fn delete(conn: &Connection, root: &str, path: &str) -> Result<bool> {
        let removed = conn.execute(
            "DELETE FROM buffer_snapshot WHERE workspace_root = ?1 AND path = ?2",
            params![root, path],
        )?;
        Ok(removed > 0)
    }

    /// Sorted by path.
    pub fn list_meta(conn: &Connection, root: &str) -> Result<Vec<SnapshotMeta>> {
        let mut statement = conn.prepare(
            "SELECT path, saved_hash, current_version, updated_at, length(CAST(text AS BLOB))
             FROM buffer_snapshot WHERE workspace_root = ?1 ORDER BY path",
        )?;
        let rows = statement.query_map([root], |row| {
            Ok(SnapshotMeta {
                path: row.get(0)?,
                saved_hash: row.get(1)?,
                current_version: row.get(2)?,
                updated_at: row.get(3)?,
                size: row.get(4)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Rows across every root (diagnostics).
    pub fn count(conn: &Connection) -> Result<i64> {
        conn.query_row("SELECT COUNT(*) FROM buffer_snapshot", [], |row| row.get(0))
            .map_err(Into::into)
    }
}

/// `window_state`: geometry per window label, in physical pixels.
pub mod window {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub struct WindowRow {
        pub label: String,
        pub x: Option<i64>,
        pub y: Option<i64>,
        pub width: i64,
        pub height: i64,
        pub scale_factor: f64,
        pub maximized: bool,
        pub updated_at: i64,
    }

    pub fn save(conn: &Connection, row: &WindowRow) -> Result<()> {
        conn.execute(
            "INSERT INTO window_state (label, x, y, width, height, scale_factor, maximized, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(label) DO UPDATE SET
                x = excluded.x, y = excluded.y, width = excluded.width, height = excluded.height,
                scale_factor = excluded.scale_factor, maximized = excluded.maximized,
                updated_at = excluded.updated_at",
            params![
                row.label,
                row.x,
                row.y,
                row.width,
                row.height,
                row.scale_factor,
                i64::from(row.maximized),
                row.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn load(conn: &Connection, label: &str) -> Result<Option<WindowRow>> {
        conn.query_row(
            "SELECT label, x, y, width, height, scale_factor, maximized, updated_at
             FROM window_state WHERE label = ?1",
            [label],
            |row| {
                Ok(WindowRow {
                    label: row.get(0)?,
                    x: row.get(1)?,
                    y: row.get(2)?,
                    width: row.get(3)?,
                    height: row.get(4)?,
                    scale_factor: row.get(5)?,
                    maximized: row.get::<_, i64>(6)? != 0,
                    updated_at: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }
}

/// `operation_record`: durable operations, reconciled at startup.
pub mod operations {
    use super::*;

    /// Status of a persisted operation; the core maps its `OperationStatus` on it.
    pub const INTERRUPTED: &str = "INTERRUPTED";

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct OperationRow {
        pub id: String,
        pub kind: String,
        pub workspace_root: Option<String>,
        pub status: String,
        pub correlation_id: String,
        pub created_at: i64,
        pub started_at: Option<i64>,
        pub finished_at: Option<i64>,
        pub updated_at: i64,
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<OperationRow> {
        Ok(OperationRow {
            id: row.get(0)?,
            kind: row.get(1)?,
            workspace_root: row.get(2)?,
            status: row.get(3)?,
            correlation_id: row.get(4)?,
            created_at: row.get(5)?,
            started_at: row.get(6)?,
            finished_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    }

    pub fn upsert(conn: &Connection, row: &OperationRow) -> Result<()> {
        conn.execute(
            "INSERT INTO operation_record (id, kind, workspace_root, status, correlation_id,
                                           created_at, started_at, finished_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                status = excluded.status, started_at = excluded.started_at,
                finished_at = excluded.finished_at, updated_at = excluded.updated_at",
            params![
                row.id,
                row.kind,
                row.workspace_root,
                row.status,
                row.correlation_id,
                row.created_at,
                row.started_at,
                row.finished_at,
                row.updated_at
            ],
        )?;
        Ok(())
    }

    /// Startup reconciliation: whatever was still running when the process
    /// died is `INTERRUPTED`. Returns how many rows changed.
    pub fn mark_interrupted(conn: &Connection, now: i64) -> Result<usize> {
        let changed = conn.execute(
            "UPDATE operation_record SET status = ?1, finished_at = COALESCE(finished_at, ?2),
                    updated_at = ?2
             WHERE status IN ('CREATED', 'RUNNING')",
            params![INTERRUPTED, now],
        )?;
        Ok(changed)
    }

    pub fn get(conn: &Connection, id: &str) -> Result<Option<OperationRow>> {
        conn.query_row(
            "SELECT id, kind, workspace_root, status, correlation_id, created_at, started_at,
                    finished_at, updated_at
             FROM operation_record WHERE id = ?1",
            [id],
            from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Newest first.
    pub fn list(conn: &Connection) -> Result<Vec<OperationRow>> {
        let mut statement = conn.prepare(
            "SELECT id, kind, workspace_root, status, correlation_id, created_at, started_at,
                    finished_at, updated_at
             FROM operation_record ORDER BY created_at DESC, id DESC",
        )?;
        let rows = statement.query_map([], from_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}
