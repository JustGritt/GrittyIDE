# Task T310 — The first real adapter: a run Gritty owns, and can speak to

- **Phase:** 4 (orchestration)
- **Status:** todo
- **Depends on:** T300-WIRE (Gate D wired end to end). **Not T305** — see below.

## Objective

Give `AgentRunProvider` its second variant, so a run Gritty owns drives a real
Claude Code process instead of a scripted fiction — and add the one thing the
run contract has never had: **a way to say something to a run that is already
running**.

The gap this closes is visible in the product today. The agent console (T233-C)
shows a session in full and offers no way to answer it, because everything it
shows is an *observed* session — one another terminal owns, which Gritty may
read and must not touch. That is correct and stays correct. A prompt box belongs
to the other lane, where Gritty started the process and is answerable for it,
and this task is what puts a process behind that box.

## Dependencies

`T300-WIRE` must be on `main`: it is what makes `AgentRunManager` reachable from
a command at all. Everything else this task needs already exists — the ten-state
machine, the bounded ring, `AgentRunEventRecord` delivery, cancellation, and the
`runs.*` namespace.

**T305 (capabilities) is deliberately not a dependency**, although the board
records one. T305 asks "what may this provider be asked to do", and the answer
for this cut is fixed at launch by `--tools` and `--permission-mode` rather than
negotiated: one provider, one declared tool set, no capability model. Building
T305 first would be an abstraction over a single implementation, which is hard
rule 9. If this task finds itself wanting to *ask* a provider what it supports,
that is the signal to stop and take T305 — not to invent one here.

## The channel, verified

The facts below were executed against `claude` 2.1.263 on darwin 25.6 on
2026-09-07, not read from documentation. The task is built on them, so a builder
who finds one of them false should stop rather than work around it.

| Verified | How |
|---|---|
| One process accepts **many** turns on stdin | `-p --input-format stream-json --output-format stream-json --verbose`; two user messages written as JSON lines, and the second turn correctly recalled the first — one conversation, not two `-p` shots |
| Gritty may **mint the session id** | `--session-id <uuid>`; the transcript appeared under that exact name |
| The prompt is **acknowledged** | `--replay-user-messages` echoes the user message back on stdout |
| A permission gate **exists** | `--permission-prompts none` → `touch /tmp/x` did not run, a `system` message with `subtype: permission_denied` arrived, and the tool result read "requires approval, and this session has no approval surface" |
| **Gritty can answer permission prompts in band** | `--permission-prompt-tool stdio` → a `control_request` with `subtype: can_use_tool` arrived carrying the tool name and its input; a `control_response` of `{"behavior":"allow","updatedInput":…}` was honoured and the command ran |
| The `stdio` flag **alone** arms it | isolated in a second run: no `initialize` control request was sent, and the `can_use_tool` request arrived anyway. `--permission-prompts host` is the default, so the flag is the only addition |
| Not every tool call is gated | an `echo` in the working directory ran with no `can_use_tool` at all. **A gate that stays quiet is not a gate that failed** — only a call the CLI judges to need approval is offered to the host, and a spike that probes with a safe command learns nothing |
| Gritty's own child is **indistinguishable from a session** | `ps` shows `claude -p …`, tty `??`, arg0 basename `claude` — which is exactly what `crates/agents/src/probe.rs:251` classifies as an observed session |

## Required reading

- `docs/architecture/agent-runs.md` — the contract, and the line it draws
- the agent-sessions architecture doc, §"The line, first" — the other side of it
- `crates/gritty-core/src/agent_run/manager.rs` — `start`, `AgentRunSpec`, `AgentRunContext`, cancellation
- `crates/gritty-core/src/agent_run/fake_driver.rs` — the translation this one mirrors
- `crates/agent-fake/src/lib.rs` §"This crate does not depend on `gritty-core`"
- `crates/agents/src/probe.rs` — how a process becomes an observed session
- `docs/tasks/T300-WIRE-gate-d-glue.md` — its non-goals are this task's inheritance
- ADR-020 (permission verdicts), ADR-010 (paths that may be opened)

