# Task T200 — Workstream model (Gate C)

- **Phase:** 3
- **Status:** review
- **Depends on:** T010 (protocol), T020 (workspace), T060 (persistence)

## Objective

Give Gritty a durable domain model for *work*: a workstream with a checked
state machine, and the decisions that block it. No AI dependency, no agent
protocol, no Git — a workstream can be created, described, moved through its
states, blocked on a question and finished entirely by hand.

This is Gate C. Everything downstream (T210 UI, T300 agent runs, T400
worktrees, T410 attention queue) attaches to the model this task freezes, and
agent adapters must never define their own task/workstream state model.

## Dependencies

- T010 for ids, the error envelope, the actor, the event envelope.
- T020 for `WorkspaceManager::require` (a workstream belongs to a workspace).
- T060 for the SQLite database, the migration runner and the store pattern.

## Required reading

- `docs/Gritty IDE Architecture Refinements.md` §4 (Gate C), §5 (actors),
  §6 (event vocabulary), §7 (ChangeSet), §8 (capabilities)
- `docs/decisions/ADR-005-agent-isolation.md`, `ADR-007-git-workstreams.md`,
  `ADR-004-persistence.md`, `ADR-014-session-persistence.md`
- `docs/architecture/persistence.md`, `docs/architecture/overview.md`
- `crates/gritty-core/src/session/` (service + store + wire shape to copy)
- `crates/persistence/src/{migrations.rs,store.rs}`

## Allowed scope

- `crates/gritty-core/src/workstream/**` (new)
- `crates/gritty-core/src/protocol/ids.rs` (two new ids)
- `crates/gritty-core/src/protocol/export.rs`, `src/event/mod.rs`, `src/lib.rs`
- `crates/persistence/src/migrations{.rs,/0002_workstreams.sql}`, `src/store.rs`
- `apps/desktop/src-tauri/src/{commands.rs,core.rs,lib.rs}`
- `packages/protocol/src/{client.ts,commands.ts,index.ts}` + its tests
- `docs/architecture/workstreams.md`, `docs/tasks/README.md`, this file

## Explicit non-goals

- No agent protocol, run lifecycle, provider adapter or capability enforcement
  (T300/T310 own those). `Decision.raisedBy` is an `Actor`, which is the only
  seam agents need.
- No ChangeSet table and no Git (T100/T400). The relationship is documented,
  not implemented.
- No UI. T210 consumes this.
- No worktrees, no attention-queue aggregation across workspaces (T410).
- No capability/permission model (§8) beyond leaving room for it.

## Required behavior

1. A workstream has an id, a workspace, a title, an optional goal, a state and
   timestamps. Ids are persisted (unlike `WorkspaceId`/`DocumentId`, which are
   per process): a workstream is durable domain state, not a handle.
2. States: `DRAFT`, `READY`, `IN_PROGRESS`, `BLOCKED`, `IN_REVIEW`, `DONE`,
   `ABANDONED`. Only the transitions in `state.rs` are legal; every other one
   fails with `Conflict` and changes nothing.
3. A decision is a question raised against a workstream: question, optional
   options, `OPEN | RESOLVED | DISMISSED`, the chosen answer, a rationale and
   the actor that raised it.
4. Raising a decision on an `IN_PROGRESS` workstream blocks it. Resolving or
   dismissing the last open decision unblocks it back to `IN_PROGRESS`. This is
   the "needs you" signal the UI reads; it is domain logic, not UI logic.
5. Everything survives restart: `workstreams.list` after a relaunch returns
   what was there, with its decisions.
6. Commands: `workstreams.list|create|update|transition|remove`,
   `decisions.list|raise|resolve|dismiss`.
7. Events: `WorkstreamCreated`, `WorkstreamChanged`, `WorkstreamRemoved`,
   `DecisionRequested`, `DecisionResolved`, on the existing single channel.
8. Protocol version bumps MINOR (additive).

## Failure behavior

- Persistence unavailable → every command fails with `Io` and a message saying
  workstreams need the session database. Nothing is kept in memory and lost
  later: a workstream that was never written never existed.
- Unknown workstream/decision id → `NotFound`.
- Blank title, blank question, option key collisions, over-long fields →
  `InvalidInput` naming the field, no side effect.
- Illegal transition → `Conflict` naming both states.
- Resolving an already-resolved decision → `Conflict`.
- A write that fails mid-transaction rolls back; the in-memory view is read
  from the database, so a failed write leaves no phantom state.
- Workspace closed or root missing → `NotFound` from `WorkspaceManager::require`.

## Acceptance criteria

- Workstreams and decisions survive a process restart (test reopens the
  database file and lists them).
- Every illegal transition is rejected with `Conflict` and leaves the stored
  state untouched (exhaustive test over the transition matrix).
- Raising and resolving decisions moves the workstream in and out of `BLOCKED`.
- `cargo test -p gritty-core -p gritty-persistence` and `pnpm check` are green,
  generated TypeScript is committed and fresh.
- No Tauri or UI type appears in `gritty-core`.

## Tests required

- `state.rs`: the full transition matrix, legal and illegal.
- Manager: create/list/update/transition/remove, decision raise/resolve/dismiss,
  the block/unblock rule, every error path above, events emitted with the right
  actor and workspace id.
- Persistence: migration 0002 applies to a fresh and to a v1 database; store
  round-trip; cascade delete of decisions.
- Protocol client: command names, request wrapping (`workstream-client.test.ts`).
- Manual: none (no UI yet).

## Commands to run

```
cargo test -p gritty-core -p gritty-persistence
pnpm check
```

