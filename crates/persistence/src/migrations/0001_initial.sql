-- T060: session state and recovery snapshots (docs/architecture/persistence.md).
-- Keys are the canonical workspace root and the workspace-relative path;
-- WorkspaceId / DocumentId are per process and never persisted.
-- No BEGIN/COMMIT here: the runner wraps each migration in one transaction.

CREATE TABLE workspace (
  root TEXT PRIMARY KEY,
  last_opened_at INTEGER NOT NULL
);

CREATE TABLE session (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  workspace_root TEXT,
  editor_preferences TEXT,
  layout TEXT,
  updated_at INTEGER NOT NULL
);

CREATE TABLE window_state (
  label TEXT PRIMARY KEY,
  x INTEGER,
  y INTEGER,
  width INTEGER NOT NULL,
  height INTEGER NOT NULL,
  scale_factor REAL NOT NULL,
  maximized INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL
);

CREATE TABLE workspace_tab (
  workspace_root TEXT NOT NULL,
  path TEXT NOT NULL,
  position INTEGER NOT NULL,
  active INTEGER NOT NULL DEFAULT 0,
  view_mode TEXT,
  cursor_line INTEGER,
  cursor_column INTEGER,
  anchor_line INTEGER,
  anchor_column INTEGER,
  scroll_line INTEGER,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (workspace_root, path)
);

CREATE TABLE buffer_snapshot (
  workspace_root TEXT NOT NULL,
  path TEXT NOT NULL,
  text TEXT NOT NULL,
  encoding TEXT NOT NULL,
  saved_hash TEXT NOT NULL,
  current_version INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (workspace_root, path)
);

CREATE TABLE operation_record (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  workspace_root TEXT,
  status TEXT NOT NULL,
  correlation_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  started_at INTEGER,
  finished_at INTEGER,
  updated_at INTEGER NOT NULL
);
