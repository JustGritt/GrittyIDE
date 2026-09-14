# Task T110 — Diagnostics: make provider and runtime failures debuggable without console spelunking

- **Phase:** 2
- **Status:** todo
- **Depends on:** ADR-001 (desktop runtime gate, accepted 2026-09-14), T010
  (core protocol), T060 (`PersistenceStatus`), T090 (`TerminalManager::list`),
  T220 (`AgentRoster`), T325 (`OwnershipMap`)

> **Read first — this spec predates two things that have since landed**
> (noted 2026-09-14, not yet re-specified). **T070** (the file watcher) is
> done, so the "watchers" row below is no longer absent for lack of a
> watcher; whether the watch exposes anything countable is for whoever takes
> this ticket to check. **The T300 run stack** (T300-C/CORE/FAKE/CMD/WIRE,
> T310-A/B/U) is done, so `AgentRunManager` exists; T305 (the capability
> model) does not, so per-provider adapter health still has no source. Every
> "T070 absent" and "T300 does not exist" line below is stale in that sense;
> re-decide §3–4 and the matching acceptance criterion before building.

## Objective

Give the desktop app one developer diagnostics surface — a small backend seam
plus a panel — reporting what the running core already tracks: workspace
ownership, discovered agent sessions, terminal/child processes, session
persistence health, the core event channel's own delivery health, and a
bounded history of recent command failures. It replaces the one-line health
dot (`core.health`, `crates/gritty-core/src/health.rs`,
`apps/desktop/src/features/shell/app-bar.tsx:50`) and ends this codebase's one
console-only escape hatch: `packages/protocol/src/events.ts`'s `Diagnostic`
callback, which funnels dropped, malformed and out-of-sequence core events
straight to `console.warn` (`client.ts:341`) and nowhere else.

Project Plan §14 (`docs/Gritty IDE Project Plan.md:486,661`) states T110's
acceptance in one line: "Core/process errors are inspectable without console
spelunking." The Coordination plan's §35 mocks up three blocks — agent
runtime, ownership, events — written before most of what they describe
existed; most of it still doesn't. The real work here is telling the two
apart: a real number where one is measurable, "not yet available" everywhere
else — hard rule 12's sibling for a *display*, not just a completion report.

## Dependencies

- **ADR-001** — accepted 2026-09-14, so Phase 2 is unblocked and this task
  can be built. (When this spec was written it was still `Proposed` and gated
  every unstarted Phase 2 task.)
- **T010** for the command/event/error conventions this task builds on.
- **T060** for `SessionService::status()`, already synchronous and in-memory.
- **T090** for `TerminalManager::list()`, the only source of running
  processes today.
- **T220** for `AgentSessionManager::list()`/`AgentRoster` — a session count
  distinct from the adapter health T300/T305 would add.
- **T325** for `OwnershipManager::list()`/`OwnershipMap`, the §35 block
  already backed by a populated wire type.

## Required reading

- `docs/Gritty IDE Project Plan.md:486,661` § 14; `docs/plans/Gritty Agent
  Runtime Coordination.md:1312,1348` § 34–35; `docs/decisions/
  ADR-001-desktop-runtime.md`; `docs/architecture/security.md`
- `crates/gritty-core/src/health.rs`, `apps/desktop/src/core/health.ts` — the
  surface this task supersedes
- `crates/gritty-core/src/session/{wire.rs:69,mod.rs:367}` (`PersistenceStatus`,
  `SessionService::status`)
- `crates/gritty-core/src/ownership/{wire.rs:52,86,112,118,160,185,mod.rs:91}`
- `crates/gritty-core/src/agent_session/{wire.rs:24,220,mod.rs:96,269}`
  (`FollowMetrics` at `:96` — the shape to follow, §6)
- `crates/gritty-core/src/terminal/{wire.rs:59,89,90,mod.rs:348}`
- `packages/protocol/src/events.ts:88,90,94,106`, `client.ts:80,82,341,349`;
  `apps/desktop/src/core/client.ts` — called with no `onDiagnostic` override
- `apps/desktop/src-tauri/src/commands.rs:925` (`guard`) — the one place every
  command's outcome already crosses, logged, and discarded
- `apps/desktop/src-tauri/src/core.rs:47,150`; `crates/gritty-core/src/
  protocol/{export.rs,mod.rs:27}` (wire-type registration, MAJOR/MINOR)
- `apps/desktop/src/features/operations/operations-panel.tsx` — the nearest
  existing panel

## Allowed scope

- `crates/gritty-core/src/diagnostics/**` (new); `crates/gritty-core/src/
  {lib.rs,protocol/export.rs}` (module registration only)
- `apps/desktop/src-tauri/src/{core.rs,commands.rs,lib.rs}` — a new `Core`
  field, the new command adapter, the `generate_handler!` entry, and the
  mechanical change to every `guard()` call site (§8); no other adapter's
  *behavior* changes
