# Workstreams and decisions

Status: fixed by T200 (Gate C; ADR-005, ADR-007). `gritty_core::workstream`
owns the domain; `crates/persistence` stores it in the same database as the
session (T060); `apps/desktop/src-tauri/src/commands.rs` exposes it as
`workstreams.*` and `decisions.*`.

This is the model everything downstream attaches to. An agent adapter (T300)
translates provider behaviour into *this*; it does not bring a task model of
its own (addendum §4, Gate C).

## What a workstream is

A unit of work: a title, an optional goal, a state, and the decisions that
block it. Deliberately usable with no AI involved — you can run one from goal
to done by hand, which is what makes the model testable before any agent
exists.

| Field | Notes |
|---|---|
| `id` | UUID v7, **persisted**. Unlike `WorkspaceId`/`DocumentId`, which are per-process handles, a workstream outlives the process, so the id in the database is the id on the wire |
| `workspaceId` | The workspace as it is open right now; rows are keyed by its canonical root |
| `title`, `goal` | Trimmed; ≤ 200 and ≤ 4000 characters |
| `state` | See below |
| `openDecisions` | Derived on read from the decisions table, never stored |
| `createdAt`, `updatedAt` | Epoch milliseconds |

## The state machine

```
        ┌──────── ABANDONED ◀────────────────────────┐
        ▼                                            │
     DRAFT ──▶ READY ──▶ IN_PROGRESS ──▶ IN_REVIEW ──▶ DONE
                 ▲          │    ▲           │          │
                 └──────────┘    │           │          │
                                 ▼           │          │
                              BLOCKED ───────┘          │
                                 (▲ IN_PROGRESS)        │
                                                        │
        IN_PROGRESS ◀───────────────────────────────────┘
```

The table in `state.rs` is the contract; the diagram is a reminder.

- Every state can reach `ABANDONED` except `DONE` and `ABANDONED` themselves.
- The two terminal states each keep one way back — `DONE → IN_PROGRESS`,
  `ABANDONED → DRAFT` — so nothing forces the user to recreate a workstream and
  lose its decisions.
- Anything else is a `Conflict` that names both states and **changes nothing**:
  the check happens inside the same SQLite transaction as the write, so a
  refusal cannot leave a half-applied row.
- A transition to the state it is already in is refused too: it is not a change.

`BLOCKED` and `IN_REVIEW` are the states that mean *the user is the one holding
this up*; `WorkstreamState::needs_attention` is what the island counts and what
T410 will aggregate.

## Decisions

A question raised against a workstream, with optional structured options.

| Field | Notes |
|---|---|
| `id` | UUID v7, persisted |
| `workstreamId` | Cascade-deleted with its workstream |
| `question` | ≤ 4000 characters |
| `options` | 0–12 `{ key, label }`, unique keys. Empty means free-form |
| `state` | `OPEN`, `RESOLVED`, `DISMISSED` |
| `chosen` | An option key, or free text when there were no options |
| `rationale` | Optional prose, on either outcome |
| `raisedBy` | `Actor`. `USER` today; `AGENT { runId }` once runs exist (T335) — the seam is already in the protocol |

Rules that live in the core rather than in the UI:

- Raising a question on `IN_PROGRESS` work moves it to `BLOCKED`. Raising one
  on a draft, on already-blocked or on finished work does not move anything.
- Resolving *or dismissing* the last open question of a `BLOCKED` workstream
  returns it to `IN_PROGRESS`.
- Leaving `BLOCKED` by hand is allowed with questions still open — the user is
  the supervisor — and those questions stay open and stay in the queue.
- Resolving needs an answer: one of the offered keys, or free text. Dismissal
  records none, because inventing one would be a lie in the run log.
- Answering an already-answered decision is a `Conflict`, detected by the
  conditional `UPDATE ... WHERE state = 'OPEN'` rather than by a read-then-write.

## Storage (schema version 2)

Migration `0002_workstreams.sql`, in the session database
(`docs/architecture/persistence.md`).

| Table | Key | Columns |
|---|---|---|
| `workstream` | `id` | `workspace_root`, `title`, `goal`, `state`, `created_at`, `updated_at`; indexed by `(workspace_root, created_at)` |
| `decision` | `id` | `workstream_id` (`REFERENCES workstream ON DELETE CASCADE`), `question`, `options` (JSON), `state`, `chosen`, `rationale`, `raised_by` (JSON `Actor`), `raised_at`, `resolved_at`; indexed by `(workstream_id, raised_at)` and `(state, raised_at)` |

Every command runs on a blocking worker and reads the database back rather than
trusting a cache: there is no in-memory copy that a failed write could leave
lying. Without a database, every command fails with `Io` — a workstream that
was never stored was never created, and pretending otherwise would lose work at
exit.

## Commands and events

| Command | Notes |
|---|---|
| `workstreams.list` | Newest first, with `openDecisions` |
| `workstreams.create` | Starts as `DRAFT` |
| `workstreams.update` | Title and goal only; an absent field is left alone, a blank goal clears it |
| `workstreams.transition` | The only way the state moves |
| `workstreams.remove` | Deletes it and its decisions; `ABANDONED` is the reversible version |
| `decisions.list` | Oldest first — the order a queue triages in; filterable by workstream and by open-only |
| `decisions.raise` / `.resolve` / `.dismiss` | See the rules above |

Events: `WorkstreamCreated`, `WorkstreamChanged`, `WorkstreamRemoved`,
`DecisionRequested`, `DecisionResolved` (dismissal included; the payload's
`state` says which). `WorkstreamChanged` carries the whole workstream: the
roster is small, and a consumer that missed an event still converges.

The session tap ignores all five: the command has already committed before its
event is emitted, so there is nothing for the session writer to persist
afterwards.

Every command requires its workspace to be open (`WorkspaceManager::require`),
and a workstream belonging to another workspace reads as `NotFound` rather than
as forbidden — the caller has no business knowing it exists.

## The UI (T210)

Two surfaces over the same controller, so they cannot disagree:

- **The island** — `apps/desktop/src/features/workstreams/island.tsx`. A pill in
  the top bar (artboard 6a, placement A: it costs no layout track, so the tab
  strip never moves) that expands into a sheet hanging under the bar. It shows
  the single most urgent item in full, then the roster, then a field for adding
  a workstream. ⌥A toggles it; Escape or a click outside closes it.
- **The attention queue** — `attention-queue.tsx`. Replaces the editor body:
  filters with counts, items oldest first, detail rail on the right.

`Workstream.allowedTransitions` exists so the renderer can offer exactly the
legal moves without keeping its own copy of the state machine. Nothing is
applied optimistically: every mutation waits for the core, and a refusal is
shown as the error envelope rather than swallowed.

The two surfaces show only what the domain can produce — work, its state, and
the questions blocking it. Artboards 4a (run launcher) and 4c (conflict), and
the queue's approval and failure filters, wait for T300. Its conflict filter
arrived with ownership (T325, `ownership.md`).

## Deliberately not here

- **Agent runs, providers, capabilities, permissions** (T300/T310, ADR-005).
  `Actor::Agent { runId }` is the only seam agents need. No provider concept
  may become a `Workstream` field (rule 6).
- **`ChangeSet`, revisions, worktrees** (T100/T400, ADR-007, addendum §7). A
  ChangeSet will reference `workstream_id`; nothing here reaches into Git, and
  raw `git diff` is never the domain model.
- **Cross-workspace aggregation for the attention queue** (T410).
- **A capability/permission model** (addendum §8). The shape leaves room for
  scoped capabilities; none are implemented.
