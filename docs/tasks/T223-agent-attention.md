# Task T223 — Attention: notify when an agent needs you, and link it to the work

- **Phase:** 3
- **Status:** done
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

- `crates/persistence/src/migrations/0004_agent_links.sql` (new),
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
   -- 0004_agent_links.sql, SCHEMA_VERSION 3 -> 4
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
15. Protocol version bumps MINOR; schema version 3 → 4 with the migration
    tested for both a fresh database and an upgrade from 3.

    *Corrected during implementation.* This line and the acceptance criterion
    below said 2 → 3, which was written before T325 landed and spent schema 3
    on `0003_ownership.sql`. `migrations_are_numbered_contiguously_from_one`
    asserts `version == index + 1`, so the number claimed for this task in
    `docs/merge-queue.md` — `0004` — has to be the one actually spent.

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
| Migration 0004 fails | The runner leaves the database at version 3 (DDL is transactional); links are unavailable and the rest of the app is unaffected |

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
- A database at schema 3 upgrades to 4 in place, and a fresh database arrives at
  4; both are tested. (Corrected from "2 → 3" for the reason in §15.)
- `pnpm check` green; generated protocol types fresh and committed.

## Tests required

- **Unit (Rust):** edge-triggered notification — fires on transition into
  `BLOCKED`/`WAITING`, not on persistence, not on transition out; throttle and
  global cap hold; focused window suppresses.
- **Unit (Rust):** link CRUD, cascade on workstream delete, duplicate link,
  link to an id not in the roster, several sessions on one workstream.
- **Unit (Rust):** migration 0004 from a fresh database and from a populated
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

### Summary

Two halves that share a task file and almost nothing else.

**Notify.** `gritty_core::agent_session::attention` is the whole policy, and it
is pure — no clock, no I/O, no Tauri — because every rule in it is a rule about
*not* notifying, and those are the ones worth testing. It folds each published
roster in and answers with the sessions that just **entered** `BLOCKED` or
`WAITING`; the first roster of a process answers nothing, because those
sessions' state did not change, Gritty's knowledge did. It is advanced from
`AgentSessionManager::publish`, the one place a roster change is already
observed, so the six-second loop and the fast loop cannot disagree about what a
transition is. What crosses into the platform is an `AttentionAlert` and an
injected `AttentionNotifier`; the composition root implements it over
`tauri-plugin-notification`, and tests record instead.

**Link.** `workstream_agent_link` is a table keyed by
`(workstream_id, provider, session_id)`, and `Workstream` does not change — its
generated TypeScript is byte-identical, verified by diffing. Three commands in
the `workstreams.*` namespace follow T200's rules exactly and **emit nothing**;
they answer with the workstream's links as they are afterwards, read back from
the database. The core stores the link and publishes the roster but never joins
them: whether a session is running *now* changes between one read and the next,
so the join is the renderer's, which is what lets a link whose session has gone
render as "linked, not running" rather than vanish.

Protocol `2.11.0`, schema `4`, migration `0004_agent_links.sql`, one new
dependency.

### Files changed

**Core** — `agent_session/attention.rs` (new: the policy, the notifier trait,
the settings wire types); `agent_session/mod.rs` (the tracker, the enabled and
focused flags, `raise_attention`, `notification_settings`, `set_notifications`,
`set_window_focused`, `AgentDiscoveryConfig::with_notifier`);
`agent_session/wire.rs` (`AgentProvider::{ALL,as_str,parse}` — the link stores
the provider in a column, so the stored and transmitted spellings must be one
function); `agent_session/tests.rs`; `workstream/link.rs` (new: the link domain
and its wire types, as a second `impl WorkstreamManager`); `workstream/mod.rs`;
`workstream/tests.rs`; `protocol/export.rs`; `protocol/mod.rs`
(2.10.0 → **2.11.0**). `event/mod.rs` is **untouched** — see decision 1.

**Persistence** — `migrations/0004_agent_links.sql` (new); `migrations.rs`
(`SCHEMA_VERSION` 3 → **4**, and the fresh-database test now names the ownership
tables it had been missing since T325); `store.rs` (`agent_links` module);
`tests/database.rs`.

