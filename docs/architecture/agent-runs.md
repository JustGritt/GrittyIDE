# Agent runs (T300, Gate D)

How Gritty models an agent it **owns**: one it started, can cancel, and is
answerable for.

This document describes the **contract** that `T300-C` landed —
`crates/gritty-core/src/agent_run/`, protocol 2.15.0. The supervisor
(`T300-CORE`), the deterministic fake (`T300-FAKE`) and the `runs.*` commands
and panel (`T300-CMD`) are built against it and are not described here except
where the contract constrains them.

## The line, first

`docs/architecture/agent-sessions.md` describes the other half of this, and the
two halves must not be confused:

| | `agent_session` (T220) | `agent_run` (T300) |
|---|---|---|
| Who started it | some other terminal | Gritty |
| What Gritty can do | read evidence; carry one permission answer back | start, cancel, and account for it |
| Where its state comes from | inference from process, transcript and hook | transitions the core accepted |
| Lifetime | as long as the other process lives | as long as Gritty supervises it |
| Identity | a provider's own session id | `AgentRunId`, minted here |

They are different modules with different lifetimes and **they never import
each other**. If correlating a run to the observed roster starts to look
necessary, the boundary was drawn wrong; the move is to stop and say so, not to
reach across (`docs/plans/execution-board.md`).

The naming follows from that. The coordination plan §8 proposed
`SessionStarted`, `SessionStopped`, `AgentBlocked`, `AgentResumed` and
`AgentFailed`; this protocol ships them as `RUN_STARTED`, `RUN_FINISHED`,
`RUN_BLOCKED`, `RUN_RESUMED` and `RUN_FAILED`. The word *session* is spoken for,
and a protocol in which `SessionStarted` sometimes means "Gritty started it" and
sometimes means "something else did" is the confusion the lane split exists to
prevent.

## What a run is

```
AgentRun {
  id, workstreamId, workspaceId, provider,
  state, activity?,
  startedAt, updatedAt, finishedAt?,
  cancellationRequested, correlationId, error?
}
```

`Operation`'s shape, reused. A run is *not* squeezed into the operation
registry — a ten-state machine and an event stream do not fit
`OperationStatus`'s five terminal states — but the fields a supervised thing
needs are the same ones, and answering "what is it, can I cancel it, how did it
end" the same way everywhere is worth more than a bespoke shape.

Three deliberate absences:

- **No field on `Workstream`.** A run's relation to its workstream is
  `AgentRun.workstreamId` and nothing else. A `currentRunId` column would be a
  provider concept on a durable type (hard rule 6) and a value that can go
  stale; "which run is on this workstream" is a query.
- **No `metadata` bag.** The plan's `AgentOperation` carries
  `metadata?: Record<string, unknown>`; `AgentRunActivity` does not. An untyped
  map from an adapter is raw provider output, and hard rule 4 forbids treating
  that as domain state. Whatever a provider wants to say is translated into a
  field with a meaning, or it is not said.
- **No `AgentAdapter` trait.** One implementation will exist. A trait over one
  implementation is the abstraction hard rule 9 forbids building ahead of a
  requirement; T305 and T310 earn it.

`AgentRunProvider` is a **new** enum with one variant, `FAKE` — not
`agent_session::AgentProvider` reused. Reusing it would silently make every
agent CLI the roster can *discover* into something Gritty offers to *start*,
which is the coupling the lane split forbids. T310 adds the second variant.

## State, and activity beside it

```
STARTING → RUNNING → { WAITING, NEEDS_DECISION, BLOCKED, REVIEW } → FINISHED
                                                    ↘ FAILED / CANCELLED / INTERRUPTED
```

Ten states, one explicit transition table (`agent_run/state.rs`), and everything
not in it is a `Conflict` that changes nothing. Two rules generate the whole
table:

1. **Every non-terminal state reaches `FAILED`, `CANCELLED` and
   `INTERRUPTED`.** Cancellation has to land from wherever the run happens to be
   (ADR-005), and a supervisor that loses what it was driving has to be able to
   say so from any state — otherwise an orphan sits in `RUNNING` forever, which
   is the failure this machine exists to prevent.