## Allowed scope

- `crates/agent-claude/**` (new)
- `crates/gritty-core/src/agent_run/**`
- `crates/gritty-core/Cargo.toml`, root `Cargo.toml`
- `crates/agents/src/probe.rs` — **only** to accept an exclusion set (§8)
- `apps/desktop/src-tauri/src/**`
- `apps/desktop/src/features/runs/**`, and the console's prompt box
- `packages/protocol/src/**` (generated types are regenerated, never hand-edited)

## Explicit non-goals

- **No `AgentAdapter` trait.** Two implementations is still not a reason; the
  fake and this one are driven by the same `FnOnce(AgentRunContext)` seam.
- **No streaming tokens.** `--include-partial-messages` is not passed. A partial
  message is exactly the unbounded high-frequency forwarding rule 8 forbids, and
  the ring would evict a turn mid-sentence.
- **No `--dangerously-skip-permissions`, ever**, and no flag that reaches it.
- **No persistence.** A run still dies with the app (ADR-021 is unwritten).
- **No repointing the transcript follower at an owned run** (§6).
- **No taking over an observed session.** `--resume` would let a watched session
  become an owned run; it is the obvious next move and it is not this task's.
  File it, do not build it.
- **No second provider.** Codex is T222-CODEX, in the other lane.

## Required behavior

### 1. The adapter crate

`crates/agent-claude` (`gritty-agent-claude`), built the way `gritty-agent-fake`
and `gritty-agents` are: it **must not depend on `gritty-core`**, and it returns
its own observation type. It owns exactly the process: building the argv,
spawning, framing JSON lines in and out, and the child's lifetime. It decides
nothing about what an observation *means*.

The launch is fixed and explicit, never assembled from caller input:

```
claude -p --input-format stream-json --output-format stream-json --verbose
       --session-id <uuid Gritty minted> --replay-user-messages
       --permission-prompt-tool stdio
```

Every flag above is load-bearing and each is verified in the table: drop
`--replay-user-messages` and the prompt box loses its acknowledgement, drop
`--permission-prompt-tool` and the run is ungated. `--permission-mode` was
`manual` in both verified runs and its contribution was **not** isolated; a
builder should establish what it does before relying on it or removing it.

### 2. The translation

`agent_run/claude_driver.rs`, mirroring `fake_driver.rs`: `ClaudeObservation` →
`AgentRunEvent`, `AgentRunActivity` and `AgentRunState`, matched on typed
variants so a variant added on either side is a compile error or a test failure,
never a silently dropped event.

An unrecognised `type` on the wire is **recorded as unknown and skipped**, not
an error: the CLI is a moving target and a new message kind must not fail a run.

### 3. `STARTING` finally means something

The run leaves `STARTING` when the `system` message with `subtype: init`
arrives, **not** when the process spawns. This is the case the state was
introduced for (`docs/plans/phase-0-spike.md` §D) and the first chance to honour
it. A child that never emits `init` within a bounded wait fails the run with an
error that says so, rather than sitting in `STARTING`.

### 4. The inbox

`AgentRunManager::start` takes `FnOnce(AgentRunContext) -> Fut` and there is no
way to reach a driver once it is running. That is the one genuinely new piece of
core: a per-run channel, so `runs.prompt` hands text to a live driver.

Constraints: the frozen contract does not change; the fake keeps working with no
edit to `agent-fake`; a prompt to a terminal run is `Conflict`, not a panic; and
the queue is **bounded** — a caller that outruns the agent is refused, never
allowed to grow memory without limit.

### 5. Two commands

