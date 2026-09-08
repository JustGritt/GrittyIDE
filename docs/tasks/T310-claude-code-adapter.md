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
- `crates/agents/src/probe.rs` and `crates/agents/src/lib.rs` — **only** to
  accept an exclusion set (§8). Widened from `probe.rs` alone on 2026-09-08,
  with the user's agreement, because a process-only filter does not satisfy §8:
  the child writes an ordinary transcript, `transcript::scan` picks up any
  `.jsonl` inside its 24-hour window keyed on the uuid Gritty minted, and
  `rosterRows` concatenates the two sources with no dedupe
  (`roster-model.ts:109`). Filtering only `discover_processes` therefore leaves
  the same agent drawn twice as a `TRANSCRIPT_ONLY` session, for a day after the
  run ends — the acceptance criterion passing its unit test while being false in
  the app. `transcript.rs` itself stays untouched: the filter is one parameter
  on `discover` and one `.filter` on its transcript results, which is the
  smallest widening that makes §8 true.
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

## Completion report — T310-A

_The adapter crate and the translation. T310-B (`AgentRunProvider::CLAUDE_CODE`,
the inbox, `runs.start`/`runs.prompt`, §8) and T310-U (the prompt box) are not
in this cut and nothing here anticipates them beyond publishing the mapping they
bind to._

### Summary

`crates/agent-claude` (`gritty-agent-claude`) exists and does exactly what §1
describes: it builds the argv, spawns the child, frames JSON lines in and out,
and owns the child's lifetime. It decides nothing about what an observation
*means* — it emits `ClaudeObservation` and publishes the intended mapping into
`AgentRunEvent`/`AgentRunActivityKind`/`AgentRunState` as **strings**, exactly
as `gritty-agent-fake` does, because it cannot depend on `gritty-core` without a
Cargo cycle. `agent_run/claude_driver.rs` is therefore still T310-B's file, and
this crate is what it will bind to.

The crate is split so the **translation is provable without a process**:
`stream::ClaudeStream` takes `&[u8]` and returns observations with no I/O, no
clock and no child, and `process::ClaudeProcess` is the only thing that spawns.
`tests/fixture.rs` drives the pure half against a recording of a real `claude`
2.1.263 session, so CI needs neither the binary nor an API key.

**No wire change, no command, no event, no `PROTOCOL_VERSION` claimed.**

### Facts established against the real binary

The task asked for two open questions to be settled rather than assumed. Both
were, against `claude` 2.1.263 on darwin 25.6 on 2026-09-08.

1. **`--permission-mode` contributes nothing** (§1 left this explicitly open).
   The same prompt was run twice with only that flag differing. With and without
   `--permission-mode manual`, `system`/`init` reported `permissionMode:
   "default"`, the `can_use_tool` request arrived identically, and the tool did
   not run. It is **not passed**: a flag that changes nothing is one a later
   reader would assume is load-bearing. `--tools` is omitted for the same reason
   one level up — restricting the tool set is T305's question, not an argv
   builder's.