- `packages/protocol/src/{commands.ts,types.ts,index.ts,events.ts,client.ts}`
  + `src/generated/**` (regenerated, never hand-edited)
- `apps/desktop/src/core/client.ts`, `apps/desktop/src/features/diagnostics/**`
  (new), `apps/desktop/src/features/shell/app-bar.tsx` (mount point only)
- `docs/architecture/security.md`, if durable enough to record; this file

## Explicit non-goals

- **No number for a subsystem that cannot compute one.** T070 and T300/T305
  do not exist. Never "watchers: 0" or "adapters: 0/connected" — a zero reads
  as "nothing is wrong," a claim about a scan this task never ran. Absent
  rows say "not yet available."
- **No per-provider adapter health.** §35's connected/degraded needs T300's
  adapter connections — control, not observation. `AgentProvider` has one
  variant today; this shows T220's session count, labelled observed sessions.
- **No new push event per recorded error or counter tick** (hard rule 8): the
  panel pulls a snapshot when opened and on a slow interval while visible;
  nothing streams while closed. Relatedly, no editing of `EventHub`'s dispatch
  logic, which is already correct — only where its `Diagnostic` messages go.
- **No persistence, and no telemetry.** The buffer and counters live in
  memory, reset on restart, and never leave the machine — writing
  command-failure text into the session database would add the at-rest risk
  `docs/architecture/security.md` already worries about; revisit with an ADR
  if a real cross-restart need appears. No migration follows: nothing here is
  persisted, so there is no schema to change and no `SCHEMA_VERSION` to claim.

## Required behavior

### What is real today, and what is not (read before writing code)

1. Four rows need no new backend plumbing, only a read call the panel already
   has reason to make: **database status** from `SessionService::status()`;
   **running processes** from `TerminalManager::list()`, filtered to entries
   whose `pid` is `Some`; **agent sessions** from
   `AgentSessionManager::list()` → `AgentRoster.sessions.len()`, grouped by
   `AgentProvider` (one group today); and **ownership**, from
   `OwnershipManager::list()` → `OwnershipMap`.
2. For ownership, report **active leases** (`LeaseState::Active`,
   `stale == false`), **stale leases** (`stale == true`), and **waiting
   requests** (`ConflictState::Waiting` among `conflicts`). §35 lists these as
   two separate numbers ("waiting requests" and "conflicts"); this codebase
   has one concept for an open, blocking refusal, so report it once.
3. Two rows are **not yet computable** and render as absent, not zero:
   **watchers** (T070 absent) and **adapters: connected/degraded per
   provider** (T300/T305 absent). Show the §1 session count instead, labelled
   as observed sessions.
4. §35's "Events: received / normalized / rejected" describes agent-provider
   event normalization (T300) — also absent, and easily confused with a block
   that **is** real today: the core→renderer event channel's own failure
   modes (listen failure, dropped envelope, malformed payload, sequence gap,
   handler throw) going to `console.warn` and nowhere else. Count that
   channel, not the agent protocol, and never label it §35's "Events" block —
   "3 core events were dropped" is not "3 agent tool calls were rejected."
5. **Recent errors** (§14) is genuinely new. Every command's result already
   crosses one place — `guard()` — which logs a failure with `tracing::warn!`
   and discards it. Add a bounded, in-memory record of the last N (suggest
   50) failures: command name, `code`, `message`, `correlationId`, elapsed
   time — readable through a new command.

### The new backend surface

6. `crates/gritty-core/src/diagnostics/` gets a small manager following the
   shape of `AgentSessionManager::FollowMetrics` — an honestly-documented
   counters struct, not a framework (hard rule 9). It holds the recent-errors
   buffer and a `record_error(command, &CoreError, elapsed)` method that
   `guard()`'s failure branch calls.
7. One new read command, `diagnostics.recentErrors`, returns the buffer,
   newest first, no request payload; its wire type is registered in
   `protocol/export.rs` beside every other root type. Protocol version bumps
   MINOR (additive). Do not build a combined `diagnostics.snapshot` re-fetching
   ownership, terminals and the agent roster — hard rule 2: those already have
   commands and events (`ownership.list`/`OwnershipChanged`,
   `agents.list`/`AgentSessionsChanged`, `terminal.list`); a second command
   for the same data is the abstraction the rule forbids.
8. `Core` gains a `diagnostics: Arc<...>` field, constructed the way `agents`
   is — eagerly, warm before anything asks. `guard` is a free function with no
   access to `Core` today, so threading a handle through it is a mechanical,
   broad change across every `#[tauri::command]` body, not a design decision.
   Do not special-case a subset of commands: half coverage would make the
   error history a claim about which commands were wired, worse than none.

### The event-channel counters

9. `EventHub` already calls `#diagnostic` for every listen failure, dropped
   payload, sequence gap and handler throw. Add a counter object incremented
   alongside those calls (received, dropped, sequenceGaps, handlerErrors),
   exposed as a synchronous getter — no new command, no core involvement,
   since this traffic is observed entirely on the WebView side. Wire
   `apps/desktop/src/core/client.ts`'s `createCoreClient(...)` to an
   `onDiagnostic` that keeps `console.warn` and also feeds these counters.