2. **Progress is otherwise narrow.** A stopped run (`BLOCKED`,
   `NEEDS_DECISION`) resumes to `RUNNING` and nowhere else: one stop at a time.
   `STARTING` reaches only `RUNNING`, because nothing else can have happened
   yet.

Three of the states earn their place explicitly:

- **`STARTING` is distinct from `RUNNING`** even though the fake leaves it
  immediately. Phase 0 found that a terminal-backed agent is not usable the
  moment its process exists (`docs/plans/phase-0-spike.md` §D); T310 waits that
  window out *here* rather than claiming to be running. The state exists now so
  T310 does not invent it after the gate froze the shape.
- **`INTERRUPTED` is not `FAILED` and not `CANCELLED`.** "The agent broke",
  "we stopped being able to watch it" and "someone asked it to stop" are three
  different answers, and a user deciding whether to retry needs to know which.
- **A run's terminal states are genuinely terminal**, unlike a workstream's:
  there is no way back, because the thing that was executing is gone. A second
  attempt is a second run.

**State and activity stay separate** (plan §6). `state` holds still at `RUNNING`
while `activity` cycles through `EDITING`, `EXECUTING`, `TESTING` and the rest,
which is what lets a UI write two lines:

```
RUNNING
Editing src/auth/session.ts
```

## The event protocol

`AgentRunEvent` is a tagged union of fourteen variants — `RUN_STARTED`,
`RUN_FINISHED`, `PROMPT_STARTED`, `PROMPT_FINISHED`, `FILE_READ`,
`FILE_WRITE_REQUESTED`, `FILE_WRITTEN`, `COMMAND_STARTED`, `COMMAND_FINISHED`,
`PERMISSION_REQUESTED`, `DECISION_REQUESTED`, `RUN_BLOCKED`, `RUN_RESUMED`,
`RUN_FAILED` — tagged the way `Actor` is, so a client that does not know a
variant can still show it as text instead of failing to parse the batch it
arrived in.

`FILE_WRITE_REQUESTED` is its own variant rather than a flag on `FILE_WRITTEN`
because the interception point T330 needs is exactly there: between wanting to
write and having written.

**Correlation.** Each event is delivered inside an `AgentRunEventRecord`
carrying `seq`, `runId` and `at`. Looking the run up supplies `workstreamId` and
`workspaceId`, so the plan's chain — *Workspace → Workstream → Agent Run* — is
answered in full without repeating three ids on every event. The plan names a
fourth link, "→ Agent Session"; this protocol drops it, because a run Gritty
owns has no reason to also be a row in the observed roster.

**Delivery** (rule 8) mirrors the agent timeline exactly:

| | carries | emitted on |
|---|---|---|
| `AgentRunChanged` | the whole `AgentRun` | a state or activity change only |
| `AgentRunEventAppended` | a delta: a coalesced batch of records | new events, never one per event |

A run is small, so a consumer that missed a `AgentRunChanged` converges on the
next one. An event stream is not, so it travels as a delta and a consumer that
missed one calls `runs.events` to resynchronise. Events live in a bounded ring
per run — `MAX_RUN_EVENTS`, oldest-first eviction — and `dropped` reports the
loss rather than hiding it, so a UI draws a gap instead of splicing unrelated
entries together.

## Untrusted by construction

Everything an adapter says is untrusted input (rule 13), **the fake included**:
it exists to prove the boundary works, not to skip it.

- Prompts, replies, tool names, command lines, questions, block reasons and
  failure messages are bounded and whitespace-collapsed **in the core**, by
  `AgentRunEvent::bounded` and `AgentRunActivity::bounded` — once, at the ingest
  seam. Never in the adapter, which is the untrusted side; never in the
  renderer, which would be trusting it. Every limit is a named constant and
  every one is tested.
- A failure message is bounded even though it arrives inside a `CoreError`,
  because it is routinely built *from* what an adapter said.