2. **An unanswered permission ask holds indefinitely** (§7: "What was not tested
   is what happens if nobody answers"). A `can_use_tool` was deliberately left
   unanswered and the process watched for **145 seconds**. It emitted nothing
   further, stayed alive, and the command never ran. There is no CLI-side
   timeout, and in particular none that resolves an ask to `allow`. So §7's safe
   default is available: hold, stay `BLOCKED`, and let a policy answer. This
   crate contains **no timeout that could resolve an ask at all**.

Two further facts the recording settled, neither of which was in the channel
table: `system`/`init` arrives **once per turn**, not once per process (so a
second one is an ordinary `WAITING → RUNNING` boundary, not a duplicate to
suppress); and the CLI **echoes the `control_response` back on stdout**, which
is what lets `PermissionAnswered` be observed rather than assumed.

### Files changed

| File | What |
|---|---|
| `crates/agent-claude/Cargo.toml` | new crate; no `gritty-core` dependency, by construction |
| `crates/agent-claude/src/lib.rs` | crate root, the two-halves table, the no-cycle argument |
| `crates/agent-claude/src/launch.rs` | the fixed argv, uuid validation, the `--permission-mode` finding |
| `crates/agent-claude/src/message.rs` | private wire shapes; `Value`-first dispatch so an unknown `type` is skipped, not fatal |
| `crates/agent-claude/src/observation.rs` | `ClaudeObservation` and the published mapping (`run_events`/`run_activity`/`run_state`) |
| `crates/agent-claude/src/stream.rs` (+`stream/tests.rs`) | framing and translation; pure |
| `crates/agent-claude/src/request.rs` | the stdin frames, and `ClaudeVerdict` — §7's policy seam |
| `crates/agent-claude/src/process.rs` | spawn, pipes, process-group termination, cancel-safe `next` |
| `crates/agent-claude/src/error.rs` | `ClaudeError` + `core_error_code()` |
| `crates/agent-claude/fixtures/two-turns-denied.jsonl` | the recording |
| `crates/agent-claude/fixtures/README.md` | its provenance, and exactly what was redacted |
| `crates/agent-claude/tests/fixture.rs` | the translation against the recording |
| `crates/agent-claude/tests/process.rs` | real children: force-kill, orphans, failures |
| `Cargo.toml` (root) | one `[workspace.dependencies]` line for the new path crate — **in the tree, not in this commit**, see below |

### Tests actually executed

All run on macOS (darwin 25.6), 2026-09-08, in this working tree.

| Command | Result |
|---|---|
| `cargo test -p gritty-agent-claude` | **ok** — 49 unit + 10 fixture + 15 process + 1 doc = **75 passed, 0 failed** |
| `cargo clippy -p gritty-agent-claude --all-targets -- -D warnings` | **ok**, no warnings |
| `cargo fmt -p gritty-agent-claude -- --check` | **ok** |
| `cargo metadata --no-deps` | **ok** (the root manifest still resolves after the shared edit) |

One test failed on its first run and found a real defect: a `control_response`
whose verdict body was an unexpected *shape* (`null`, a number) failed to parse,
which left the ask outstanding and would have parked the run in `BLOCKED`
forever after it had already been answered. `ControlVerdict::behavior` is now a
`Value` read loosely — anything that is not literally `"allow"` is a denial, the
run resumes, and the unsafe direction stays closed.

What the process tests prove, by spawning real `/bin/sh` children:

- a child that **ignores SIGTERM** and has started a `sleep` of its own is
  force-killed, and **the grandchild dies with it** — asserted with `kill(pid,0)`
  against the recorded pid, not against a log line (rule 12);
- **dropping the handle** without calling `terminate` leaves no orphan either;
- a missing binary, a non-executable binary and a missing working directory are
  three distinguishable errors, each naming what was wrong;
- a child that never says `init` gives up with `Timeout` rather than sitting in
  `STARTING`, and is killed on the way out;
- a child that dies mid-turn reports `Code(3)`, `unfinished`, a bounded stderr
  tail, and one counted malformed partial line;
- a child killed from outside reports `Signal(9)` → `INTERRUPTED`, not `FAILED`;
- garbage on stdout is counted and survivable;
- the fixed launch line reaches the child unchanged (asserted from *inside* the
  process, via its own argv).

**Not run, and deliberately so:**

- `pnpm check` **was not run whole**, and I am not claiming it green. Two other
  agents have uncommitted work in this same tree (`W-t080` in `crates/search` and
  `packages/protocol`, `W-t100u` in `apps/desktop/src/features/git`), so its
  TypeScript and workspace-wide Rust stages cover their work as well as mine, and
  `check:generated` cannot pass while anyone's generated types are uncommitted.
  It was decomposed instead: the four commands above are the parts that are
  actually about this crate, and all four are green.
- `cargo test --workspace` was not run for the same reason.
- The task's **manual check in `pnpm dev`** (two turns, then cancel) was not run:
  it needs T310-B's `runs.start`/`runs.prompt` to reach a driver at all, and
  neither exists yet. The equivalent evidence available at this cut is the
  recorded session itself, which *is* two turns in one process with a cancelled
  tool in the middle.

### Architectural decisions

- **The translation is published as strings, not performed here.** Same
  arrangement as `FakeAction::run_events`, same mechanical reason: `gritty-core`
  must depend on this crate, so the reverse edge is a cycle. T310-B writes the
  typed match in `claude_driver.rs` and the cross-crate agreement test, which can
  name both sides.
- **The pure half and the process half are separate types.** `ClaudeStream`
  takes bytes; `ClaudeProcess` owns the child and feeds it. That is what makes
  "provable by recorded fixture" true rather than aspirational.
- **No `AgentAdapter` trait** (§non-goals), and no permission-policy *trait*
  either. §7's seam is `ClaudeVerdict`, a two-variant enum the adapter
  serialises without an opinion — the same swap point T310-ANSWER needs, with
  none of an abstraction's cost. `ClaudeVerdict::Allow { updated_input }` already
  carries the shape ADR-020's "allow with edits" will want, and a test asserts
  the wire form now so that ticket finds it proven.
- **Resource bounds here; meaning bounds in the core.** Nothing in this crate
  clips or collapses a string, because `AgentRunEvent::bounded` is the one ingest
  seam (rule 13) and it is on the trusted side. What *is* bounded is what a
  hostile child could otherwise grow: `MAX_LINE_BYTES` (1 MiB, oversized lines
  dropped **whole**, never truncated into a half-event), `MAX_OUTSTANDING_TOOLS`
  (64), `MAX_KIND_BYTES` (64), `MAX_STDERR_TAIL_BYTES` (4 KiB).
- **Paths and commands are passed through raw.** Whether a path is inside the
  workspace is the core's question (ADR-010, `AgentRunPath`); an adapter that
  answered it would be choosing what the renderer may open.
- **Tool classification is by the names Claude Code ships**, and lives here
  because it is provider knowledge — the core must never learn that a tool called
  `Edit` writes a file. An unknown name, or a known name whose input is the wrong
  shape, is `Other`: never a guess, never an error.
- **`ClaudeStream` holds no copy of the run's state.** It tracks only what
  translation needs (the partial line, unclosed calls, outstanding asks). A
  second answer to "what state is this run in" is one too many.
- **Process group, three ways.** `process_group(0)` at spawn; `terminate` runs
  SIGTERM → 500 ms → SIGKILL over the group; `Drop` SIGKILLs it. An orphaned
  `claude` is the worst outcome available here, so it is guarded at every exit
  including a panicking driver.
- **`ClaudeProcess::next` is cancel-safe**, so T310-B can `select!` it against
  the per-run inbox: the only await is `AsyncReadExt::read` (documented to lose
  no data when dropped) and the partial line lives on `self`.

### Known limitations

- **Unix only, and said plainly.** `killpg` is `#[cfg(unix)]`, exactly as
  `gritty-terminal` declares `nix`. On other platforms `terminate` reaches only
  the direct child, so a grandchild would survive; `tests/process.rs` is
  `#![cfg(unix)]` and nothing here has been run on Windows (rule 11).
- **A line over 1 MiB is dropped whole.** A `Write` tool call carrying a very
  large file would lose its `FILE_WRITE_REQUESTED`. The direction is deliberate —
  a lost event, never a wrong one — but it is a real gap, and the counter is how
  it becomes visible.
- **Thinking blocks and per-tool detail are coarse.** A `thinking` block becomes
  `Thought` (activity only) and its content is discarded; §6 already predicted
  this is the thing most worth revisiting, and named ADR-027 for it.
- **`AskUserQuestion` is not mapped to `DECISION_REQUESTED`.** The protocol has
  the variant and the CLI plausibly has the tool, but it was not observed in any
  recording, and mapping it from memory would be exactly the guess this task's
  channel table exists to avoid.
- **`PermissionAnswered` depends on the CLI echoing `control_response`.** That is
  observed, not documented. If a future release stops echoing, the run would
  stay `BLOCKED` after being answered rather than resuming. Recorded as a risk
  below; the safe alternative (reporting the resume when *Gritty writes* the
  answer) is one line in T310-B if it happens.
- **The two shared manifests are in the tree but not in this commit.** Root
  `Cargo.toml` and `Cargo.lock` both carry `W-t080`'s concurrent additions
  (`gritty-search`, the `grep-*`/`ignore` block) in the *same hunk* as mine, and
  staging another agent's dependency tree under this ticket would be wrong — so
  this commit is `crates/agent-claude/**` and this task file only. Nothing here
  needs either file: the crate is a workspace member through the `crates/*` glob,
  and its own manifest names every dependency. The `[workspace.dependencies]`
  line matters only when `gritty-core` comes to depend on this crate, which is
  T310-B. Nothing in CI runs `--locked`, so the lock regenerates. Whoever lands
  this wave should commit both files once.
- The recorded fixture used `--model haiku --tools Bash` for cost and
  determinism. Neither is in the shipped launch line, and
  `fixtures/README.md` says so.

### New dependencies

**No new third-party crate.** Every dependency is already in
`[workspace.dependencies]`:

- `serde` / `serde_json` — the wire is newline-delimited JSON and tool inputs are
  arbitrary JSON objects.
- `thiserror` — how every other adapter crate here spells a refusal.
- `tracing` — dropped-line counts and a killed group, logged once by count, never
  by payload.
- `tokio` (with `process` + `io-util` added **on this crate's dependency entry**,
  not widened workspace-wide) — the child and its pipes.
- `nix`, unix-only — `killpg`, so cancellation reaches the whole group.

### Follow-up tasks (filed, not built)

- **T310-ANSWER** — the verdict behind §7's seam. The channel is verified and the
  seam (`ClaudeVerdict`) is built and tested; what is missing is the user's
  answer reaching it. Now known to be safe to wait for: an unanswered ask holds
  indefinitely and never resolves to `allow`.
- **T310-RESUME** — `--resume`/`--fork-session` to take over an observed session.
  `launch.rs` asserts neither flag is present, so this is a deliberate edit when
  it happens.
- **ADR-027** — only if §6's coarse feed disappoints. The concrete thing lost is
  named above.
- **ADR-021** — persistence for a run.
- **New:** decide whether `AskUserQuestion` should become `DECISION_REQUESTED`,
  and record a real session that raises one before mapping it.
- **New:** `crates/agents`' `provider_of_command` will classify this child as an
  observed session (§8). Not this cut's, and nothing here works around it — the
  exclusion set is T310-B's, and this crate already reports the session id back
  from `init` so it can be named.

### Remaining risks

- **The CLI is a moving target.** Everything above was measured against 2.1.263
  on one day. The mitigations are structural rather than hopeful: an unknown
  `type` is recorded and skipped with a counter, every wire field is
  `#[serde(default)]`, a known kind with an unexpected body fails only itself,
  and `ClaudeStreamStats::unknown` *rising* is how a format change announces
  itself. A release that renamed `isReplay` or `can_use_tool` would still be
  degradation rather than breakage — but it would be silent degradation, and
  nothing yet watches that counter.
- **The 145-second hold is evidence, not a guarantee.** It is much longer than
  any plausible timeout and the CLI emitted nothing at all, but "indefinitely"
  was inferred from "at least 145 seconds".
- **The fixture is one conversation.** It covers `init`, replay, thinking, text,
  a gated `Bash` call, a denial, a tool result, two `result`s and four unmodelled
  kinds — but not a file write, not a search, not an `error_during_execution`,
  and not a very long session. Those paths are unit-tested from hand-built lines,
  which is the weaker evidence and is stated as such.
- **Nothing has driven this crate end to end from the app.** It compiles, its
  tests pass, and no `AgentRunManager` has ever called it. T310-B is the first
  time the two halves meet, and that is where a mismatch in the published mapping
  would surface — which is why the mapping is published as assertable strings
  rather than described in prose.

## Completion report — T310-B

_`AgentRunProvider::CLAUDE_CODE`, the run inbox, `runs.start` / `runs.prompt`,
and §8. T310-U (the console's prompt box) is not in this cut._

### Summary

A run Gritty owns now drives a real Claude Code process and can be spoken to
while it runs. Three commits on `main`: `d179acf` (the inbox and the
translation), `5672514` (the wire, protocol 2.19.0), `4cfee82` (§8 and the DEV
panel affordance).

**The split moved.** T310-A's report says plainly that
`agent_run/claude_driver.rs` remained T310-B's file, so this ticket carried the
core-side translation as well as everything the board listed. That is recorded
here rather than silently absorbed.

### The inbox (§4)

`AgentRunManager::start` takes an already-built `FnOnce(AgentRunContext)`, so a
receiver made *inside* `start` could never reach a driver constructed *outside*
it. The driver owns its end by capturing it, exactly as `fake_driver::drive`
captures its script, and `AgentRunContext` stays `Clone` and purely outbound.

`start`'s signature is **untouched** and `start_with_inbox` sits beside it, so
`fake_driver.rs` and `crates/agent-fake` needed no edit at all — the constraint
"the fake keeps working with no edit to `agent-fake`" is met more strongly than
by editing it carefully. Putting the sender on `AgentRunSpec` was rejected: that
type is documented as *what a run needs before it exists*, and a supervision
channel is not that.

`MAX_QUEUED_PROMPTS = 4` lives in `manager.rs`, not `wire.rs`, because nothing
on the wire reports pending prompts — it is supervision policy, which is the
same line that puts `MAX_RUN_EVENTS` on the other side. `prompt()` refuses at
six named edges and does its state check and `try_send` in one critical section,
so a run cannot end between the two and leave text queued for a corpse.

### Two decisions worth arguing with

1. **`MAX_PROMPT_TEXT` (16 KiB) is not `MAX_TEXT` (2000).** `MAX_TEXT` bounds
   untrusted *adapter output* for display, where clipping costs a reader
   nothing. A prompt is what a person typed and is about to be paid for;
   truncating it silently would be worse than refusing it. So the request
   refuses, the full text reaches the child, and the recorded `PROMPT_STARTED`
   is still clipped to `MAX_TEXT` because *that* one is a preview.

2. **The permission seam is an enum, not a trait.** `PermissionPolicy::DenyAll`,
   with `decide()` `async` so `T310-ANSWER` changes one body and no call site.
   It matches the adapter's own choice for `ClaudeVerdict` and avoids the
   abstraction-over-one-implementation hard rule 9 forbids.

### A contract collision, resolved and not papered over

**T310's failure table asks for something the frozen Gate D contract forbids.**
The table says a child killed, or a pipe lost while it lives, ends the run
`INTERRUPTED`. `manager.rs` is explicit that no driver can reach that state —
*"a driver claiming to have been interrupted is claiming to know something only
the supervisor knows"* — and `settle` maps a driver error to `FAILED`.

The frozen contract wins. The run ends `FAILED` with an error naming the signal
or the lost pipe, so the information survives even though the state's
granularity does not. It is asserted, not left for a reader to notice:
`state_agrees_with_the_adapter_except_for_interrupted` requires the adapter's
`INTERRUPTED` to arrive as `FAILED` *and* the message to say supervision was
lost. Widening a frozen contract needs deliberate review and an ADR, which is
not this ticket's to take — filed below.

### §8, and why its scope was widened

The task file said `probe.rs` — **only**. That is not enough, and the
widening was agreed with the user before anything was written.

Gritty's own child is `claude -p …` in `ps` **and** writes an ordinary
transcript under the session id Gritty minted. `transcript::scan` returns any
`.jsonl` inside its 24-hour window, and `rosterRows` concatenates the two
sources with no dedupe (`roster-model.ts:109`). A process-only filter therefore
suppresses the process row and leaves the transcript row: the same agent still
appears, now as a `TRANSCRIPT_ONLY` session that also reads as *not running* — a
second lie on top of the first — and it lingers for a day after the run ends.
The acceptance criterion would have passed its unit test while being false in
the app.

So the exclusion lands in `gritty_agents::discover`, where both sources meet.
`transcript.rs` is untouched: one parameter and one `.filter` is the smallest
widening that makes §8 true.

`OwnedSessions` carries **two keys** because either alone has a hole. The
session id is durable and names the transcript, but `discover_processes` has to
recover it from a command line `ps` may have truncated. The pid is exact and
needs no parsing but means nothing to a transcript that outlives the process.

The seam into `agent_session` is a **closure, not a handle**. `ownership` is a
handle because a registry is shared ground both lanes legitimately use; a run
manager *is* the other lane, and `agent_run`/`agent_session` never import each
other. `scan()` reads the set inside its `spawn_blocking`, so a run started
while a tick was queued is still excluded by it.

### Tests actually executed

- `cargo test -p gritty-core --lib agent_run` — **107 passed, 0 failed** (13 new
  `claude_driver` tests, 2 new manager tests).
- `cargo test -p gritty-agents` — **110 passed, 0 failed** (2 new).
- `cargo test -p gritty-desktop --test commands` — **51 passed, 0 failed**
  (3 new).
- `pnpm check` — **green, exit 0 captured from `$?`**, including
  `check:generated`.

**Three tests were verified to have teeth by making them fail on purpose**, then
restored:

- `a_session_gritty_owns_is_excluded_from_both_sources`, with the transcript
  filter removed, reports the owned agent again as
  `evidence: TranscriptOnly, process: None` — exactly the failure described
  above.
- `the_owned_set_is_read_on_every_scan`, with the set frozen at construction,
  reports 2 sessions instead of 1. It is the assertion a captured-once
  implementation would fail and every other test in that file would pass.
- `a_real_run_starts_after_init_takes_two_turns_and_cancels_without_an_orphan`
  first failed by asserting on a state (`WAITING`) that was already true from
  turn one; it now waits on the *second* `PROMPT_FINISHED`, which is the thing
  that actually marks the second turn.

`a_real_run_…` covers T310's acceptance criteria minus the console, through the
real IPC and with no `claude`, no credentials and no cost: the run leaves
`STARTING` only after `system`/`init`, answers a prompt, takes a **second**
prompt into the same process, and `runs.cancel` reaches a background job that
sits in its own process group and ignores SIGTERM. "No orphan survives" is
asserted against a live pid, never against the absence of a log line (rule 12).

One test corrected an assertion of mine rather than the code:
`WorkspacePath::parse("")` deliberately returns the workspace root, so an empty
path is inside by definition. Asserted explicitly now so it reads as decided.

### Known limitations

- **The manual pass in `pnpm dev` was not run.** It needs the running app driven
  by a person and this session has no way to do that. Every acceptance
  criterion except "the same agent appears once in the *sidebar*" is covered by
  the automated test above; the sidebar half is covered at the roster level
  (`a_session_gritty_owns_is_excluded_from_both_sources`) but has not been seen
  on screen.
- **No run has ever driven the real `claude` binary through the manager.** The
  adapter's own `tests/process.rs` drove the real framing against a recording,
  and this ticket drove the whole stack against a stub. The two have never met a
  live agent in one process.
- **A narrow double-draw race remains.** The exclusion set is read fresh per
  scan and a session id is registered before the child is spawned, so the window
  is microseconds wide — but a `ps` that observed a child registered after the
  set was read would report it twice for one tick.
- **`MAX_CONCURRENT_RUNS = 4` was not re-justified.** Its doc comment says in
  terms that *"T310 must justify it against real process cost … rather than
  inherit it"*. This ticket inherited it. Filed.
- **The DEV panel is the throwaway surface**, not the product's. A run still
  belongs on its workstream's island row (`T300-PANEL`).

### Follow-ups this task files rather than builds

- **`T310-U`** — the console's prompt box, now with a process behind it.
- **`T310-ANSWER`** — the verdict behind `PermissionPolicy`. Until it lands an
  owned run can converse and read but is refused every gated tool.
- **`RUN-INTERRUPTED-CONTRACT`** — decide whether a driver may report
  `INTERRUPTED`, or whether the task's failure table should be corrected to say
  `FAILED`. Today the two documents disagree and the code follows the frozen
  one.
- **`RUNS-CONCURRENCY-COST`** — re-take the `MAX_CONCURRENT_RUNS` judgement now
  that a run is a real process with real token spend.

## Completion report — T310-U

_The console's prompt box, and an owned run in the console. The adapter
(T310-A), the wire and the inbox (T310-B) are already on `main`; nothing here
touches Rust, and **no protocol change is claimed** — this cut is entirely in
`apps/desktop/src`._

### Summary

Clicking a run in the sidebar's Agents section now opens its console as an
editor tab, beside the consoles for the sessions Gritty merely watches. The feed
is the event ring the core recorded (§6) and below it is the prompt box (§9):
⌘↵ sends, nothing is optimistic, and it is disabled with a reason whenever the
run is not accepting input.

Before this, `App.tsx:341` read `if (row.sessionId === null) return` — an owned
run's row was the one thing in the sidebar that could not be clicked, and
`agents-section.tsx` said so in a comment naming T310 as what would fix it.

### The scope widening, agreed before anything was written

The board recorded T310-U's scope as `features/runs/**`, `agent-console.tsx` and
`use-agent-console.ts`. **That scope cannot reach the ticket's own title.** The
console tab machinery was keyed on a session id end to end, and §8 deliberately
keeps an owned run *out* of the observed roster — so a run can never arrive as a
session, and no edit inside those three paths can put one on screen. The
widening (the tab reducer, the tab strip, the editor pane, the sidebar row,
`App.tsx`, `app.css`) was put to the user with the alternative — build the
component and render it only from the DEV panel, delivering no console — and the
answer was to widen and deliver the ticket. Recorded here the way T310-B
recorded its §8 widening rather than absorbed silently.

### The one thing that crosses the lane boundary

A **console key**: `run:<id>` or `session:<id>`. It is not new — `roster-model.ts`
has minted exactly these strings as `RosterRow.key` since T233-M — and this
ticket makes the tab reducer read them back rather than inventing a second
identity. It is a display identity and nothing else crosses on it:

- `agent_run` and `agent_session` still never import each other in the core.
- Two components, not one with a flag: `AgentConsole` reads a timeline the core
  follows, `RunConsole` reads the ring `useRuns` holds. Neither knows the other.
- `agentTabItems` takes `RosterRow[]` — the projection that already draws both
  side by side — so a tab's label and status come from the same row the sidebar
  drew, and one agent cannot end up with two names.

`agent-console-render.test.ts` asserts the agreement rather than describing it:
it runs the real `rosterRows` and requires every key it mints to parse. Breaking
`parseConsoleKey`'s `run` prefix fails that test and nothing else, which is the
drift it exists to catch.

### Three asymmetries, and why they are not abstracted away

1. **A session has a follower; a run has none.** `openTimeline`/`closeTimeline`
   open and close a per-session tail; a run's events arrive on the machine-wide
   `AgentRunEventAppended` `useRuns` already subscribes to. So opening a run
   console fetches `runs.events` once and closes nothing, and `followedSession`
   returns `null` for a run rather than a run id — asserted, because a function
   that returned one would have `closeTimeline` called with a run id.
2. **`loadedId` is a new field, and it earns itself.** "Is this console's feed
   already here" cannot be read off `timeline` for both kinds: a run's events
   live in `useRuns`, so without it a run tab re-fetched its ring on every click
   of an already-open tab. It is also what makes a *retry* work — a refused
   `runs.events` leaves it null, so clicking again asks again, which is the same
   allowance the session path gets from `timeline === null`.
3. **Only a run gets a prompt box.** §9's rule is applied to both lanes it
   names: an observed session's console has none *and no disabled one either*,
   and a scripted run gets none for the same reason — `runs.prompt` refuses a
   `FAKE` run and no waiting opens a channel that does not exist. So
   `promptGate` has three answers (`ready` / `refused` / `absent`), not two.

### The prompt gate is a mirror, and says so

`AgentRunState::accepts_input`'s doc comment already named this box as the
surface that disables against it — *"the manager refuses against this table and
the console disables its prompt box against it, so neither keeps its own copy of
the answer."* The wire carries no `acceptsInput` flag, so the renderer mirrors
the table, exactly as `runs-reducer.ts` already mirrors `MAX_RUN_EVENTS`,
`is_terminal` and `needs_attention`. The test is the total one — every one of
the ten states named, `RUNNING` and `WAITING` open and the other eight refused
with a reason — so a state added to either side fails it rather than falling
through a default nobody chose. An unrecognised state from a newer core is
**refused**: letting someone type into a run this build cannot vouch for is the
worse failure.

Two edges beyond the state table, both from `prompt()`'s own order: an
outstanding cancel shuts the box even from `RUNNING` (the more recent intent
wins), and a run that has actually ended says "it ended" rather than "it is
stopping".

### Two smaller decisions worth arguing with

1. **A command and its result are two feed rows, not one paired row.** The
   timeline pairs a tool call with its result because both carry a `toolUseId`;
   `COMMAND_STARTED`/`COMMAND_FINISHED` carry only the command text, and pairing
   on that attaches the wrong result the first time an agent runs `cargo test`
   twice. Two honest rows beat one that is sometimes a lie.
2. **The DEV panel's prompt form is deleted, not kept beside the console.** Two
   prompt surfaces with different rules about what may be sent is exactly the
   confusion a debug affordance should not add. The panel keeps its start
   buttons and its raw event list.

### Files changed

| File | What |
|---|---|
| `features/runs/run-console.tsx` | **new** — the run's console: header, feed, prompt box |
| `features/runs/run-console-model.ts` | **new** — pure: `runFeedRows`, `promptGate` |
| `features/runs/run-console-model.test.ts` | **new** — 15 tests |
| `features/runs/run-console-render.test.ts` | **new** — 15 tests, static markup |
| `features/runs/run-console.dom.test.tsx` | **new** — 9 tests, jsdom: ⌘↵, drafts, disabling |
| `features/runs/use-runs.ts` | `openEvents` reports whether the log landed |
| `features/runs/runs-panel.tsx` | the throwaway prompt form removed |
| `features/agents/agent-tabs-reducer.ts` | console keys; `loadedId`; `openedRun` |
| `features/agents/use-agent-console.ts` | opens either lane; `ConsoleRuns`; `activeTarget` |
| `features/agents/agent-console.tsx` | `agentTabItems` takes rows; §9 noted on the session console |
| `features/agents/agents-section.tsx` | every row is a button now |
| `features/editor/editor-pane.tsx` | resolves the key and draws whichever console it names |
| `features/editor/tab-strip.tsx` | `AgentTabItem.sessionId` → `.id`, an opaque key |
| `App.tsx` | `runs` into the console hook; `row.key` on select; `consoleFacts` off the rows |
| `app.css` | `.agent-prompt*`; the now-dead `.agents-section__pick--static` removed |
| `docs/architecture/agent-runs.md` | the console section: what crosses, and what does not |

### Tests actually executed

All on macOS (darwin 25.6), 2026-09-08, in this working tree.

| Command | Result |
|---|---|
| `npx vitest run --project desktop` | **ok** — 62 files, **728 passed, 0 failed** (39 new) |
| `pnpm check` | **green, exit 0 captured from `$?`** — Biome, tsc, vitest, `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`, `check:generated` |
| `pnpm --filter @gritty/desktop build` | **ok** — the renderer bundles; `dist/` is git-ignored |

**Four tests were verified to have teeth by making them fail on purpose**, then
restored:

- Widening `ACCEPTS_INPUT` to include `BLOCKED` fails three tests across the
  model, the static render and the jsdom pass — the gate, the reason line and
  the "the draft survives" case all notice.
- `runConsoleKey` minting a `session:` key fails *mints a key the two lanes
  cannot collide in* **and** *follows no session while a run console is
  opening*, which is the one that would have had `closeTimeline` called with a
  run id.
- `parseConsoleKey` renaming its `run` prefix fails *mints keys the console can
  read back* and nothing else — the drift between two modules that is otherwise
  invisible.
- Dropping the `metaKey || ctrlKey` guard fails *leaves a bare ↵ to make a
  newline*.

### Known limitations

- **The manual pass in `pnpm dev` was not run.** It needs the running app driven
  by a person and this session has no way to do that. The renderer bundles and
  every rule above is asserted, but no one has yet seen a run's console on
  screen, typed into the box, or watched `PROMPT_STARTED` arrive behind it. This
  is the same gap T310-B recorded, and it is now the whole of what stands
  between this feature and "exercised in the running app".
- **"The console shows … why it was denied" is only half true.** The feed draws
  the ask (`PERMISSION_REQUESTED`) and the block (`RUN_BLOCKED`, carrying
  `Bash needs permission: …`), then `RUN_RESUMED`. The *verdict* — that the
  default policy denied it, and with what reason — never reaches the ring:
  `ClaudeVerdict::deny_by_default`'s message goes to the model as a `tool_result`
  and nowhere else. A reader therefore cannot tell a denied ask from an allowed
  one. That is missing information on the wire, not a rendering choice, so it is
  filed rather than invented here — a renderer that printed "denied" would be
  asserting something the core did not send.
- **No files-touched list.** `AgentConsole` puts the files a session wrote above
  its feed, and a run's `FILE_WRITTEN` events carry an `AgentRunPath` with an
  openable `path`, so the same list is available. It is not built: §9 asks for
  the prompt box and §6 for the feed, and building it here would be taking
  scope. Filed.
- **No Cancel in the console.** The sidebar row has it, so the capability is not
  lost, but the surface you are looking at while a run misbehaves is not the one
  with the stop button. Filed.
- **`RunsPanel` still runs its own `useRuns`.** Two controllers, two
  subscriptions, one core — they converge through `AgentRunChanged`, which is
  how a run started in the DEV panel reaches the sidebar at all. Pre-existing,
  and it goes when the panel does (`T300-PANEL`).
- **A run tab is labelled by its short id**, because that is what the sidebar
  row calls it. The workstream title is in the console header, not on the tab.

### Follow-ups this task files rather than builds

- **`T310-VERDICT-EVENT`** — record the permission verdict in the run's ring, so
  the console can say a tool was denied and why. Needed for T310's own
  acceptance criterion, and T310-ANSWER will want it anyway.
- **`T310-U-FILES`** — the files-touched list on a run's console, from
  `FILE_WRITTEN` paths, clickable through the same `openTouchedFile` path T232
  built.
- **`T310-U-CANCEL`** — a stop control in the run console, and the confirmation
  question it needs.
- **T310-U's manual pass** — folded into the standing `MANUAL-PASS-COVERAGE`
  ticket rather than filed twice.

### Remaining risks

- **Nothing has driven this console against a live `claude`.** Every test drives
  it against hand-built rings and a stubbed controller. The mapping it renders
  is `claude_driver.rs`'s, which T310-B asserted against the adapter, so a
  mismatch would have to be in what the *ring* holds rather than in what this
  draws — but the two halves have still never met a real agent on screen.
- **The prompt gate is a mirror and mirrors drift.** It is total over the ten
  states and fails closed on an eleventh, so a *new* state cannot slip through.
  What it cannot catch is `accepts_input` changing which of the ten it admits:
  nothing fails, and the box would refuse a run the core would accept. A wire
  flag would close that, and is the obvious thing to add if it ever bites.
- **`loadedId` is a third id beside `activeId` and `pendingId`.** Three fields
  describing one tab is one more than is comfortable, and the invariant that
  keeps them consistent (a pending id is always the tab that was last opened)
  lives in the reducer rather than in the type.