**Desktop shell** — `src-tauri/src/notifications.rs` (new: `PluginNotifier`,
and the two things the desktop plugin cannot do, written down);
`src-tauri/src/core.rs` (build the notifier from the app handle, like the event
sink); `src-tauri/src/lib.rs` (register the plugin, five commands, and
`on_focus_event`); `src-tauri/src/commands.rs` (five adapters);
`src-tauri/tests/commands.rs`; `src-tauri/Cargo.toml`; root `Cargo.toml`,
`Cargo.lock`. `capabilities/default.json` is **untouched**, deliberately: the
WebView is granted none of the plugin's commands.

**Protocol** — `client.ts` (`workstreams.{linkAgent,unlinkAgent,agentLinks}`,
`agents.{notifications,setNotifications}`), `commands.ts`, `types.ts`,
`workstream-client.test.ts`, `agents-client.test.ts`, and **8 new generated
files** plus `version.ts`. `Workstream.ts` is unchanged, byte for byte.

**UI** — `features/workstreams/agent-links.ts` + `.test.ts` (new: the join and
its labels, pure); `features/workstreams/agent-link-panel.tsx` (new: the island
section, the per-workstream list, the link form);
`features/agents/agent-notification-setting.tsx` (new: the switch);
`features/workstreams/{use-workstreams.ts,island.tsx,attention-queue.tsx}`;
`features/agents/agent-integration-settings.tsx`; `App.tsx` (one prop: the
queue needs the roster to say what is working on an item); `app.css`.

**Docs** — `docs/architecture/agent-sessions.md` (an "Attention (T223)" section;
"specified but not built" is now T222 and T224 only),
`docs/architecture/workstreams.md`, `docs/architecture/persistence.md` (the
header said schema 2 and the table was missing T325's two rows as well as this
one), `docs/merge-queue.md`, this file.

### Tests actually executed, and results

| Command | Result |
|---|---|
| `cargo test -p gritty-persistence` | green — 7 unit + **20** integration (18 → 20) |
| `cargo test -p gritty-core` | green — **304** (280 → 304, +24) + 3 crash-recovery |
| `cargo clippy --workspace --all-targets -- -D warnings` | green, no warnings |
| `cargo test -p gritty-desktop` | green — 16 lib (14 → 16) + **35** adapter (32 → 35) |
| `pnpm test` (vitest) | green — **500** across 66 files (491 → 500, +9) |
| `pnpm check` | green — lint, typecheck, vitest, `cargo fmt --check`, clippy, `cargo test --workspace`, generated-types freshness |

The +24 in the core are 11 for the notification policy alone (edge-trigger,
persistence across ticks, transition out, the cooldown, the global cap and its
recovery, focus, off-by-default, the first roster, bounding untrusted text,
forgetting sessions that left), 5 for the manager wiring, and 8 for the link
(a session nobody can see, restart, several sessions on one workstream and one
session on two, duplicate and absent-unlink no-ops, cascade, cross-workspace
`NotFound`, the bounds, `Io` without a database, and the wire shape).

Migration 0004 is tested **both** ways, per `persistence.md`:
`migrates_an_empty_database_to_the_latest_version` and
`migrates_from_each_prior_version` cover the fresh database and every prior
version, and
`a_database_written_before_agent_links_existed_migrates_forward_with_its_rows`
builds a real schema-3 file with a workstream and an ownership lease in it,
opens it, and asserts `from == 3`, that both rows survived, and that the new
table works against them.

**Not executed: everything on a screen.** See below.

### Architectural decisions

1. **A link returns its own snapshot and emits no event** (§14). T047's
   precedent, and the reason it applies here rather than being cited: the two
   surfaces that draw links — the island and the attention queue — read one
   `WorkstreamsController`, which is T210's rule precisely so they cannot
   disagree. The mutation's caller is therefore the only consumer, and
   `WorkstreamAgentLinkChanged` would be a contract maintained for nobody. The
   rail was the one plausible second consumer, and it reads the same controller
   through a prop instead. `changed: false` on the response is what makes a
   duplicate link and an absent unlink *report* their no-op rather than lie
   about having written. If a consumer outside this controller ever appears —
   the most likely is T326 cross-checking touched files — the event is additive
   and costs a MINOR bump then, which is cheaper than an event with no
   subscriber now (rule 9).
