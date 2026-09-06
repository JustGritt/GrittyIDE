-- T223: which observed agent session is working on which workstream.
--
-- A table and not a column on `workstream`, for three reasons in order of
-- weight. Hard rule 6: a `provider` on the domain model is exactly the
-- provider-specific field the rule names. Lifetimes: a workstream is durable
-- and a discovered session is in-memory, rediscovered each launch, with an id
-- belonging to a process that may be long dead — putting the ephemeral inside
-- the durable would make every workstream read carry a usually-stale claim.
-- Cardinality: one workstream legitimately accumulates several sessions over
-- its life (resumed, restarted, a second agent on the same goal), and a single
-- field would silently mean *the most recent one*.
--
-- The primary key is the triple, so the same session may be linked to two
-- workstreams and a duplicate link is a no-op rather than a second row. Links
-- cascade with their workstream (foreign_keys=ON); removing a link never
-- touches the session, which Gritty does not own in the first place.
-- No BEGIN/COMMIT here: the runner wraps each migration in one transaction.

CREATE TABLE workstream_agent_link (
  workstream_id TEXT NOT NULL REFERENCES workstream (id) ON DELETE CASCADE,
  -- The wire's `AgentProvider`, e.g. CLAUDE_CODE. Stored as its own column
  -- rather than folded into the session id, so the id stays exactly what the
  -- roster reported and the two are joined on the same pair the roster is.
  provider TEXT NOT NULL,
  session_id TEXT NOT NULL,
  linked_at INTEGER NOT NULL,
  PRIMARY KEY (workstream_id, provider, session_id)
);

-- "Which workstreams is this session working on" — the lookup the rail makes
-- per roster row, and the one the composite primary key cannot serve.
CREATE INDEX workstream_agent_link_by_session ON workstream_agent_link (session_id);
