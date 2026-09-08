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