2. **The link is a table; `Workstream` does not grow a field.** Rule 6 is the
   first reason and the weakest to argue from alone, so two more: a durable row
   must not carry an id belonging to a process that may be long dead, and one
   workstream legitimately accumulates several sessions, which a field would
   silently reduce to *the most recent one*. Verified rather than asserted —
   `linking_does_not_change_the_workstreams_wire_shape` compares the serialised
   workstream before and after a link, the adapter test compares the whole
   listed object, and `packages/protocol/src/generated/Workstream.ts` is
   byte-identical to `origin/main`'s.
3. **The notification policy is pure and the platform is injected.** The core
   never depends on Tauri, and the rules that matter — edge-triggering, the
   throttle, the cap, focus suppression — are unit tests over a function with a
   clock argument rather than claims about a timer.
4. **The first roster of a process notifies about nothing.** §1 says "from
   something else", and a first sighting has no something-else. It is also the
   difference between launching Gritty and being told three times about work
   that has been sitting there for an hour. A session that appears on any
   *later* roster is a real arrival and does notify.
5. **Focus suppression is suppression, not deferral.** A transition that
   happened while the window was focused is not replayed when it loses focus:
   the user was looking at the island, which is where that session already was.
6. **The setting is per process and off at every launch.** §2 says off by
   default and that enabling is what asks for permission; storing "enabled"
   would mean either re-arming at startup (which §2 forbids) or inventing a
   settings-persistence mechanism this repository does not have (rule 14). The
   cost is real and is in the limitations below.
7. **`ENDED` and `IDLE` never notify.** `AgentActivity::needs_you` already
   answers this and is what the island counts; using the same predicate is what
   keeps the notification and the badge from disagreeing.
8. **The bound on links is checked after the insert, inside the transaction.**
   So a *duplicate* at the bound stays the no-op it is instead of being refused
   for asking for something already true; the refusal rolls its own insert back,
   which a test asserts rather than assumes.
9. **An unreadable stored provider skips its row and logs**, rather than failing
   the whole read. It can only come from a hand-edited database (a newer schema
   is refused outright), and losing one row from a list is a smaller lie than
   telling the user their workstreams are unreadable.
10. **`tauri-plugin-notification 2.4.0`, pinned in the workspace table.**
    *Justification, per the dependency policy:* nothing in the tree can raise a
    system notification, and hand-rolling `UNUserNotificationCenter` would need
    an `unsafe_code` allow against a workspace-wide deny. It overlaps no
    existing dependency; its only new transitive weight is `notify-rust` and the
    `objc2` notification bindings, which are platform shims rather than a
    framework. It is used from Rust only, and the WebView is granted none of its
    three commands — `capabilities/default.json` is unchanged, which is the
    enforcement, not the intent.

### Known limitations

- **macOS only, and the Linux build proves nothing about it.**
  `tauri-plugin-notification` sits under
  `[target.'cfg(target_os = "macos")'.dependencies]`; every other platform gets
  a notifier that answers `supported() == false`, so the setting is absent
  rather than broken — the failure table's own wording. Two reasons, and the
  second is the one that forced it. Nobody has run a notification on any other
  platform, so carrying the dependency everywhere would claim support that was
  never verified (rule 11). And on Linux the plugin pulls `notify-rust`, `zbus`
  and `dbus` into every test binary, which was enough to make `rust-lld` die
  with `SIGBUS` linking `gritty-core`'s lib test and `gritty-desktop`'s
  `commands` test — three CI runs, same two binaries, every time. Bringing
  notifications to a second platform is a task, not a build flag, and it starts
  from code Linux CI no longer compiles.

- **Not executed — human manual check: the notification on screen.** No agent
  session on this machine has a screen. That a notification actually appears
  while the window is backgrounded, what it looks like, and whether macOS
  prompts for permission the first time are **not verified and are not claimed**
  here. What is verified is everything up to the platform call: the transition
  detection, the throttle, the cap, the focus suppression, the off-by-default
  setting, and that the manager hands exactly one alert to an injected notifier
  for one arrival and none for the ticks after it.
- **Clicking a notification does nothing** (§6 unimplemented). The desktop half
  of `tauri-plugin-notification` spawns `notify_rust`'s `show()` and drops the
  handle, so there is no callback to focus the window from. Implementing §6
  would mean bypassing the plugin for `notify-rust` directly — a second
  dependency and a platform-specific action loop — which is a task, not a line.
  Nothing in the code or the UI copy suggests the click does anything.
