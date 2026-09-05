-- T200: workstreams and the decisions that block them (Gate C).
-- Keyed by the canonical workspace root, like every other table here, but the
-- workstream's own id IS persisted: it is durable domain state, not a
-- per-process handle. Decisions cascade with their workstream (foreign_keys=ON).
-- No BEGIN/COMMIT here: the runner wraps each migration in one transaction.

CREATE TABLE workstream (
  id TEXT PRIMARY KEY,
  workspace_root TEXT NOT NULL,
  title TEXT NOT NULL,
  goal TEXT,
  state TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE INDEX workstream_by_workspace ON workstream (workspace_root, created_at);

CREATE TABLE decision (
  id TEXT PRIMARY KEY,
  workstream_id TEXT NOT NULL REFERENCES workstream (id) ON DELETE CASCADE,
  question TEXT NOT NULL,
  -- JSON array of { key, label }; NULL for a free-form question.
  options TEXT,
  state TEXT NOT NULL,
  chosen TEXT,
  rationale TEXT,
  -- JSON Actor: who asked. USER today, AGENT once runs exist (T330).
  raised_by TEXT NOT NULL,
  raised_at INTEGER NOT NULL,
  resolved_at INTEGER
);

CREATE INDEX decision_by_workstream ON decision (workstream_id, raised_at);
-- The attention queue reads open decisions first; keep that lookup cheap.
CREATE INDEX decision_by_state ON decision (state, raised_at);