- **Paths follow `AgentTimelineKind::FileTouched`'s rule exactly** (ADR-010),
  as one reusable type: `AgentRunPath { path?: WorkspacePath, display: string }`.
  `path` is present only when the target is inside the workspace — its presence
  is precisely what the renderer may open, and its absence is what stops
  anything outside the root being opened from the WebView. `display` is always
  safe to show and never to resolve.
- Tracing carries a run's id, state, provider and duration. Never event text.

## What the contract deliberately does not decide

- **Persistence.** An `AgentRun` does not survive a Gritty restart, and the
  session tap explicitly persists neither of its events. That is right for a
  fake — a row claiming on startup that a run is `RUNNING` when the process
  driving it died with the app is exactly the orphan the state machine exists to
  prevent — and ADR-021 decides it properly before T310 gives a real provider
  something worth losing.
- **Answering.** A run may *reach* `BLOCKED` or `NEEDS_DECISION`, and
  `PERMISSION_REQUESTED` / `DECISION_REQUESTED` report the ask. Nothing here
  turns one into something the user can answer: `Decision.raisedBy` is already
  an `Actor`, so T335 costs no shape change when it does.
- **Ownership.** `OwnershipLease.agentRunId` has had somewhere to point since
  T325. Nothing here constructs a lease or calls `ownership.acquire`; that is
  T405's, once there is a run to schedule.
- **Capabilities.** The one provider, `FAKE`, can emit everything — which is
  exactly why it cannot stand in for capability-awareness (T305).

## The command surface (T300-CMD, protocol 2.16.0)

Four commands, one namespace, added as a MINOR: the contract above already
carried the types and both events, so nothing existing changed shape.

| Command | Request | Answers |
|---|---|---|
| `runs.startFake` | `StartFakeRunRequest { workspaceId, workstreamId, script? }` | `AgentRun` |
| `runs.cancel` | `CancelRunRequest { runId }` | `AgentRun` |
| `runs.list` | `ListRunsRequest { workspaceId }` | `AgentRun[]` |
| `runs.events` | `RunEventsRequest { runId }` | `AgentRunEventLog` |

They live in `crates/gritty-core/src/agent_run/request.rs` rather than in
`wire.rs`, which is the frozen contract: a request belongs beside the thing
that serves it, the split `operation::registry` and `operation::demo` already
make.

Three things this surface deliberately does not have:

- **No `runs.start`.** `AgentRunProvider` has one variant, so `startFake` is
  the only way to start a run at all and a run cannot reach a real process by
  construction. T310 adds the second variant and the command that uses it.
- **No command that answers a run.** A run reaches `BLOCKED` or
  `NEEDS_DECISION` and says so; replying to that is T335's, and half of it
  built here would be redone there.
- **No script body on the wire.** `script` is a *name*, bounded at
  `MAX_SCRIPT_NAME` and resolved by the adapter. Letting a caller post the
  steps would make the renderer the author of what an "agent" does, and an
  unknown name is `NotFound` rather than a silently different run.

`runs.list` is scoped to a workspace because that is the boundary a run is
cancelled at: closing a workspace ends its runs, so a list that crossed
workspaces would be listing work that is already over. `runs.events` is the
T010 rehydration convention — a renderer that reloaded asks for the state it
missed rather than replaying events it never received — and its `dropped`
count is what lets a client tell a real gap from a delivery failure.

### The panel is the throwaway version

`apps/desktop/src/features/runs/` is a DEV-only collapsed row beside the
operations panel: start a scripted run, watch its state and activity, cancel
it, read its events. It exists to make the manual test possible.

The surface an owned run actually deserves is **a second line on its
workstream's island row** — `RUNNING · Editing src/auth/session.ts` — feeding
the same "needs you" count the island already keeps. A run is attached to a
workstream, so a fourth roster beside the island, the agent rail and the
attention queue would break the one-roster-one-count rule those three already
follow, and would give T410 two queues to aggregate instead of one. That is a
designed change with no ticket yet; it is recorded rather than taken.