- **Permission cannot be refused, reported or asked for on desktop.** The
  plugin's `request_permission` and `permission_state` both return `Granted`
  unconditionally there; the real prompt is the operating system's, raised by
  the first notification shown. So the "permission denied" branch of the failure
  table is implemented, typed and unit-tested against a fake notifier, and
  **cannot currently be reached on macOS**. The setting reports what the plugin
  says rather than inventing an answer.
- **The setting does not survive a restart.** Deliberate (decision 6), and
  still annoying: a user who wants notifications must turn them on each launch.
  The follow-up below is where it belongs.
- **Nothing was seen on a screen at all.** The island's Linked-work section, the
  linked count on a roster row, the queue's agent list and the notification
  switch have never been rendered. Every claim about them below the type level
  is reasoned from the code — the same admission T325 made, for the same reason.
- **The link form only offers sessions in the current roster.** The *command*
  deliberately accepts an id that is not, and a test pins that, but a picker of
  sessions nobody can see would be a worse control than none. Linking a session
  that ended before you got to the island is therefore not possible from the UI
  today.
- **Nothing links automatically, and nothing suggests a link.** Not even
  "this session's working directory is the open workspace root", which is the
  obvious heuristic and is exactly the false claim in durable state the non-goals
  forbid.
- **50 links per workstream and 200 characters per session id** are guesses at
  "more than anyone needs", not measurements — the same honesty T325 owed for
  its 200-lease cap.
- **The five-minute cooldown and three-per-minute cap are judgement, not data.**
  They were picked so a session flapping between `WORKING` and `WAITING` cannot
  nag and ten agents finishing at once are one interruption. Nobody has lived
  with them yet.
- **A link is not shown in the agent rail**, which is where the roster actually
  lives since T231. The island says which sessions a workstream has; the rail
  does not yet say which workstream a session belongs to.

### Follow-up tasks

- **T326 — observed violations.** T325 named this and named T223's link as the
  missing piece: a `FILE_TOUCHED` timeline entry can now be attributed to a
  workstream through the session that produced it. Both halves exist; the
  cross-check does not.
- **T224 — reveal the terminal.** A linked session that is `BLOCKED` is exactly
  the row a user wants to jump to, and the pid and tty are already on the wire.
- **A durable settings surface**, which the notification switch is the first
  real customer for. Today the only user-facing settings that survive a restart
  are the editor preferences and layout blobs, which are UI-owned; a boolean the
  *core* acts on has nowhere to live. Whoever builds it should keep §2's rule
  that restoring "enabled" must not re-ask for permission at startup.
- **§6, if it is still wanted**: click-to-focus needs `notify-rust` directly and
  a per-platform action loop.
- **Show the link in the rail**, so the roster row says which workstream it is
  working on — the mirror of what the island now does.
- **T410** aggregates attention across workspaces; the notification policy is
  already workspace-agnostic (the roster is machine-wide) and would need only a
  place to send the user.

### Remaining risks

- **The half of this task that is easiest to get wrong is the half nobody has
  seen.** Notification behaviour is judged by how it feels over a week, and
  every rule here was chosen to fail towards silence: off by default, silent
  while focused, silent on the first roster, silent inside the cooldown, silent
  past the cap. If it turns out to be *too* quiet, that is a constant to change;
  the alternative bias would have been a burst on launch, which is the failure
  users do not forgive.
- **A link is a claim the user makes, and nothing checks it.** Gritty records
  that a session is working on a workstream because someone said so. A wrong
  link is not detectable and stays until it is removed — which is the price of
  refusing the automatic heuristic, and the right price, but it is a price.
- **Two versions of the protocol may be in flight again.** This branch takes
  2.11.0 and schema 4 off `main`'s 2.10.0 and 3, and has written both into
  `docs/merge-queue.md`. Whoever merges second must check the constants rather
  than trust a clean merge — two branches agreeing on a number is the unlucky
  outcome the queue exists to prevent, and a migration number in particular
  *must* be spent in order, because
  `migrations_are_numbered_contiguously_from_one` asserts it.
- **`workstream_agent_link` is the first table whose rows point at something
  Gritty does not own.** The rows are correct as history and increasingly stale
  as fact, and every read of them has to keep saying so. The place that will
  forget is a future feature that treats a link as "the agent working on this
  right now" — which is exactly what a field on `Workstream` would have
  encouraged, and why it is not one.