Protocol **2.18.0** (MINOR: both are additive, and the renderer already tolerates
values it does not know — see `run-status.ts`'s widened `LOOKUP`).

| Command | Request | Answers |
|---|---|---|
| `runs.start` | `StartRunRequest { workspaceId, workstreamId, provider, prompt? }` | `AgentRun` |
| `runs.prompt` | `PromptRunRequest { runId, text }` | `AgentRun` |

`runs.startFake` stays exactly as it is. It is the deterministic path every
other test depends on, and folding it into `runs.start` would put a fiction and
a real process behind one name.

`text` is bounded at the boundary like every other string on this surface, and
a `runs.prompt` that arrives while the run is not accepting one is `Conflict`
naming the state — never silently dropped.

### 6. The console's feed comes from the ring, not the transcript

An owned run is rendered from `runs.events` plus `AgentRunEventAppended` — the
same rehydrate-then-follow shape the timeline uses, against a different source.

The child **does** write an ordinary Claude Code transcript, so repointing
T230's follower at it would give a richer feed for free. That is rejected here,
for three reasons, and the rejection is the decision most worth revisiting if
the coarse feed disappoints:

1. it would be the first place the two lanes touch, and `agent-runs.md` is
   explicit that they never import each other;
2. the feed would come from a file rather than from the process Gritty is
   supervising, so the console and the run's own state could disagree — the
   inspectability failure the run machine exists to prevent;
3. the frozen contract already carries a conversation. `PROMPT_STARTED { text }`
   is what the user said and `PROMPT_FINISHED { text? }` is what the agent
   replied; the file and command variants carry the work between them.

What is lost is thinking blocks and per-tool detail. If that turns out to
matter, it is an ADR (next free: **ADR-027**), not a quiet re-wiring.

### 7. Permissions: the plumbing here, the policy in T310-ANSWER

`--permission-prompt-tool stdio` is passed, and it is verified to work: a
`control_request` with `subtype: can_use_tool` arrives on stdout carrying the
tool and its input, and the tool does not run before a `control_response` goes
back. That is the same stdio channel the prompt travels on, so it costs no
second transport.

**What was not tested is what happens if nobody answers.** Both verified runs
answered immediately, so "the run holds indefinitely" is an inference, not an
observation. Establish it — an ask left outstanding must never resolve to
`allow`, and if the CLI has its own timeout, its default is what actually
governs an unattended run.

This task builds the plumbing and stops at the policy:

- a `can_use_tool` request is recorded as `PERMISSION_REQUESTED` and moves the
  run to `BLOCKED`, so the ask is visible and the state is honest;
- the answer goes through **one seam** — a policy the driver consults — and the
  policy this task ships is **deny, with a reason**, which is the only default
  that is safe before anyone can answer;
- a denial is not a failure. The run continues; the agent is told no and carries
  on, exactly as it does in a terminal.

`T310-ANSWER` replaces that policy with the user's own verdict, through the
vocabulary that already exists (ADR-020) and the queue T335 built. It is a
swap at a seam this task defines, not a rework — which is why the seam is a
required deliverable and not an implementation detail.

**Never `allow` by default, and never on a timeout.** An unanswered ask is a
denial; the opposite is a tool running because nobody was looking.

### 8. Gritty must not report its own child as someone else's session

`provider_of_command` classifies any executable named `claude`, with no tty
filter, so without this the same agent appears twice: once as an owned run,
once as an observed session — the roster merge in `roster-model.ts` would draw
both.

The fix keeps the lanes disjoint: **the scan takes an exclusion set as an
input**. Gritty minted every session id it owns, so it knows exactly which to
exclude; the composition root passes them. Neither module imports the other, and
`crates/agents` learns nothing about runs.

### 9. The prompt box

In the console, below the feed. A textarea, ⌘↵ to send, disabled with a reason
when the run is not accepting input, and **nothing optimistic**: the user's text
appears when `PROMPT_STARTED` arrives, not on the keystroke — which is the rule
`use-agent-console.ts` already holds for the timeline, and is why
`--replay-user-messages` is passed.

An observed session's console gets **no** prompt box and no disabled one either.
There is nothing to enable, and a greyed-out control implies a permission the
user could acquire.

## Failure behavior

- `claude` not on `PATH`, or not executable → the run never leaves `STARTING`
  and ends `FAILED` with an error naming the binary. Not `INTERRUPTED`: nothing
  was ever being supervised.
- Child exits non-zero, or its stdout closes mid-turn → `FAILED`, carrying the
  exit status and a bounded tail of stderr.
- Child killed, or Gritty loses the pipe while the process lives →
  `INTERRUPTED`, which is the state that exists for exactly this.
- `runs.cancel` → the child **and its process group** die, verified by
  force-kill test, and the run ends `CANCELLED`. An orphaned `claude` is the
  worst outcome available here: it would keep running, keep costing money, and
  then appear in the observed roster as a session nobody started.
- App shutdown → every live run `INTERRUPTED`, every child dead.
- A malformed or oversized JSON line is dropped with a counter, never parsed
  into a truncated event and never fatal.
- Everything the child says is untrusted (rule 13): bounded and
  whitespace-collapsed **in the core**, at the ingest seam — never in the
  adapter, which is the untrusted side, and never in the renderer, which would
  be trusting it. A path is openable only when it is inside the workspace
  (ADR-010, `AgentRunPath`).

## Acceptance criteria

- A run started from the UI reaches `RUNNING` only after `init`, answers a
  prompt, accepts a **second** prompt into the same process, and ends.
- The reply appears in the console; the user's own text appears only once the
  run confirms it.
- `runs.cancel` leaves no `claude` process behind (`ps` proves it).
- The same agent appears **once** in the sidebar, not twice.
- A tool needing permission moves the run to `BLOCKED`, is refused by the
  default policy, and the console shows what was asked and why it was denied.
- **The tool did not run while the answer was outstanding** — asserted against
  an observable side effect, not against the absence of a log line.
- No renderer-blocking work, no unbounded forwarding, no new warnings.
- `pnpm check` green.

## Tests required

- **Unit**: the translation, both directions of agreement with the adapter's
  variants; the bounds on every string; the path boundary; unknown message kinds.
- **Adapter, no network**: drive the framing against a **recorded** stdout
  fixture (`fixtures/`), so CI needs neither `claude` nor an API key. This is
  the fake's determinism argument applied one layer out.
- **Process**: spawn a stub child that behaves like the CLI — slow start, dies
  mid-turn, ignores SIGTERM — and assert `FAILED`/`INTERRUPTED`/`CANCELLED` land
  correctly and no orphan survives. Rule 12: force-kill, or do not claim it.
- **Integration through the real IPC**: `runs.start`, `runs.prompt`, the refusals.
- **Manual**, in `pnpm dev` — two turns, then cancel. This one needs a real
  `claude` and is the task's own manual check; say plainly if it was not run.

## Commands to run

```
cargo test -p gritty-agent-claude
cargo test -p gritty-core --lib agent_run
cargo test -p gritty-desktop --test commands
cargo test -p gritty-core            # regenerates packages/protocol/src/generated
pnpm check
```

## Recommended split

This is larger than one reviewable unit, and the board has split this shape
twice already (T300 → C/FAKE/CORE/CMD, T233 → M/S/C). The natural cut:

| | Scope | Provable by |
|---|---|---|
| **T310-A** | the adapter crate and the translation | recorded fixture; no wire change |
| **T310-B** | `AgentRunProvider::CLAUDE_CODE`, the inbox, `runs.start`, `runs.prompt`, §8 | integration tests |
| **T310-U** | the prompt box and the run feed in the console | jsdom render tests |

This file stays the spec for all three, as T300's does for its four.

## Follow-ups this task files rather than builds

- **T310-ANSWER** — the verdict behind §7's seam. The channel is verified and
  built here; what is missing is the user's answer reaching it. The immediate
  next ticket: without it an owned run can converse but not act.
- **T310-RESUME** — take over an observed session as an owned run via
  `--resume`/`--fork-session`. Needs a rule for a session still live elsewhere.
- **ADR-027** — only if §6's coarse feed proves insufficient.
- **ADR-021** — persistence for a run, already filed, now with something to lose.

## Completion report

_To be filled by the implementing session: summary, files changed, tests
actually executed with results, architectural decisions, known limitations,
follow-up tasks, remaining risks._
