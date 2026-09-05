# Task T223 — Attention: notify when an agent needs you, and link it to the work

- **Phase:** 3
- **Status:** not yet started
- **Depends on:** T200 (workstream model), T210 (island), T220 (discovery), T221 (`BLOCKED`)

## Objective

Two things, both about not having to look at the island to find out that
something needs you.

1. **Notify.** When a session enters `BLOCKED` or `WAITING`, raise a system
   notification — throttled, off by default, and silent while the window is
   focused.
2. **Link.** Attach a discovered agent session to the workstream it is working
   on, so the roster stops being a flat list of processes and starts being a
   view of the work.

The second is the one Herdr structurally cannot do. It has spaces, tabs, panes
and agents, and no concept of *the task*; Gritty has had a durable workstream
and decision model since T200. Connecting the two is the point where
observability starts paying into the domain instead of sitting beside it.

## Dependencies

- **T200** for `gritty_core::workstream`, the state machine and the schema.
- **T210** for the island and the attention queue, where both features surface.
- **T220** for the roster and `AgentSession.id`.
- **T221** for `BLOCKED`. Without it, notifications can still fire on
  `WAITING`, but the headline case is missing; this task is worth more after
  T221 and should follow it.

## Required reading

- `docs/architecture/workstreams.md` — the domain, and its "deliberately not
  here" list
- `docs/architecture/agent-sessions.md` — the roster's lifetime
- `docs/architecture/persistence.md` — migrations, the blocking-worker rule
- `crates/persistence/src/migrations.rs` — `SCHEMA_VERSION`, `MIGRATIONS`
- `crates/persistence/src/migrations/0002_workstreams.sql` — the shape to follow
- `crates/gritty-core/src/workstream/` — commands, events, the transaction rule
- `apps/desktop/src/features/workstreams/island.tsx`

## Allowed scope

- `crates/persistence/src/migrations/0003_agent_links.sql` (new),
  `crates/persistence/src/{migrations.rs,store.rs}`
- `crates/gritty-core/src/{workstream,agent_session}/**`
- `crates/gritty-core/src/protocol/{mod.rs,export.rs}`
- root `Cargo.toml`, `apps/desktop/src-tauri/Cargo.toml`,
  `apps/desktop/src-tauri/src/{commands.rs,core.rs,lib.rs}`,
  `apps/desktop/src-tauri/{tauri.conf.json,capabilities/}`
- `packages/protocol/src/{client.ts,commands.ts,types.ts,index.ts}` + tests
- `apps/desktop/src/features/{agents,workstreams,session}/**`
- `docs/architecture/{agent-sessions.md,workstreams.md,persistence.md}`,
  `docs/tasks/README.md`, this file

## Explicit non-goals

- **No field on `Workstream`.** Hard rule 6 forbids provider-specific fields in
  the domain model, and a durable row must not carry an ephemeral session id.
  The link is a separate table and separate commands; `Workstream`'s shape does
  not change. See "Required behavior" §6 for the argument.
- **No control.** A linked session is still one Gritty does not own. Linking
  grants no ability to prompt, answer or cancel it.
- **No automatic linking.** No heuristic that attaches a session to a workstream
  because their working directories or titles look alike. A wrong automatic link
  writes a false claim into durable state; the user links, explicitly.
- **No state transition driven by an agent.** A `BLOCKED` session does not move
  its linked workstream to `BLOCKED`. That coupling belongs to runs Gritty owns
  (T300/T330), where the agent is a participant in the domain rather than
  something observed next to it. Raising a *decision* from an observation is
  the same mistake and is equally out of scope.
- **No notification on every change.** Only on entering a state that needs the
  user, throttled, and never while the window is focused.
- **No sound.** Herdr ships audio alerts; this task does not. A notification is
  the platform's business and the user already controls its sound.
- **No badge counts, menu-bar item or tray icon.** Separate surface, separate
  task.
- **No cross-workspace aggregation** — that is T410.

## Required behavior

### Notifications

1. A notification fires when a session **transitions into** `BLOCKED` or
   `WAITING` from something else. Never on a state that merely persists across
   ticks: the roster is polled, and a level-triggered notification would fire
   every interval.
2. **Off by default.** Enabling it is a user action in settings, and the first
   enable is what triggers the platform permission request — never app startup.
3. **Silent while the Gritty window is focused.** If the user is looking at the
   island, the island is the notification.
4. Throttled: at most one notification per session per cooldown, and a global
   cap per interval so a machine with many agents cannot produce a burst.
5. The body names the session's task and its working directory. It is
   **untrusted text** (rule 13): bounded in the core, never interpreted, and
   never a path the notification can open.
6. Clicking it focuses the Gritty window and opens the island. It does not act
   on the session.
7. New dependency: **`tauri-plugin-notification`**. Justification for the
   completion report — nothing in the tree can raise a system notification, and
   hand-rolling `UNUserNotificationCenter` would need an `unsafe_code` allow
   against a workspace-wide deny. It does not overlap an existing dependency.
   Its capability is scoped to notification only, and the WebView gains no new
   permission: the notification is raised from the **Rust** side, from the same
   place that already observes the roster diff, so the renderer neither decides
   nor sends it.

### The workstream link