### The panel

10. `apps/desktop/src/features/diagnostics/` renders the six rows above: agent
    sessions, ownership, processes, database (§1–2), the event channel (§9),
    recent errors (§7). Absent rows (§3–4) render a stated reason ("needs
    T070", "needs T300/T305"), never a blank cell and never a zero.
11. It ships in every build, unlike `OperationsPanel`
    (`apps/desktop/src/features/operations/operations-panel.tsx`), which is
    development-only: this debugs a real user's field failure, and gating it
    behind a debug build reproduces the console-spelunking problem for
    exactly the people who cannot open one.
12. Collapsed by default, mounted from `app-bar.tsx` (T048's precedent: a
    `<details>` costing no layout until opened, in the file already holding
    the health popover this supersedes). Opening it reads `ownership.list`,
    `agents.list`, `terminal.list`, `diagnostics.recentErrors`, the local
    counters, and `session.status`; closing stops polling. Ownership and
    agent-session rows also refresh on `OwnershipChanged`/`AgentSessionsChanged`
    while open.
13. Nothing here substitutes for `GRITTY_LOG` (`lib.rs:287`) — the panel is a
    live snapshot, the log is the record. Say so in the panel's own copy.

## Security

- **Never store the raw request payload.** The buffer carries only `code`,
  `message` and `correlationId` — never the `serde_json::Value` `guard()`'s
  callers parse from, and never `details` (`error-envelope.tsx` renders it
  once for the triggering action, a different trust context than a rolling,
  passively-viewed history). Memory only, capped, reset on restart (rule 13).
- **Truncate `message`** (suggest 500 bytes), the way `note` fields already
  cap user text (`ownership/wire.rs:20`, `MAX_NOTE`); redact wherever the
  source is untrusted, per hard rule 13.

## Failure behavior

| Situation | Behavior |
|---|---|
| No workspace open | Ownership row shows "no workspace open," not zero — `ownership.list` needs a `WorkspaceId` it does not have |
| `diagnostics.recentErrors` with an empty buffer | Returns `[]`; the panel says "no errors recorded this session" |
| A command fails before `guard` is reached | Not recorded — outside this task's seam; the buffer records what crosses `guard`, not the whole IPC stack |
| The diagnostics command itself fails | Not fed back into the buffer it was reading; surfaces via the ordinary `ErrorEnvelope` |
| Ring buffer at capacity | Oldest entry drops silently — a diagnostics feature needing its own monitoring has missed the point |
| Panel closed mid-poll | The in-flight read's result is not applied to unmounted state, as every other feature hook already does |
| `session.status()` reports `available: false` | Database row shows "unavailable" with `message` if present — expected, not a failure |
| Counters run for a very long session | 64-bit, must not silently wrap |

## Acceptance criteria

- With a workspace open, an active lease, and a discovered agent session,
  opening the panel shows non-zero lease and session counts using only
  `ownership.list`/`agents.list` plus the one new `diagnostics.recentErrors`.
- A triggered command failure (e.g. `workspace.open` on a missing path)
  appears in recent-errors with its code and message, and ages out once past
  the bounded buffer.
- Publishing a malformed core event increments the event-channel counters,
  and the same message still reaches `console.warn` as it does today.
- The watcher and adapter-health rows render a stated reason, never a number,
  on a tree with no T070/T300 code present.
- `pnpm check` green; generated protocol types fresh and committed.

## Tests required

- **Unit (Rust):** recent-errors buffer — records on failure, drops oldest at
  capacity, returns newest first, empty buffer round-trips to `[]`.
- **Unit (Rust):** `guard()`'s failure branch records exactly once per failed
  command, never on success, never twice for a panicked one.
- **Unit (TS):** event-channel counters increment on a listen failure, a
  malformed payload, a sequence gap and a handler throw, and not on a
  well-formed dispatch; `console.warn` still fires alongside them.
- **Unit (TS):** the panel renders "not yet available" for watcher/adapter
  rows and a real number for the four computable ones, given a fixed fixture.
- **Integration:** opening the panel with no workspace reports "no workspace
  open" for ownership rather than erroring the whole panel.
- **Manual:** trigger a real command failure, a real ownership conflict
  (T325's fixture) and a background terminal; confirm all three appear
  without opening a browser devtools console.

## Commands to run

```
cargo test -p gritty-core
cargo clippy --workspace --all-targets -- -D warnings
pnpm check
```

## Completion report

_Filled in by the implementing session. It must state, per row in "What is
real today, and what is not," which shipped with a real number and which
shipped as "not yet available," and confirm none shipped as a zero standing
in for "not measured." It must also record the exact mechanism used to thread
the new manager through every `guard()` call site, since this spec
deliberately leaves that Rust-level plumbing to the implementer rather than
prescribing it._
