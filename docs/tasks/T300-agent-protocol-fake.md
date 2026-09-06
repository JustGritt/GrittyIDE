# Task T300 — Agent runs are Gritty's own, and a deterministic fake proves the whole lifecycle (Gate D)

- **Phase:** 4
- **Status:** superseded
- **Depends on:** T200 (workstream model, Gate C — a run attaches to a
  workstream and brings no task model of its own), T210 (island — the
  surface a run's state eventually has to reach)

## Objective

Give Gritty a domain model for agent runs it **owns**, and a fully
deterministic fake adapter that can live through an entire run — start,
work, block on a permission or a decision, resume, finish, fail, or get
cancelled — with no process, no network and no real provider. This is
Gate D: T305, T310, T320 and T335 all depend on it, and none of them can
be designed with confidence against a type that does not exist yet.

The most important thing here is not a type or an event: it is the line
between what this task builds and what already exists.
`crates/gritty-core/src/agent_session/mod.rs:9-13` says there is no
command in `agent_session` that starts, stops, answers or approves
anything, "because Gritty does not own these sessions: another terminal
does," and that a run T300 brings "get[s] their own type and their own
lifecycle." `docs/architecture/agent-sessions.md:7-10` makes the same
point in its own words: runs Gritty owns "are a different type with a
different lifecycle; nothing here may be mistaken for one."
`docs/plans/execution-board.md:144-149` turns that into a rule for this
task: Lane O (this task, T305, T310, T320) and Lane A (`agent_session`,
T223, T326) are disjoint for exactly this reason, and "if a task in this
lane finds itself editing `agent_session`, the lane boundary was wrong
and it stops rather than reaching."

## Dependencies

- **T200**, for `WorkstreamId` and the state machine an `AgentRun`
  attaches to. `docs/architecture/workstreams.md:8-9`: an agent adapter
  "translates provider behaviour into *this*; it does not bring a task
  model of its own." An `AgentRun` is not a second kind of task — hard
  rule 2 forbids a second abstraction where `Workstream` already owns
  the responsibility.
- **T210**, for the island the run's state eventually summarises into.
  This task's own UI (below) is deliberately minimal.
- **ADR-005** (`docs/decisions/ADR-005-agent-isolation.md`): "A
  deterministic fake agent is mandatory before any real provider
  (T310)," and cancellation and orphan detection are "hard requirements
  from the first implementation" — this one.

## Required reading

- `AGENTS.md` hard rules 2, 4, 6, 8, 9, 10, 13, 14, and the contract-gate
  diagram naming T300 as Gate D.
- `docs/architecture/agent-sessions.md:1-13,396-411` and
  `crates/gritty-core/src/agent_session/{mod.rs:1-26,90,wire.rs:1-13,283-292}`
  — the lane boundary, in prose and in the code this task must not
  touch, plus the `FileTouched` path rule to reuse.
- `docs/plans/execution-board.md:144-158,215-234,230` — Lane O's paths,
  reserved protocol minor, and the pre-assigned crate name.
- `docs/decisions/ADR-005-agent-isolation.md` in full;
  `ADR-018-agent-integration-bridge.md` §§5-7 (report, never decide);
  `ADR-019-workspace-ownership.md:2-8,49-52,156-186` (the
  `Decision.raisedBy`/`agentRunId` precedent and its naming collisions).
- `docs/plans/phase-0-spike.md:119-129` (§D, shell readiness) and
  `:137-139,156-170` (shutdown guarantee; the determinism lesson).
- `docs/plans/Gritty Agent Runtime Coordination.md` §§5-8 (210-411),
  §36's T300 entry (1399-1412), §40 (1635-1667).
- `crates/gritty-core/src/protocol/ids.rs:88-91`, `protocol/actor.rs`,
  `ownership/wire.rs:4-8,90-96`,
  `crates/persistence/src/migrations/0003_ownership.sql:8` and
  `crates/gritty-core/src/workstream/wire.rs:97-99` — `AgentRunId` and
  `Actor::Agent{run_id}` already on the wire, and the two seams (the
  ownership lease, `Decision.raised_by`) left for this task and T335.
- `crates/gritty-core/src/operation/{mod.rs,registry.rs,state.rs}`
  against `crates/gritty-core/src/terminal/mod.rs:1-13` — the bounded
  operation contract versus the closer precedent, a long-lived
  supervised process with its own registry.
- `crates/agents/src/transcript.rs:39-45` — the bounding discipline (not
  the numbers) to reuse for event text.

## Allowed scope

> **Split by `PLAN-FANOUT` (2026-09-06).** T300 is `superseded` on the
> board and this file is the spec for the four tickets that replaced it:
>
> - **`T300-C`** (lane `C`) — the wire only: `agent_run/wire.rs`, the
>   exporter, `packages/protocol/*`, the version bump. §§1-7.
> - **`T300-FAKE`** (lane `O2`) — `crates/agent-fake/**`. §§8-11.
> - **`T300-CORE`** (lane `O`) — `gritty-core/src/agent_run/**`: the manager,
>   supervision, cancellation, the bounded ring. §§12-15.
> - **`T300-CMD`** (lane `O3`) — `src-tauri` adapters and
>   `features/runs/**`. §16 onward.
>
> `T300-C` lands alone and is reviewed alone: the ten-state machine and the
> event vocabulary are the only genuinely risky design calls here, and
> everything behind them is mechanical. The other three start together.
> Every non-goal below binds all four — in particular, none of them edits
> `agent_session`.

- `crates/gritty-core/src/agent_run/**` (new)
- `crates/agent-fake/**` (new) — package `gritty-agent-fake`, the name
  `docs/plans/execution-board.md:230` pre-assigns to Lane O
- `crates/gritty-core/src/{protocol/{mod.rs,export.rs},event/mod.rs,lib.rs}`
  — wiring only; shared magnets (`docs/plans/execution-board.md:238-253`),
  claim and release quickly
- root `Cargo.toml` (new workspace member, one `[workspace.dependencies]`
  line for `gritty-agent-fake` with its justification)
- `apps/desktop/src-tauri/src/{commands.rs,core.rs,lib.rs}` — `runs.*`
  command adapters
- `packages/protocol/src/{client.ts,commands.ts,types.ts}` and a new
  `runs-client.test.ts`; `src/generated` is written by the exporter
- `apps/desktop/src/features/runs/**` (new)
- `docs/architecture/agent-runs.md` (new), `docs/tasks/README.md`,
  `AGENTS.md`'s contract-gate line for T300, this file

## Explicit non-goals

- **No editing `agent_session`.** Not the wire types, not the manager,
  not its tests. If correlating a run to the observed roster starts to
  look necessary, the lane boundary was drawn wrong for this task
  (`docs/plans/execution-board.md:149`) — stop, do not import across it.
- **No `AgentAdapter` trait.** One implementation exists; a trait over
  one implementation is the abstraction hard rule 9 forbids building
  ahead of a requirement — `agent-sessions.md`'s "Adding a provider"
  made this call already for the observed side. T305/T310 earn it.
- **No field on `Workstream`.** Hard rule 6 forbids provider-specific
  fields there; a `currentRunId` column is T223's agent-link mistake
  worn again — a run's relation to its workstream is a query on
  `AgentRun.workstream_id`, never a column on `Workstream`.
- **No command that answers a decision or a permission request.** A run
  can *reach* `NEEDS_DECISION`/`BLOCKED`, and the fake can be told, in a
  test, to unblock itself — but turning that into a workstream
  `Decision` the user answers is T335's job
  (`crates/gritty-core/src/workstream/wire.rs:97` reserves the seam);
  building half of it here leaves T335 to redo it.
- **No context manifest (T320), capability negotiation (T305), real
  provider or process spawning (T310).** This task's one provider,
  `FAKE`, can emit everything — exactly why it cannot stand in for
  capability-awareness.
- **No write enforcement, no ownership writes.** `AgentRunId` exists so
  `OwnershipLease.agent_run_id` has somewhere to point once a run holds
  a lease; nothing here constructs one or calls `ownership.acquire` —
  that is T405's, once a run exists to schedule.
- **No free-form metadata bag.** The coordination plan's `AgentOperation`
  (§7) carries `metadata?: Record<string, unknown>`; `AgentRunActivity`
  does not — an untyped bag from an adapter is the raw provider output
  hard rule 4 forbids treating as domain state.
- **No persistence, no migration.** See "Known limitations" — a real
  gap, named rather than hidden.

## Required behavior

### The domain model

1. New module `crates/gritty-core/src/agent_run`, adapter
   (`gritty-agent-fake`) split from domain, the way `gritty-agents`/
   `agent_session` and `gritty-terminal`/`terminal` already are —
   ADR-005's split, a third time. `AgentRun` carries `id: AgentRunId`,
   `workstream_id`, `workspace_id`, `provider: AgentRunProvider`,
   `state: AgentRunState`, an optional `activity: AgentRunActivity`,
   `started_at`, `updated_at`, an optional `finished_at`,
   `cancellation_requested: bool`, `correlation_id`, an optional
   `error: CoreError` — `Operation`'s shape, reused.
2. `AgentRunProvider` is a **new** enum, not `agent_session::AgentProvider`
   reused: reusing it would let the observed roster's provider list
   silently become a run choice too, the coupling the lane split
   forbids. One variant, `FAKE`; T310 adds a second.
3. `AgentRunState`: `STARTING, RUNNING, WAITING, NEEDS_DECISION, BLOCKED,
   REVIEW, FINISHED, FAILED, CANCELLED, INTERRUPTED` (plan §6).
   `STARTING` stays distinct from `RUNNING` because of the phase-0
   shell-readiness finding above: a future terminal-backed adapter
   (T310) waits out that window in `STARTING`; the fake, needing no
   shell, may leave it immediately, but the state exists now so T310
   does not invent it later.
4. `AgentRunActivityKind`: `THINKING, READING, EDITING, SEARCHING,
   EXECUTING, TESTING, WAITING, REQUESTING_PERMISSION,
   REQUESTING_DECISION` (plan §7), paired in `AgentRunActivity` with an
   optional `path: WorkspacePath`, an optional bounded `command` and
   `started_at` — no `metadata` bag (non-goals). State and activity stay
   separate, per plan §6: `state` is `RUNNING` while `activity` cycles
   through `EDITING`, `EXECUTING`, and so on.

### The event protocol and correlation

5. `AgentRunEvent`, a tagged enum: `RUN_STARTED, RUN_FINISHED,
   PROMPT_STARTED, PROMPT_FINISHED, FILE_READ, FILE_WRITE_REQUESTED,
   FILE_WRITTEN, COMMAND_STARTED, COMMAND_FINISHED,
   PERMISSION_REQUESTED, DECISION_REQUESTED, RUN_BLOCKED, RUN_RESUMED,
   RUN_FAILED` — plan §8, with `AgentBlocked`/`AgentResumed`/
   `SessionStarted`/`SessionStopped` renamed into the run's own
   vocabulary (say in the completion report if that survived). Every
   event carries `run_id` and a `Timestamp`; looking the run up
   supplies `workstream_id`/`workspace_id`, answering §8's correlation
   chain — "Workspace → Workstream → Agent Run" — in full. The plan
   names a fourth link, "→ Agent Session"; this task drops it, since an
   owned run has no reason to also be a roster entry.
6. A file path follows `AgentTimelineKind::FileTouched`'s rule exactly
   (ADR-010): a `WorkspacePath` inside the workspace, a display string
   outside it. Every string is bounded in the core before crossing the
   protocol, reusing the discipline (not the numbers) of
   `transcript.rs`'s constants — named, never literal, each tested.
7. Events deliver as a bounded ring per run, `agent_session`'s timeline
   shape: oldest-first eviction, `dropped` reported not hidden,
   coalesced batches, never one event per call (rule 8).
   `AgentRunChanged` carries one whole `AgentRun` on a state or activity
   change only, matching `WorkstreamChanged`; `AgentRunEventAppended`
   carries a delta, matching `AgentTimelineAppended` — a consumer that
   missed one calls `runs.events`.

### The fake adapter

8. `gritty-agent-fake` is its **own crate**, not a `testing` module in
   `gritty-core` or `crates/agents` — `crates/agents` is Lane A's, and a
   fake there is Lane O writing inside Lane A's scope. It also compiles
   as an ordinary dependency, since the manual test drives it from the
   running app, not only `cargo test`.
9. The fake is driven **synchronously by the caller**, never real
   elapsed time: an ordered script (a prompt, a read, a write, a
   command, a permission request, a decision request, a failure, a
   finish), advanced one step at a time. A test asserts the exact state
   and event sequence, twice, identical, with no `sleep` — the
   determinism lesson above, applied here. Its own step vocabulary is
   deliberately **not** identical to `AgentRunEvent`: the translation
   between them, in `agent_run`, is the seam T310's real adapter needs,
   and a fake that skips it tests nothing about the boundary that
   matters (hard rule 4).
10. The fake can be told, mid-run and out of band from the wire
    protocol, to answer a pending permission or decision and resume —
    test-only, not a `runs.*` command (answering is T335's).
11. A script that ends with no finish/fail step, or whose driving task
    is dropped, still resolves to a terminal state rather than staying
    `RUNNING` forever — ADR-005's "orphan detection," reusing the
    operation registry's own contract: a panic is contained and the
    thing it drove ends `FAILED` with an `Internal` error.

### Supervision and cancellation

12. Each `AgentRun` is supervised like a terminal session, not squeezed
    into the generic `OperationRegistry` — a ten-state machine and event
    stream do not fit `OperationStatus`'s five terminal states. Hard
    rule 10 is satisfied the same way the operation model does it: a
    `CancellationToken` per run, checked at await points.
13. `runs.cancel` is cooperative and idempotent: unknown id `NotFound`;
    a terminal run returns its snapshot unchanged, like
    `OperationRegistry::cancel`; a running or blocked run's token fires
    and it ends `CANCELLED`.
14. `MAX_CONCURRENT_RUNS` bounds active runs per workspace, the shape of
    `MAX_OPEN_TIMELINES = 8` — this task picks the number; T310 must
    justify it against real process cost.
15. Closing the run's workspace, app shutdown, or removing the
    workstream it targets all cancel the run rather than orphaning it —
    the guarantee already proved for terminals ("`Core::shutdown` kills
    every session before the session writer flushes"); the workstream
    case is an explicit reaction to `WorkstreamRemoved`, since there is
    no foreign key to cascade (see "Known limitations").

### Commands and a minimal surface

16. Five commands, `runs.*`: `startFake` (workstream id, a named
    script/preset), `cancel`, `list` (by workspace), `events` (by run,
    rehydration per the T010 convention) — nothing starts a real
    provider, which does not exist yet. `startFake` requires the target
    workstream to exist (`NotFound` otherwise) but no particular state;
    whether it is a sensible target is T405's job.
17. `apps/desktop/src/features/runs/**` stays small: a list of active
    runs and a control to start a fake one, enough for the manual test.
    A designed surface for owned runs in the island is unassigned;
    record that gap in the completion report rather than build past it
    (rule 14).
18. Protocol version bumps MINOR (additive): one namespace, five
    commands, two events, no change to an existing shape.

## Security

Every string an adapter produces is untrusted input (rule 13), the fake
included — it exists to prove the boundary works, not to skip it.

- Prompts, tool names, command lines and paths are bounded and
  whitespace-collapsed in the core, never the adapter or renderer, and
  render as text only — never Markdown, HTML, an openable path outside
  the `FileTouched` rule (§6 above), or a runnable command. Tracing
  carries a run's id, state, provider and duration, never event text.
- The fake never touches the real filesystem or a real process; its
  events are entirely scripted fiction, which is what makes the tests
  below safe anywhere, including CI, with no fixture ever mutated.

## Failure behavior

| Situation | Behavior |
|---|---|
| `runs.startFake` targets an unknown workstream, or a closed workspace | `NotFound`; no run created |
| More than `MAX_CONCURRENT_RUNS` already active | `Conflict`, naming the count |
| `runs.cancel` on an unknown run | `NotFound` |
| `runs.cancel` on a run already terminal | idempotent no-op, current snapshot returned |
| `runs.cancel` on a running or blocked run | token fires; run ends `CANCELLED` |
| Fake script ends with no finish/fail step | run ends `FAILED`, `Internal`, naming the gap |
| Fake's driving task panics or is dropped | run ends `FAILED`, `Internal`, contained like an operation body panic |
| Owning workstream removed, its workspace closed, or app shutdown, mid-run | run is cancelled, not orphaned |
| An event names a `run_id` matching no known run | dropped, logged once, never forwarded |
| Duplicate or out-of-order fake steps | rejected by the state machine and logged |

## Acceptance criteria

The coordination plan's own line for this task, verbatim: **a fake
adapter can deterministically simulate an agent lifecycle.**

- A scripted run passes through `STARTING → RUNNING → BLOCKED
  (permission) → RUNNING → NEEDS_DECISION → RUNNING → FINISHED`; a test
  asserts the exact state and event sequence, run twice, identical, with
  no `sleep` anywhere in the assertion path.
- A cancelled run reaches `CANCELLED` from any non-terminal state, and a
  terminal one is unaffected; a run whose script panics or is dropped
  mid-way still reaches `FAILED`, never left `RUNNING`.
- `Workstream`'s and `Decision`'s wire shapes are unchanged, verified by
  diffing the generated types; `pnpm check` green and types fresh.

## Tests required

- **Unit (Rust):** every accepted and refused `AgentRunState`
  transition; duplicate/out-of-order event handling; the bounded ring's
  drop-and-report behaviour under overflow; every string field bounded
  before a test asserts the wire shape.
- **Unit (Rust):** the fake's script covering the full state machine at
  least once — a decision, a permission, a failure, and a cancellation
  from each non-terminal state; orphan detection, where a dropped
  driving task and a script with no terminal step both resolve to
  `FAILED`, never left running.
- **Unit (Rust):** `runs.cancel` idempotency on a terminal run;
  `MAX_CONCURRENT_RUNS` refusal; workstream-removed and
  workspace-closed reactions.
- **Unit (TS):** the `runs.*` client maps every command and event; the
  reducer replaces on `AgentRunChanged`, appends on
  `AgentRunEventAppended`, and drops events for an unknown `runId`.
- **Integration:** `apps/desktop/src-tauri/tests/commands.rs` — start a
  fake run through the real command surface; assert the emitted
  `CoreEvent` sequence matches the scripted one exactly.
- **Manual:** from the Runs panel, start a fake run, watch it through a
  block-and-resume cycle to finish; start a second and cancel it
  mid-run; close the workspace with a run active and confirm it
  disappears rather than lingering.

## Commands to run

```
cargo test -p gritty-core         # also regenerates packages/protocol/src/generated
cargo test -p gritty-agent-fake
cargo clippy --workspace --all-targets -- -D warnings
pnpm check
pnpm dev                          # the manual pass above
```

## Completion report

_(filled in by the implementing session)_ — three obligations unique to
this task: name the `AgentRunEvent` variants actually shipped and say
whether the renaming away from coordination plan §8 proposed above
survived, and why; state plainly, under "Known limitations," that an
`AgentRun` does not survive a Gritty restart, and say whether that is
acceptable for a fake alone or already a gap worth an ADR before T310
gives a real agent something worth losing; and record, as a follow-up
rather than as scope quietly taken, the first surface a real run should
get in the island once T310 exists.

### T300-C — the run contract, alone (Gate D)

Branch `t300-c-agent-run-contract`. Sections **1-7 only**; §§8-11 are
T300-FAKE's, §§12-15 T300-CORE's, §16 onward T300-CMD's.

**Summary.** `crates/gritty-core/src/agent_run/` now exists and contains the
whole of what Gate D freezes and nothing more: the domain model, the ten-state
machine with its transition table, the fourteen-variant normalized event
vocabulary, the two `CoreEvent` families that deliver it, and the bounding that
makes rule 13 true for all of it. There is no manager, no adapter, no command
and no UI — deliberately, so that the two genuinely risky design calls (the
state machine and the event vocabulary) are reviewable on their own while
everything mechanical waits behind them.

**Files changed**

| File | What |
|---|---|
| `crates/gritty-core/src/agent_run/mod.rs` (new) | the module, and the `agent_session` boundary written down where it will be read |
| `crates/gritty-core/src/agent_run/state.rs` (new) | `AgentRunState`, the transition table, 9 tests |
| `crates/gritty-core/src/agent_run/wire.rs` (new) | every wire type, the bounds and `clip`, 17 tests |
| `crates/gritty-core/src/{lib,event/mod,protocol/mod,protocol/export,session/tap}.rs` | wiring, `PROTOCOL_VERSION` 2.13.0, the exporter, the tap's exhaustive match |
| `packages/protocol/src/types.ts`, `src/generated/**` | 12 re-exports; 10 new generated files plus `CoreEvent`/`version` |
| `packages/protocol/src/runs-events.test.ts` (new) | 3 tests over the two event families |
| `docs/architecture/agent-runs.md` (new) | the contract, and what it deliberately does not decide |

**Tests actually executed.** `cargo test -p gritty-core` — 345 passed, 0
failed (26 of them new). `pnpm check:rust` — every crate green on the second
run; the first hit the known `gritty-terminal` flake
`pty.rs::kill_terminates_the_whole_process_tree`, which passed alone
immediately after (16/16). `pnpm lint`, `pnpm typecheck` — clean.
`pnpm vitest run packages/protocol` — 18 files, all green including the new
one. Two pre-existing TS failures are **not** this branch's and were confirmed
so by stashing every change and reproducing them on `e8ada81`:
`packages/editor/src/monaco/model-eol.dom.test.ts` (a 10 s `beforeAll` timeout
importing Monaco, identical on the clean base) and
`packages/markdown/src/render.test.ts` (a 2000 ms wall-clock budget, measured
2037 / 2552 ms under five concurrent agent builds and passing when run alone).
Neither package is touched here.

**The `AgentRunEvent` variants shipped, and the renaming.** Fourteen:
`RUN_STARTED`, `RUN_FINISHED`, `PROMPT_STARTED`, `PROMPT_FINISHED`,
`FILE_READ`, `FILE_WRITE_REQUESTED`, `FILE_WRITTEN`, `COMMAND_STARTED`,
`COMMAND_FINISHED`, `PERMISSION_REQUESTED`, `DECISION_REQUESTED`,
`RUN_BLOCKED`, `RUN_RESUMED`, `RUN_FAILED`.

**The renaming survived, and it should.** Plan §8's `SessionStarted`,
`SessionStopped`, `AgentBlocked`, `AgentResumed` and `AgentFailed` became
`RUN_STARTED`, `RUN_FINISHED`, `RUN_BLOCKED`, `RUN_RESUMED` and `RUN_FAILED`.
The word *session* is already spoken for by `agent_session`, where it means the
opposite thing — an agent Gritty did **not** start — and a protocol in which
`SessionStarted` sometimes means "Gritty started it" and sometimes "something
else did" is exactly the confusion the Lane A / Lane O split exists to prevent.
The one substantive change beyond renaming is that `RUN_FINISHED` is the
success ending only: cancellation is reported by the run's *state*, not by a
fifteenth variant, because a cancelled run's ending is a fact about the run and
not something the adapter reported.

**Architectural decisions**

1. **`AgentRunPath` is one type, not a repeated pair.** §4 named
   `path: Option<WorkspacePath>` and §6 separately required
   `FileTouched`'s rule (ADR-010). Five places need that rule, so it is one
   struct — `{ path?: WorkspacePath, display: string }` — rather than a
   two-field convention re-derived per call site. `path` present ⇔ inside the
   workspace ⇔ openable by the renderer; `display` is always safe to show and
   never to resolve.
2. **Bounding is one call, `AgentRunEvent::bounded()`.** Named limits,
   character-counted, idempotent, and tested by walking the serialized JSON of
   every variant. Writing it as a method rather than as advice means
   T300-CORE has exactly one seam to apply it at, and the test fails if a new
   variant is added without a bounding arm.
3. **A `CoreError`'s `message` is bounded too.** The first draft exempted it
   as "core-built"; the vocabulary-wide test immediately proved that wrong,
   since a failure message is routinely built *from* what an adapter said.
   `details` is left alone — it is structured context the core builds itself.
4. **Every non-terminal state reaches `FAILED`, `CANCELLED` and
   `INTERRUPTED`.** That is what makes §11's orphan guarantee and the
   "cancellable from any non-terminal state" criterion unconditional rather
   than true-for-most-states. Progress transitions are otherwise narrow: a
   stopped run resumes to `RUNNING` before it can stop on anything else.
5. **Terminal states are dead ends**, unlike `WorkstreamState`'s, which each
   keep one way back. The thing that was executing is gone; a second attempt is
   a second run.
6. **No `allowedTransitions` on `AgentRun`.** `Workstream` carries one because
   a user drives those transitions; a run's are driven by its adapter, and
   offering them to a UI would invite a command that must not exist.
7. **`exitCode` is renamed by hand.** `rename_all` on a tagged enum renames
   variants, not the fields inside them — the same trap `AgentTimelineKind`
   already worked around. Caught by inspecting the generated TypeScript, and
   now covered by a test that rejects any `snake_case` key in the vocabulary.

**Known limitations**

- **An `AgentRun` does not survive a Gritty restart**, and `session/tap.rs`
  persists neither of its events. For a fake this is not merely acceptable, it
  is *correct*: a row claiming on startup that a run is `RUNNING` when the
  process driving it died with the app is precisely the orphan the state
  machine exists to prevent, and failing closed is the right direction. It
  becomes a real gap the moment T310 gives a run something worth losing — a
  real provider's process, its transcript, half an hour of work — and ADR-021
  (already on the board, Wave 4, "due before T310") is where that is decided.
  This ticket does not pre-empt it; it names it.
- **The contract ships without its commands.** `runs.*` (§16) is T300-CMD's,
  so protocol 2.13.0 carries types and events only. The command surface the
  next contract ticket needs is listed under follow-ups.
- **`MAX_RUN_EVENTS` is stated, not enforced.** The constant is here because
  `dropped` counts against it and that is a promise the protocol makes; the
  ring that honours it is T300-CORE's.
- **The fake's step vocabulary does not exist yet**, so the translation seam
  §9 requires is specified (in `mod.rs` and `agent-runs.md`) but unbuilt.

**Follow-ups** (recorded, not taken)

- **The first surface a real run should get in the island.** T210's island
  reports workstreams and T231's rail reports observed sessions; an owned run
  is a third thing and has no designed home. The recommendation is *not* a
  fourth surface: a run is attached to a workstream, so its state and activity
  belong on that workstream's island row — a second line reading `RUNNING /
  Editing src/auth/session.ts`, with `needs_attention()` feeding the same
  "needs you" count the island already keeps. That keeps `agents-panel.tsx`'s
  rule intact (one roster, one count) and gives T410 one queue to aggregate
  rather than two. Worth a ticket of its own once T310 exists; T300-CMD's panel
  is explicitly the throwaway version.
- **The `runs.*` command surface for the next contract ticket**, verbatim from
  §16 so it need not be re-derived: `runs.startFake` (workspaceId,
  workstreamId, script preset) → `AgentRun`; `runs.cancel` (runId) →
  `AgentRun`; `runs.list` (workspaceId) → `AgentRun[]`; `runs.events` (runId) →
  `AgentRunEventLog`. `AgentRunEventLog` is already generated and exported, so
  only the three request types and the client methods remain.
- **`AGENTS.md`'s contract-gate diagram still reads "T300 Agent protocol +
  fake agent (Gate D)".** T300 is superseded by four tickets. Left alone
  deliberately: `AGENTS.md` is read by every concurrent session and editing it
  mid-wave is disruptive. Worth one line when the wave closes.
- **A `RUN_INTERRUPTED`-shaped question for T310.** `INTERRUPTED` is reachable
  and meaningful, but no event announces it — the state change does. If a real
  provider needs to say *why* it was interrupted, that is a variant, and it is
  cheaper to add now than after a client depends on the count.

**Risks**

- The event vocabulary is frozen by this gate and three tickets start against
  it the day it lands. A variant added later is additive and cheap; a variant
  whose *shape* is wrong is not. The one most likely to move is
  `PERMISSION_REQUESTED`, which reports `{ tool, detail }` today and will meet
  T335's richer `AgentPermissionRequest` when runs learn to be answered.
- `MAX_RUN_EVENTS = 512` and the seven string limits are this ticket's
  judgement, unmeasured. They are named constants with tests, so moving them is
  a one-line change, but T310 should check them against a real provider's
  output rather than inherit them.

### T300-CORE — the run manager: supervision, cancellation, the ring

Branch `t300-core-run-manager`. Sections **12-15 only**; §§1-7 were T300-C's and
are unchanged here, §§8-11 are T300-FAKE's, §16 onward T300-CMD's.

**Summary.** `crates/gritty-core/src/agent_run/` grew the half T300-C
deliberately left unbuilt: a registry that owns every run in the process, a
`CancellationToken` per run, the bounded ring that `MAX_RUN_EVENTS` was until
now only a promise about, and the three reactions that stop a run outliving
what it was working on. Nothing on the wire changed — no command, no event, no
exported type, no `PROTOCOL_VERSION`, no migration — which is the whole point
of landing it behind a contract that was frozen first.

Supervision is a terminal session's shape, not `OperationRegistry`'s: a
ten-state machine with an event stream beside it does not fit
`OperationStatus`'s five terminal states, and squeezing it in would mean either
flattening the machine this gate exists to freeze or widening `OperationStatus`
for one caller. What *is* reused, verbatim, is the operation model's discipline
— a child token per run under a root token, the driver running as its own task
so a panic is contained, and every path out of the supervising task leaving the
run terminal.

**Files changed**

| File | What |
|---|---|
| `crates/gritty-core/src/agent_run/manager.rs` (new) | `AgentRunManager`, `AgentRunContext`, `AgentRunSpec`, `MAX_CONCURRENT_RUNS`, `FINISHED_RETENTION` |
| `crates/gritty-core/src/agent_run/ring.rs` (new) | the per-run bounded ring and its coalesced batch, 7 tests |
| `crates/gritty-core/src/agent_run/tests.rs` (new) | 27 supervision and cancellation tests |
| `crates/gritty-core/src/agent_run/mod.rs` | declares the two modules, re-exports the manager, records that the contract's two named gaps are closed |

**The API surface T300-CMD and T300-FAKE call.** Written out because both were
being built at the same time as this.

```rust
// crates/gritty-core/src/agent_run/manager.rs — re-exported from agent_run
pub const MAX_CONCURRENT_RUNS: usize = 4;
pub const FINISHED_RETENTION: usize = 50;

pub struct AgentRunSpec {          // not a wire type; runs.startFake builds one
    pub workspace_id: WorkspaceId,
    pub workstream_id: WorkstreamId,   // the CALLER guarantees it exists
    pub provider: AgentRunProvider,
    pub actor: Actor,
    pub correlation_id: CorrelationId,
}

impl AgentRunManager {
    pub fn new(Handle, Arc<EventBus>, Arc<WorkspaceManager>) -> Arc<Self>;

    // runs.startFake: NotFound (workspace not open), Conflict (the bound)
    pub fn start<F, Fut>(self: &Arc<Self>, AgentRunSpec, drive: F) -> Result<AgentRun, CoreError>
    where F: FnOnce(AgentRunContext) -> Fut + Send + 'static,
          Fut: Future<Output = Result<(), CoreError>> + Send + 'static;

    pub fn cancel(&self, &AgentRunId, Actor) -> Result<AgentRun, CoreError>;  // runs.cancel
    pub fn list(&self, &WorkspaceId) -> Vec<AgentRun>;                        // runs.list
    pub fn events(&self, &AgentRunId) -> Result<AgentRunEventLog, CoreError>; // runs.events
    pub fn get(&self, &AgentRunId) -> Option<AgentRun>;

    pub fn cancel_for_workstream(&self, &WorkstreamId) -> Vec<AgentRun>;   // on WorkstreamRemoved
    pub fn interrupt_for_workspace(&self, &WorkspaceId) -> Vec<AgentRun>;  // on WorkspaceClosed
    pub async fn shutdown(&self, grace: Duration);                         // on Core::shutdown
}

impl AgentRunContext {   // Clone + Send; what the adapter's future receives
    pub fn run_id(&self) -> &AgentRunId;
    pub fn snapshot(&self) -> Option<AgentRun>;
    pub fn is_cancelled(&self) -> bool;
    pub fn cancelled(&self) -> WaitForCancellationFuture<'_>;
    pub fn token(&self) -> &CancellationToken;
    pub fn record(&self, AgentRunEvent);                                  // buffered
    pub fn record_all(&self, impl IntoIterator<Item = AgentRunEvent>);    // buffered + flush
    pub fn flush(&self) -> bool;                                          // one coalesced batch
    pub fn transition(&self, AgentRunState) -> Result<AgentRun, CoreError>;
    pub fn set_activity(&self, Option<AgentRunActivity>) -> Result<AgentRun, CoreError>;
    pub fn advance(&self, AgentRunState, Option<AgentRunActivity>) -> Result<AgentRun, CoreError>;
    pub fn finish(&self, Option<String>) -> Result<AgentRun, CoreError>;   // -> FINISHED
    pub fn fail(&self, CoreError) -> Result<AgentRun, CoreError>;          // -> FAILED
}
```

Three things about it are worth saying out loud, because they are the boundary
rather than the plumbing:

- **The driver's future ends the run or the manager does.** `drive` returns
  `Ok(())` once it has called `finish` or `fail`. Returning without ending the
  run is the orphan case and the manager fails it, `Internal`, naming the gap;
  returning `Err` fails it with that error; panicking fails it with the panic
  message. There is no fourth outcome.
- **`CANCELLED` and `INTERRUPTED` are not reachable from `AgentRunContext`.** A
  run does not cancel itself, and a driver claiming to have been interrupted is
  claiming to know something only its supervisor knows. `transition` refuses
  every terminal state.
- **The adapter bounds nothing.** `record` puts every event through
  `AgentRunEvent::bounded` at the ring's edge, which is the single seam T300-C
  built it for (rule 13). An adapter that bounded its own strings would be
  trusted to have done so; it is not.

**Tests actually executed.**

- `cargo test -p gritty-core` — **417 passed, 0 failed** (34 new: 27 in
  `agent_run::tests`, 7 in `agent_run::ring`).
- `cargo clippy -p gritty-core --all-targets -- -D warnings` — clean.
- `cargo fmt --all -- --check` — clean.
- `pnpm check` — **green, exit status 0** (captured from `$?`, not from a piped
  `tail`). Biome 251 files; vitest **76 files / 593 tests passed**, both
  known-flaky suites (`MONACO-JSDOM-FLAKE`, `MARKDOWN-BUDGET-FLAKE`) included
  and passing on the first run; every Rust crate green; `check:generated`
  clean, which is the mechanical proof that this branch changed no wire type.
- `pnpm --filter @gritty/desktop sidecar` first, per `SIDECAR-FRESH-CHECKOUT`.

The failure and recovery behaviour below was **provoked, not asserted from
compilation** (rule 12). Each row is a test that forces the failure and then
watches the run reach a terminal state on the event stream:

| Forced | Test | Result |
|---|---|---|
| cancel from each of the six non-terminal states | `a_run_is_cancellable_from_every_non_terminal_state` | `CANCELLED` from all six, no `RUN_FAILED` |
| a driver parked on a future that never resolves | `a_driver_parked_forever_is_still_cancelled` | aborted, `CANCELLED` |
| a driver that returns without ending its run | `a_driver_that_returns_without_ending_its_run_fails_it` | `FAILED`, `Internal`, names the gap |
| a driver whose script is dropped | `a_run_whose_script_is_dropped_still_ends` | `FAILED` |
| a driver that panics | `a_panicking_driver_is_contained_and_fails_its_run` | `FAILED`, panic message kept, process fine, manager still usable |
| **the supervising task killed with the run live** | `shutdown_force_ends_a_run_whose_supervisor_died`, `cancelling_a_run_whose_supervisor_died_still_ends_it` | force-ended `INTERRUPTED` / `CANCELLED`, never left `RUNNING` |
| workstream removed mid-run | `removing_the_workstream_cancels_only_its_own_runs` | `CANCELLED`; another workstream's run untouched |
| workspace closed mid-run | `closing_the_workspace_interrupts_its_runs` | `INTERRUPTED` |
| shutdown with a live and a blocked run | `shutdown_leaves_no_run_running` | both `INTERRUPTED` before `shutdown` returns |

The acceptance criterion itself — `STARTING → RUNNING → BLOCKED (permission) →
RUNNING → NEEDS_DECISION → RUNNING → FINISHED` with the exact state *and* event
sequence, run twice, identical — is `the_scripted_lifecycle_is_deterministic`.
There is no `sleep` anywhere in the assertion path: each step is acknowledged
before the next is sent, and an ending is awaited on the event stream rather
than on the clock.

**Architectural decisions**

1. **A stop's ending depends on who asked.** §15 says all three cases are
   "cancelled, not orphaned", but `AgentRunState::Interrupted`'s own contract
   names two of them literally — "the app is shutting down, the workspace
   closed" — and distinguishes itself from `Cancelled` because "nobody asked".
   So: `runs.cancel` and a removed workstream end `CANCELLED` (a person asked
   for the work to stop), a closed workspace and shutdown end `INTERRUPTED`
   (the ground went out from under the run). Both satisfy §15's actual
   requirement, which is that the run is terminal; the state now also says
   *which* happened, which is worth more to the user than one word for three
   different events.
2. **The manager holds `WorkspaceManager` but no database.** A run against a
   workspace that is not open has nowhere to happen, and that check is cheap
   and synchronous, so "a closed workspace is `NotFound`" is a core invariant
   here rather than an adapter convention. Workstream *existence* needs the
   session database, which is `WorkstreamManager`'s and asynchronous — so
   `runs.startFake`'s "unknown workstream is `NotFound`" is **T300-CMD's
   precondition**, stated on `AgentRunSpec::workstream_id` where its caller
   will read it.
3. **`MAX_CONCURRENT_RUNS = 4`, per workspace, refusing rather than queueing.**
   The shape of `MAX_OPEN_TIMELINES`, and deliberately smaller than it: a run
   is an agent working inside the user's workspace, and this project puts
   inspectability above orchestration power. Four is already more than one
   person can watch, and T410 exists because even four can need attention at
   once. The fake costs nothing, so the bound constrains nothing today — which
   is exactly why it is set now rather than after a real provider makes the
   answer expensive. **T310 must justify it against real process cost**
   (memory, token spend, how many worktrees T400 can hold) rather than inherit
   it.
4. **Cancellation is cooperative *and* enforced.** The token fires for a driver
   that watches it; the supervising task aborts one that does not. That is what
   makes "cancellable from any non-terminal state" unconditional rather than
   true-for-well-behaved-adapters — a run `BLOCKED` on a permission it will
   never get is parked on an await, and only the abort reaches it.
5. **Once a stop is requested, the ending is the manager's.** A driver mid-step
   when the cancel lands gets `Conflict` from `transition` and its `finish` is
   redirected to the chosen ending — the operation registry's rule ("once
   cancellation was requested, the only terminal state is `CANCELLED`"),
   because the alternative is a run that un-cancels itself in the window before
   its task is aborted.
6. **`RUN_FAILED` is recorded by whoever decided on the failure.** The first
   draft had `fail()` record it and the orphan path not, so a consumer reading
   only the event stream saw an orphaned run's log stop mid-sentence. It now
   happens in the one place a run becomes `FAILED`, so a driver's own failure,
   an orphan and a panic all close their log the same way. A cancellation
   records nothing — that stays what T300-C decided, a fact about the run
   rather than something the adapter reported.
7. **`dropped` is cumulative on both sides**, unlike the agent timeline's,
   whose batch carries a delta. That is what
   `AgentRunEventAppendedPayload::dropped`'s contract says ("a client compares
   it with what it last saw"), so a batch is worth emitting when it has records
   *or* when the total moved — a burst that evicted everything undelivered
   still reports that it did.
8. **Batches coalesce at a step boundary, not on a timer.** A state or activity
   change flushes on its own, `record_all` flushes, and the pending list
   auto-flushes at `MAX_RUN_EVENTS` so a driver that never flushes still cannot
   make it unbounded. A wall-clock flusher would have bought timeliness for a
   silent stream at the cost of the determinism T300-FAKE's tests are built on;
   see follow-ups.
9. **Emissions are collected under the registry lock and emitted after it is
   released.** `EventBus::emit` assigns sequence numbers under its own lock and
   calls sinks under it, so a sink that re-entered this manager would deadlock
   — the same re-entrancy `TerminalManager` documents for its PTY host.

**Known limitations**

- **An `AgentRun` still does not survive a Gritty restart**, and this ticket
  does not change that — the manager is entirely in memory, `session/tap.rs`
  persists neither run event, and there is no migration. For a fake that is
  correct rather than merely acceptable, for exactly the reason T300-C gave: a
  row claiming on startup that a run is `RUNNING` when the process driving it
  died with the app is the orphan this whole state machine exists to prevent,
  and failing closed is the right direction. It becomes a real gap the moment
  T310 gives a run something worth losing, and **ADR-021** is where that is
  decided. What this ticket adds to that question is one datum: the in-process
  guarantee is now strong enough that persistence would only ever need to
  reconcile a *process death*, never a live orphan.
- **`FINISHED_RETENTION = 50` is the whole of a session's run history.** Beyond
  it the oldest ended run is pruned entirely, ring and all, and `runs.events`
  on it becomes `NotFound`. Same shape as the operation registry's 200; smaller
  because a run carries up to `MAX_RUN_EVENTS` records beside it.
- **No timer-driven flush.** A driver that records events for a long time
  without changing state or calling `flush` leaves them undelivered until it
  does (they are in the ring, so `runs.events` still answers). The fake drives
  step by step, so this is invisible today; a real provider streaming tokens
  might notice.
- **The reactions are explicit calls, not a subscription.** `EventBus` is
  emit-only — there is no way for the core to observe its own events, and a
  sink that tried would deadlock — so `cancel_for_workstream` and
  `interrupt_for_workspace` must be wired by the composition root next to
  `workstreams.remove` and `workspace.close`. That wiring is **T300-CMD's**;
  until it exists the guarantees are proved but not connected.

**Follow-ups** (recorded, not taken)

- **Wire the three reactions in the composition root** (T300-CMD, or a
  one-line ticket after it): `cancel_for_workstream` after `workstreams.remove`
  succeeds, `interrupt_for_workspace` after `workspace.close`, and
  `AgentRunManager::shutdown` in `Core::shutdown` **before** the session writer
  flushes, where the terminal manager's already is.
- **A coalescing flush interval**, if a real provider (T310) turns out to
  stream many events between state changes. It belongs behind an explicit
  `start_flusher(interval)` the app calls and tests do not, so determinism is
  not traded away for timeliness by default.
- **`MAX_CONCURRENT_RUNS` against real cost** — T310's, as above.
- **The island surface for an owned run** is unchanged from T300-C's
  recommendation and still unassigned: a second line on the workstream's own
  island row (`RUNNING / Editing src/auth/session.ts`), with
  `needs_attention()` feeding the count the island already keeps, rather than a
  fourth surface.

**Risks**

- **The context is the seam two concurrent tickets meet at.** `AgentRunContext`
  was designed while `gritty-agent-fake` and the `runs.*` adapters were being
  written against a description of it rather than against it. The shapes most
  likely to need a nudge are `record_all` (whether an adapter wants to hand
  over a step's events at once or one at a time) and `advance` (which refuses a
  no-op state change, so an adapter whose activity changed while the state held
  still must use `set_activity`). Both are additive changes to one file.
- **`abandon_supervisor` is `#[cfg(test)]` and exists to be nasty.** It kills a
  run's supervising task with the run still live, which is the only way to
  reproduce a dead supervisor from outside. If a future refactor makes it
  inconvenient, the instinct to delete it should be resisted: it is the test
  that would catch a stop path quietly assuming someone is still watching.
- **The one thing not proved here is the app.** Every guarantee above is a
  `cargo test`; the manual pass in "Tests required" needs a Runs panel and
  `runs.*` commands, which are T300-CMD's. Until that lands, "close the
  workspace with a run active and confirm it disappears" is proved in the core
  and unproved in the product.
