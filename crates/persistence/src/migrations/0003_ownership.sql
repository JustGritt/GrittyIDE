-- T325: write-ownership leases over parts of the workspace, and the requests
-- they refused. Keyed by the canonical workspace root like every other table
-- here; the lease's own id IS persisted, because a lease outlives the process
-- that granted it — which is exactly what makes a stale lease possible.
--
-- A lease is held by a workstream, never by an agent session: hard rule 6, and
-- a durable row must not carry an id that only exists while a process does.
-- `agent_run_id` is the seam T300 fills; nothing writes it today.
--
-- Everything cascades from the workstream. A conflict between two workstreams
-- is meaningless once either of them is gone, so it references both, and the
-- lease it is waiting on as well (foreign_keys=ON).
-- No BEGIN/COMMIT here: the runner wraps each migration in one transaction.

CREATE TABLE ownership_lease (
  id TEXT PRIMARY KEY,
  workspace_root TEXT NOT NULL,
  workstream_id TEXT NOT NULL REFERENCES workstream (id) ON DELETE CASCADE,
  -- The run that claimed it (T300). Always NULL today.
  agent_run_id TEXT,
  -- FILE | DIRECTORY | GLOB, with the pattern the core compiled it to.
  target_kind TEXT NOT NULL,
  target_pattern TEXT NOT NULL,
  mode TEXT NOT NULL,
  note TEXT,
  acquired_at INTEGER NOT NULL,
  -- NULL means it holds until released. EXPIRED is derived from this on read
  -- and never stored: a state a clock decides must not be able to disagree
  -- with the clock.
  expires_at INTEGER,
  state TEXT NOT NULL,
  released_at INTEGER
);

-- Acquisition reads every lease of one workspace that still holds, on every
-- request; that is the lookup to keep cheap.
CREATE INDEX ownership_lease_by_workspace ON ownership_lease (workspace_root, state, acquired_at);
CREATE INDEX ownership_lease_by_workstream ON ownership_lease (workstream_id, state);

CREATE TABLE ownership_conflict (
  id TEXT PRIMARY KEY,
  workspace_root TEXT NOT NULL,
  requested_by TEXT NOT NULL REFERENCES workstream (id) ON DELETE CASCADE,
  owned_by TEXT NOT NULL REFERENCES workstream (id) ON DELETE CASCADE,
  holding_lease_id TEXT NOT NULL REFERENCES ownership_lease (id) ON DELETE CASCADE,
  -- What was asked for, not what is held: they overlap, they are rarely the
  -- same shape, and the refusal is about the request.
  target_kind TEXT NOT NULL,
  target_pattern TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  state TEXT NOT NULL,
  resolved_at INTEGER
);

CREATE INDEX ownership_conflict_by_workspace ON ownership_conflict (workspace_root, created_at);
-- The attention queue reads the WAITING ones first; keep that lookup cheap.
CREATE INDEX ownership_conflict_by_state ON ownership_conflict (state, created_at);