8. A new table, not a column:

   ```sql
   -- 0003_agent_links.sql, SCHEMA_VERSION 2 -> 3
   CREATE TABLE workstream_agent_link (
     workstream_id  TEXT NOT NULL REFERENCES workstream(id) ON DELETE CASCADE,
     provider       TEXT NOT NULL,
     session_id     TEXT NOT NULL,
     linked_at      INTEGER NOT NULL,
     PRIMARY KEY (workstream_id, provider, session_id)
   );
   CREATE INDEX ... ON workstream_agent_link (session_id);
   ```

9. **Why a table and not a field.** Three reasons, in order of weight.
   - Hard rule 6. A `provider` on `Workstream` is precisely the
     provider-specific field the rule names, and `providerSessionId` is the
     same thing wearing a neutral name.
   - Lifetimes disagree. A workstream is durable and survives the process; a
     discovered session is in-memory, rediscovered each launch, and its id
     belongs to a process that may be long dead. Putting the ephemeral inside
     the durable would make every workstream read carry a claim that is usually
     stale.
   - Cardinality. One workstream legitimately accumulates several sessions over
     its life — resumed, restarted, a second agent on the same goal — and a
     single field would silently mean *the most recent one*.

10. Commands: `workstreams.linkAgent`, `workstreams.unlinkAgent`,
    `workstreams.agentLinks { workstreamId? }`. They follow T200's rules
    exactly: run on a blocking worker, read the database back rather than
    trusting a cache, require the workspace to be open, and fail with `Io`
    when there is no database.
11. Linking a session id that is not currently in the roster is **allowed**.
    The roster is a snapshot of *now*; a link records a fact about the past.
    Refusing would make a link impossible to create for a session that ended
    two seconds ago.
12. Reading a link whose session is not in the current roster returns the link
    with the session absent — the UI renders "linked, not running", never an
    error and never an invented row.
13. Deleting a workstream cascades its links. Removing a link never touches the
    session.
14. Events: `WorkstreamChanged` is **not** reused — a link is not a change to
    the workstream. Add `WorkstreamAgentLinkChanged` carrying the workstream id
    and its current links, or return the fresh set from the command and emit
    nothing, if nothing else needs to react. Prefer the latter and say why in
    the completion report; T047 set the precedent that a command returning its
    own snapshot need not emit.
15. Protocol version bumps MINOR; schema version 2 → 3 with the migration
    tested for both a fresh database and an upgrade from 2.

### The UI

16. An island roster row offers "Link to workstream…", choosing from the open
    workspace's workstreams; a linked row shows which one.
17. A workstream in the island and the attention queue shows its linked
    sessions, with their live activity when they are in the roster and
    "not running" when they are not.
18. Nothing is applied optimistically — T210's rule. Every mutation waits for
    the core and a refusal shows the error envelope.

## Failure behavior

| Situation | Behavior |
|---|---|
| Notification permission denied by the OS | The setting reports it and stays off; nothing retries on a loop, and the roster is unaffected |
| `tauri-plugin-notification` unavailable on a platform | The setting is absent rather than broken. No claim of support that was not run (rule 11) |
| Many sessions turn `BLOCKED` at once | The global cap collapses them; the island still shows all of them |
| Window focus unknown | Treat as focused and stay silent. A missed notification is better than one over the user's face |
| Link target workstream deleted | Cascade removes the link |
| Linked session id never returns to the roster | Renders "linked, not running" forever. Correct: it says what happened |
| Same session linked to two workstreams | Allowed; the primary key is the triple. The UI shows both |
| Duplicate link | Primary key makes it a no-op that reports so |
| No database | Every link command fails `Io`, as T200 requires. Notifications still work — they need no persistence |
| Migration 0003 fails | The runner leaves the database at version 2 (DDL is transactional); links are unavailable and the rest of the app is unaffected |

## Acceptance criteria

- With notifications enabled and Gritty in the background, a session entering
  `BLOCKED` produces exactly one notification; it produces none while the
  window is focused, and none on subsequent ticks in the same state.
- A session can be linked to a workstream from the island, the link survives a
  restart, and after the agent has exited the workstream shows it as linked and
  not running.
- Deleting the workstream removes the link; the roster row is unaffected.
- `Workstream`'s wire shape is unchanged — verified by diffing the generated
  types.
- A database at schema 2 upgrades to 3 in place, and a fresh database arrives at
  3; both are tested.
- `pnpm check` green; generated protocol types fresh and committed.

## Tests required

- **Unit (Rust):** edge-triggered notification — fires on transition into
  `BLOCKED`/`WAITING`, not on persistence, not on transition out; throttle and
  global cap hold; focused window suppresses.
- **Unit (Rust):** link CRUD, cascade on workstream delete, duplicate link,
  link to an id not in the roster, several sessions on one workstream.
- **Unit (Rust):** migration 0003 from a fresh database and from a populated
  schema-2 database, per `docs/architecture/persistence.md`.
- **Unit (Rust):** `Workstream`'s serialised shape is byte-identical to before.
- **Unit (TS):** the client methods; the island reducer's rendering of a link
  whose session is absent from the roster.
- **Manual:** enable notifications, background the window, trigger a permission
  prompt in another terminal; then link that session to a workstream, restart
  Gritty, confirm the link is still there and reads "not running".

## Commands to run

```
cargo test -p gritty-persistence
cargo test -p gritty-core
cargo clippy --workspace --all-targets -- -D warnings
pnpm check
```

## Completion report

_(filled in by the implementing session — including the `tauri-plugin-notification`
justification required by the dependency policy, and the §14 decision on whether
a link emits an event)_