## Completion report

### Summary

Gate C. `gritty_core::workstream` now owns a durable model of work: a
workstream (title, goal, seven-state machine) and the decisions that block it,
stored in the session database, exposed as `workstreams.*` / `decisions.*` and
announced with five new events. No agent, no Git, no UI — a workstream can be
run from goal to done by hand, which is what makes the model testable before
any agent exists.

Two rules that could have been left to the UI live in the core instead, because
a UI that forgets them would silently lose the "needs you" signal: raising a
question on `IN_PROGRESS` work blocks it, and answering or dismissing the last
open question unblocks it. Leaving `BLOCKED` by hand is still allowed, with the
questions left open.

### Files changed

| File | What |
|---|---|
| `crates/gritty-core/src/workstream/state.rs` | the state machine, its transition table and its refusals |
| `crates/gritty-core/src/workstream/wire.rs` | wire types, validation helpers, limits |
| `crates/gritty-core/src/workstream/mod.rs` | the manager: nine commands over the database |
| `crates/gritty-core/src/workstream/tests.rs` | 14 manager tests against a real SQLite file |
| `crates/gritty-core/src/protocol/ids.rs` | `WorkstreamId`, `DecisionId` (persisted ids) |
| `crates/gritty-core/src/protocol/{mod,export}.rs` | protocol 2.2.0 → 2.3.0; eleven new exported types |
| `crates/gritty-core/src/event/mod.rs`, `src/session/tap.rs` | five new events; the tap ignores them, with a reason |
| `crates/gritty-core/src/lib.rs` | module map |
| `crates/persistence/src/migrations{.rs,/0002_workstreams.sql}` | schema version 2 |
| `crates/persistence/src/store.rs` | `workstreams` and `decisions` repositories |
| `crates/persistence/tests/database.rs` | round-trip, ordering, cascade, double-close |
| `apps/desktop/src-tauri/src/{commands,core,lib}.rs` | nine command adapters; the manager shares the session database |
| `packages/protocol/src/{client,commands,types}.ts` + generated | the `workstreams` and `decisions` namespaces |
| `packages/protocol/src/workstream-client.test.ts` | command names, request wrapping, error surfacing |
| `docs/architecture/workstreams.md`, `persistence.md`, `docs/tasks/README.md` | the durable decisions |

### Tests actually executed

- `cargo test -p gritty-persistence` — 15 passed (2 new).
- `cargo test -p gritty-core` — 160 passed (25 new: 6 state machine, 6 wire,
  13 manager).
- `pnpm vitest run packages/protocol` — 44 passed (4 new).
- `pnpm check` — Biome, tsc, 297 vitest tests, `cargo fmt --check`,
  `clippy -D warnings`, `cargo test --workspace`, generated-types freshness:
  green (the freshness step needs the regenerated bindings committed, which
  they are).

Not executed: nothing in the running app — T200 adds no UI. First manual
exercise happens in T210.

### Architectural decisions

1. **Workstream ids are persisted**, unlike `WorkspaceId`/`DocumentId`. A
   workstream outlives the process, so the row id is the wire id. Rows are
   still keyed by the canonical workspace root, like the rest of the session
   schema, and every command goes through `WorkspaceManager::require` — a
   workstream of another workspace reads as `NotFound`, not as forbidden.
2. **One database, one migration chain.** Workstreams live in the session
   database rather than a second file: one integrity check, one moved-aside
   path, one schema version.
3. **Synchronous write-through, no cache.** Session tabs are debounced and
   best-effort; a workstream is not. The command commits before it returns and
   the result is read back from the transaction, so a failed write leaves no
   phantom. Without a database every command fails with `Io` rather than
   keeping work in memory that would vanish at exit.
4. **Domain refusals travel inside the SQLite transaction** (a `CoreError`
   carried in the error the `write` closure returns, unwrapped on the way out).
   That is what makes "an illegal transition changes nothing" true rather than
   merely intended — the rollback does it, not care.
5. **The blocking rules are domain logic**, per above.
6. **`Decision.raisedBy` is an `Actor`.** The only seam T330 needs; no provider
   concept enters the model (rule 6).

### Known limitations

- One workspace at a time, inherited from T020: `workstreams.list` is scoped to
  the open workspace, so there is no cross-workspace queue yet (T410's job).
- No ordering or priority beyond newest-first workstreams and oldest-first
  decisions; the UI sorts by attention.
- `options` is JSON in one column. Fine at ≤ 12 options; a `decision_option`
  table would be needed only if options ever have to be queried.
- No pagination. A workspace with thousands of workstreams would list them all;
  the roster is a human artefact, so this is a deliberate bet, not an oversight.
- `WorkstreamState::needs_attention` exists but nothing consumes it yet.

### Follow-up tasks

- **T210 Workstream UI** — the island (artboards 5a/5b) and the attention queue
  (4b) over this model. Next.
- **ChangeSet** — when T100 lands, a `change_set` table referencing
  `workstream_id` (ADR-007, addendum §7). Not before Git exists.
- **Cross-workspace attention** (T410) needs `decisions.list` without a
  workspace filter, which means a root→display-name lookup for the UI.
- **Session-restore integration**: `session.restore` could report how many
  workstreams need attention, so the launcher's resume rail can say so.

### Remaining risks

- The state machine is now frozen by Gate C. If T300 turns out to need a state
  the seven do not cover (a run that is *starting*, say), that is an ADR and a
  migration, not a quiet addition.
- Nothing here has been exercised through the real Tauri command path yet; the
  adapters are thin and typed, but T210 is where that gets proven.
