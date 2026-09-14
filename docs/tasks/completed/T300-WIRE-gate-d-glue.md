# Task T300-WIRE — Connect the three Gate D halves

- **Phase:** 4 (orchestration)
- **Status:** done
- **Depends on:** T300-C (contract), T300-CORE (manager), T300-FAKE (adapter), T300-CMD (command surface)

## Objective

Make Gate D usable. T300-CORE, T300-FAKE and T300-CMD each landed a correct
half and none could reach the others, so nothing in the running application
could start, watch or cancel a run. This task is the glue: a driver that
performs the translation the fake publishes but cannot implement, the run
manager built into the composition root, and the four `runs.*` command
adapters.

## Dependencies

All three siblings are on `main` (`df03bf6`). The TypeScript half was already
complete before this task started — `COMMANDS` carries the four names,
`client.ts` exposes `core.runs.*`, the generated types are current, and
`RunsPanel` is mounted in `App.tsx` — so this task adds no protocol surface and
spends no `PROTOCOL_VERSION`.

## Required reading

- `crates/gritty-core/src/agent_run/mod.rs` — the line between an owned run and
  an observed session
- `crates/agent-fake/src/lib.rs` — the mapping table, published as strings
- `crates/gritty-core/src/agent_run/manager.rs` — `start`, `AgentRunContext`,
  `shutdown`, `cancel_for_workstream`, `interrupt_for_workspace`
- `HANDOFF.md` § "Start here: the one thing that is half-built"

## Allowed scope

- `crates/gritty-core/Cargo.toml`
- `crates/gritty-core/src/agent_run/**`
- `apps/desktop/src-tauri/src/**`
- `apps/desktop/src-tauri/tests/commands.rs`

## Explicit non-goals

- **No fifth command.** Answering a permission or a decision is out of band by
  design (§10) and `runs.*` is deliberately four commands. A run that stops on
  an answer parks; see the follow-up.
- No `AgentAdapter` trait — one implementation, and hard rule 9.
- No change to the frozen contract, the protocol version or the schema.
- No new panel. `RunsPanel` is T300-CMD's throwaway debug surface and stays that.

## Required behavior

1. `gritty-core` depends on `gritty-agent-fake`; the reverse edge stays
   forbidden.
2. A driver translates `FakeObservation` into `AgentRunEvent`, `AgentRunActivity`
   and `AgentRunState`, typed, and a test asserts it agrees with
   `FakeAction::run_events`, `FakeAction::run_activity`, `FakePhase::run_state`
   and `FakeError::core_error_code`.
3. The four `runs.*` adapters exist, are registered, and reconcile T300-CMD's
   `start_fake(request, correlation)` with T300-CORE's `start(spec, drive)` in
   the adapter rather than by changing either side.
4. `AgentRunManager` is built in `Core` and shut down before the session writer
   flushes.
5. `cancel_for_workstream` runs after `workstreams.remove`;
   `interrupt_for_workspace` runs after `workspace.close`.

## Failure behavior

- Unknown script name → `InvalidInput` on `script`; unknown or foreign
  workstream, or a workspace that is not open → `NotFound`. Neither creates a run.
- A cancelled run ends `CANCELLED` whether it is parked or working; the driver
  yields between steps so an abort always has a point to land on.
- A script that runs out with nothing terminal in it ends `FAILED` with an
  `Internal` error rather than sitting in `RUNNING` (§11).
- App shutdown ends every live run `INTERRUPTED`.
- Every string the adapter reports is bounded by the core, and a path that is
  not inside the workspace is never openable.

## Acceptance criteria

- A run started from `RunsPanel` walks its states and ends, visibly, in the
  running app.
- A parked run is ended by `runs.cancel`.
- Removing a workstream cancels its runs; closing a workspace interrupts them.
- `pnpm check` green.

## Tests required

- Unit: the four agreement tests, the path boundary, the error codes.
- Integration through the real IPC: start/finish/replay, the three refusals,
  cancel, and both reaction hooks.
- Manual: the panel, in `pnpm dev`. **Not run by this task** — see below.

## Commands to run

```
cargo test -p gritty-core --lib agent_run::fake_driver
cargo test -p gritty-desktop --test commands
pnpm check
```

## Completion report

### Summary

Gate D is wired end to end. `gritty-core` now depends on `gritty-agent-fake`;
`agent_run::fake_driver` performs the translation the fake publishes as strings
and cannot implement itself; `Core` owns an `AgentRunManager`; and the four
`runs.*` adapters are registered, so `RunsPanel` — which was already mounted and
already had a typed client — reaches a real supervisor.

The two API shapes were reconciled in the adapter, as instructed. The two checks
`AgentRunSpec` documents as the adapter's are made there: the script name
resolves, and the workstream exists in that workspace (one `workstreams.list`
answers both, since it refuses a workspace that is not open).

### Files changed

