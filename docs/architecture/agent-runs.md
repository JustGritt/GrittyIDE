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
  cancellationRequested, correlationId, error?,
  pending?            // T310-ANSWER, protocol 2.27.0
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
| `AgentRunChanged` | the whole `AgentRun` | a state, activity or pending-ask change only |
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
- **Answering a decision.** A run may *reach* `NEEDS_DECISION`, and
  `DECISION_REQUESTED` reports the question. Nothing turns it into something
  the user can answer yet: `Decision.raisedBy` is already an `Actor`, so doing
  so costs no shape change. A *permission* ask is answerable since T310-ANSWER —
  see the last section.
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
- **No command that answered a run** when this was written. A permission ask
  now has one, `runs.answerPermission` (T310-ANSWER, below); a decision still
  does not.
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

`runs-panel.tsx` is a DEV-only collapsed row beside the operations panel:
start a scripted run or a real one, watch its state and activity, cancel it,
read its events. It exists to make the manual test possible. Reading a run
properly, and speaking to it, are no longer there at all — that is the console
below (T310-U), reached by clicking the run's row in the sidebar.

The surface an owned run actually deserves is **a second line on its
workstream's island row** — `RUNNING · Editing src/auth/session.ts` — feeding
the same "needs you" count the island already keeps. A run is attached to a
workstream, so a fourth roster beside the island, the agent rail and the
attention queue would break the one-roster-one-count rule those three already
follow, and would give T410 two queues to aggregate instead of one. That is a
designed change with no ticket yet; it is recorded rather than taken.

## The console, and the one thing that crosses (T310-U)

An owned run is drawn in the editor area, beside the consoles for the sessions
Gritty merely watches, and it is the only place in the app where a user speaks
*to* an agent.

Everything about the boundary above survives that, because exactly one string
crosses it: a **console key**, `run:<id>` or `session:<id>`, which
`roster-model.ts` already minted for the sidebar row and which the tab reducer
takes apart again. It is a display identity, not a domain one. Nothing merges:
`agent_run` and `agent_session` still never import each other in the core, the
two consoles are two components, and each reads only its own lane's source —
`AgentConsole` a timeline the core follows, `RunConsole` the event ring the core
recorded. The key is what lets one tab strip, one preview rule and one "at most
one feed is open" rule serve both without either lane learning the other exists.

Three asymmetries are deliberate and are the reason it is two components:

- **A session has a follower; a run does not.** `agents.openTimeline` /
  `closeTimeline` open and close a per-session tail. A run's events arrive on
  the machine-wide `AgentRunEventAppended` a client is already subscribed to, so
  opening its console fetches `runs.events` once and closes nothing.
- **The feed is the ring, never the transcript.** The child writes an ordinary
  transcript and repointing the session follower at it would give a richer feed
  for free. Rejected: the console and the run's own state could then disagree,
  which is the inspectability failure the run machine exists to prevent. What is
  lost — thinking blocks, per-tool detail — is the price, and changing it is an
  ADR rather than a quiet re-wiring.
- **Only a run gets a prompt box.** An observed session's console has none and
  no disabled one either: there is nothing to enable, and a greyed-out control
  implies a permission the user could acquire. A scripted run gets none for the
  same reason — `runs.prompt` refuses it, and no waiting opens a channel that
  does not exist.

The box disables against `AgentRunState::accepts_input` (`RUNNING` and
`WAITING`), mirroring the six edges `AgentRunManager::prompt` refuses at, and it
is **never optimistic**: what was typed reaches the feed when `PROMPT_STARTED`
arrives, which is the run confirming it, and is why the adapter passes
`--replay-user-messages`.

## The user's verdict on a permission ask (T310-ANSWER, protocol 2.27.0)

A run's child asks before a gated tool runs (`control_request`/`can_use_tool`
on `--permission-prompt-tool stdio`), and holds until it is answered — for as
long as it takes; there is no timeout on either side. Since T310-ANSWER the
answer is the user's.

```
child ──can_use_tool──▶ driver ──ask_permission──▶ manager ─┬─ AgentRun.pending + BLOCKED (one AgentRunChanged)
                                                            └─ PendingVerdict ◀─ oneshot ─┐
user ──runs.answerPermission { runId, askId, ALLOW_ONCE | DENY }──▶ manager ──────────────┘
driver ──control_response──▶ child ──echo──▶ driver ──RUN_RESUMED──▶ RUNNING
```

- **The ask is on the run**, as `pending: AgentRunPermissionAsk { askId, tool,
  verb, target, detail?, askedAt }`, for the reason `AgentSession.pending` is on
  a session: the queue has to answer it for a run nobody has open. It arrives in
  the same `AgentRunChanged` that moves the run to `BLOCKED`, and it is taken
  off the run the moment an answer is accepted, when a stop is requested, when
  the run leaves `BLOCKED`, and when it ends. `askId` is minted by the core.
- **Two verdicts.** `ALLOW_ONCE` and `DENY`, spelled as the observed side
  spells them so one card and one set of chords answer both. There is no
  `ALLOW_FOR_RUN`: a remembered grant for a run needs its own persistence and
  revocation story (`T310-GRANTS`).
- **An allowance carries no input.** The CLI runs a bare `{"behavior":"allow"}`
  as issued, and it *would* honour a modified `updatedInput` (both verified
  against 2.1.270), so Gritty never sends one: nothing it holds can change what
  runs.
- **Every other ending is a denial.** A stop or shutdown denies before the kill
  (the supervisor gives a cancelled driver one grace period to do so before
  aborting it); a second ask while one is outstanding is denied without
  disturbing the first, and its echo is not allowed to resume the run; an ask
  naming no tool is denied; a `control_request` of another subtype is refused
  with an error response.
- **Why a oneshot and not the inbox.** An ask arrives mid-turn, which is exactly
  when the driver does not read the inbox, so a verdict queued there behind a
  prompt could never be reached. And why the driver loop has an arm for it
  rather than awaiting the verdict inline: inline, the loop could see neither a
  cancellation nor the child dying.

In the renderer the ask joins T335's surfaces rather than adding one: the same
`PermissionPrompt` at the foot of the run's console, the same `permission` kind
in the attention queue (with `source: 'run'`), the same banner, header count and
status-bar count. `features/runs/run-permission.ts` is the whole of the
difference — the ask phrased for the card, and the card's answer mapped back.
The two lanes still never import each other in the core.