| File | Why |
|---|---|
| `crates/gritty-core/Cargo.toml` | the missing dependency edge |
| `crates/gritty-core/src/agent_run/fake_driver.rs` | new: the driver and the translation |
| `crates/gritty-core/src/agent_run/fake_driver/tests.rs` | new: 20 tests |
| `crates/gritty-core/src/agent_run/mod.rs` | module declaration and re-exports |
| `apps/desktop/src-tauri/src/core.rs` | build the manager; shut runs down before the session flush |
| `apps/desktop/src-tauri/src/commands.rs` | four adapters; the two reaction hooks |
| `apps/desktop/src-tauri/src/lib.rs` | four `generate_handler!` entries |
| `apps/desktop/src-tauri/tests/commands.rs` | 5 integration tests through the real IPC |

### Tests actually executed

- `cargo test -p gritty-core --lib agent_run::fake_driver` — 20 passed
- `cargo test -p gritty-desktop --test commands` — 40 passed (5 new)
- `pnpm check` — **exit 0** (read from `$?`, not a piped tail): 873 Rust tests,
  640 vitest across 78 files

**The manual click-through was not run, and this task does not claim it.** This
session has no screen access, so nobody has watched `RunsPanel` drive a run in
`pnpm dev`. What *was* executed is the next thing down: the five integration
tests invoke the real adapters through the real IPC — `get_ipc_response` with
the webview's own URL and invoke key, on Tauri's mock runtime — so the command
names, the ACL, the request shapes, the event channel and both reaction hooks
are all exercised by machine. The gap that leaves is the WebView itself: React
state, rendering, and whether the panel is usable. One session of
`docs/testing/phase-0-manual-pass.md` shape closes it, and it is the one thing
between this ticket and `done`.

### Architectural decisions

**The translation is typed here and asserted against the fake's strings.** The
fake publishes the mapping as `&'static str` because it cannot name a
`AgentRunEvent`. Copying those strings would have made the driver a string
matcher; instead it matches on `FakeAction` — so a variant added on this side is
a compile error — and `events_agree_with_the_fake`, `activities_agree_with_the_fake`
and `run_states_agree_with_the_fake` assert the two lanes still say the same
thing, so a variant added on the fake's side is a test failure.

**Pacing is a parameter, not a constant.** The fake has no clock and
`advance_all` would finish a run before the panel painted. `FakeDriverOptions`
carries the pace: `None` for tests (fast and deterministic), `DEMO_STEP_DELAY`
for the adapter. The delay doubles as the await point a cancellation lands on;
with `None` the loop still yields, because a driver with no await point can only
be abandoned, not stopped.

**The default preset is `reviewed`, not `fullLifecycle`.** The acceptance script
stops on a permission that nothing in this protocol can answer, so as the
default it would leave every first click parked and looking broken. `reviewed`
walks `STARTING → RUNNING → WAITING → RUNNING → REVIEW → FINISHED` and ends.
`fullLifecycle` is still reachable by name, and parking is then something the
caller asked for.

**A parked run waits rather than spins.** `FakeAdvance::Blocked` consumes
nothing, so looping on it would burn a core. The driver holds the state and
awaits cancellation.

**Activity timestamps are wall-clock.** The fake's logical clock starts at
1 January 2026 so its observations stay byte-identical; that is determinism, not
a fact about when anything happened. The manager already stamps ring events with
`Timestamp::now()`, and taking the activity's time from the fiction would have
shown an activity nine months older than the events beside it.

### A defect this found

`~/elsewhere` **parses** as a valid `WorkspacePath` — `~` is an ordinary
filename character on POSIX — so the first version of the driver marked a
home-relative path as *inside* the workspace and handed the renderer something
it would offer to open. `WorkspacePath` is right to accept it; the adapter
boundary is where the convention belongs, and `workspace_path` now treats a
leading `~` as outside. Caught by `nothing_hostile_survives_the_boundary`, which
is exactly what `FakeScriptPreset::Hostile` exists for, and pinned separately by
`only_a_path_actually_inside_the_workspace_is_openable`.

Not a traversal: it would have resolved to a literal `~` directory under the
root and failed to open. Still wrong, and still the class of thing rule 13 is
about.

### Known limitations

- **A run that stops on a permission or a decision can only be cancelled.**
  Deliberate, and the reason is in the non-goals. `T300-ANSWER` is filed.
- `COMMAND_FINISHED` carries an empty `command`: the fake's `commandDone` does
  not repeat it, and the protocol's variant requires one. The ring pairs them —
  the `COMMAND_STARTED` before it in the same run — but a client reading one
  record alone cannot. Worth a contract look when T310 has a real provider that
  does carry it.
- Runs are not persisted, so they do not survive a restart. ADR-021 decides
  whether they should.
- The reaction hooks are in the command adapters, so a workspace closed by any
  other path would not fire them. There is no such path today.

### Follow-up tasks

| Ticket | Why |
|---|---|
| `T300-ANSWER` | answering a permission or decision on a run Gritty owns; belongs beside T335's queue, which already solved it for observed agents |
| `T300-PANEL` | the run belongs on its workstream's island row, not in a fourth roster (T300-CMD's own note) |

### Remaining risks

Everything proved here was proved against the fake, which by construction
touches no process, no network and no filesystem. That is what makes it safe to
run in CI and it is also the limit of the claim: nothing here says anything
about a real provider, and T310 inherits an entirely untested spawn path.

The second gap is the WebView, named under "Tests actually executed": the
adapters are exercised by machine, the panel is not exercised at all.
