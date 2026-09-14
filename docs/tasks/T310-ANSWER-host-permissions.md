# Task T310-ANSWER — The user's verdict, behind the seam T310 built

- **Phase:** 4 (orchestration)
- **Status:** review
- **Depends on:** T310-B (the run inbox and the `can_use_tool` plumbing), T335 (the permission queue), ADR-020

## Objective

An owned run can converse but not act. `T310` ships the permission channel and a
policy that always answers **deny**, because deny is the only safe default
before anyone can answer. This task replaces that policy with the user's own
verdict, so a run Gritty started can edit a file and run a command with a human
saying yes.

It is a swap at a seam that already exists, not new plumbing. If it turns into
new plumbing, the seam was drawn wrong and that is worth reporting rather than
routing around.

## What is already verified

Executed against `claude` 2.1.263 on darwin 25.6 on 2026-09-07. This task
inherits these and does not need to re-establish them:

- `--permission-prompt-tool stdio` makes the CLI ask the host. A
  `control_request` with `subtype: can_use_tool` arrives on stdout carrying
  `tool_name` and the full `input`, and the run **holds** until answered.
- `{"behavior":"allow","updatedInput":…}` inside a `control_response` of
  `subtype: success` is honoured — the command then runs.
- The flag alone is sufficient; no `initialize` handshake is required.
- **Not every call is gated.** An `echo` in the working directory ran with no
  ask at all. A quiet gate is not a broken gate, and a test that probes with a
  safe command proves nothing.

Two things are **not** established and are this task's first job:

1. the exact shape of a **denial** response, and whether it carries a message
   the agent sees;
2. whether `updatedInput` may be *modified* rather than echoed — which would be
   an editable permission prompt, and is out of scope either way (record it).

## Required reading

- `docs/tasks/completed/T310-claude-code-adapter.md` §7 — the seam this replaces
- ADR-020 — the verdict vocabulary, and why it is what it is
- `docs/tasks/completed/T335-agent-permissions.md` — the queue, and "asked in place,
  answered in a queue"
- `apps/desktop/src/features/agents/permission-model.ts`, `permission-prompt.tsx`,
  `permission-chords.ts` — what T335 built, and what this reuses rather than rebuilds
- `crates/gritty-core/src/agent_run/manager.rs` — `BLOCKED`, `RUN_RESUMED`, cancellation

## Allowed scope

- `crates/agent-claude/**`
- `crates/gritty-core/src/agent_run/**`
- `apps/desktop/src-tauri/src/**`
- `apps/desktop/src/features/agents/permission-*.{ts,tsx}`
- `apps/desktop/src/features/runs/**`
- `packages/protocol/src/**` (regenerated, never hand-edited)

## Explicit non-goals

- **No second permission UI.** T335 built the prompt and the queue for observed
  sessions; an owned run's ask joins them. Two surfaces asking the same question
  in two shapes is the failure this task exists to avoid.
- **No remembered grants, no allowlist, no "always allow".** That is policy with
  a persistence story and it needs its own ticket (`T310-GRANTS`). One ask, one
  answer, one run.
- **No `updatedInput` editing.** Record whether it is possible; do not build it.
- **No bypass, no auto-allow, no timeout that resolves to allow** — see below.

## Required behavior

1. **A `can_use_tool` request reaches the same queue as a session's.** It is
   drawn where T335 draws one and answered with the same verdicts and the same
   chords. A user should not have to learn that an ask came from a run rather
   than a session in order to answer it.

2. **`allow` and `deny` both travel back on the run's stdio**, and the run
   leaves `BLOCKED` by the transition the state machine already has
   (`RUN_RESUMED`). A denial resumes the run too — the agent is told no and
   carries on, which is what happens in a terminal.

3. **The ask names what it is for.** Tool, input, and the run's workstream, so
   an answer is not given blind. The input is untrusted and bounded in the core
   (rule 13) — it is agent-authored text arriving from a process, and it is
   rendered as text, never as markup and never as a path anything opens.

4. **One outstanding ask per run.** The state machine already says a stopped run
   resumes to `RUNNING` and nowhere else. A second `can_use_tool` before the
   first is answered is a protocol violation by the child: log it, deny it, and
   do not let it overwrite the first.

5. **Cancelling a blocked run answers the ask.** `runs.cancel` on a run waiting
   for a verdict must not leave the child holding: deny, then kill. A child that
   is never answered and never killed is the orphan the run machine exists to
   prevent.

## Failure behavior

- **An unanswered ask is a denial.** Never an allow, never on a timeout, never
  because the window closed or the user walked away. This is the single rule the
  task exists to protect: the failure mode of getting it wrong is a tool running
  because nobody was looking.
- Gritty quits with an ask outstanding → deny, then `INTERRUPTED`.
- The child dies while an ask is on screen → the ask is withdrawn from the
  queue, visibly, rather than left as a question about a run that is gone.
- A malformed `control_request` (no `request_id`, unknown subtype) is logged and
  denied, never guessed at.
- A `control_response` that cannot be written (the pipe is gone) ends the run
  `INTERRUPTED` — Gritty stopped being able to supervise it, which is precisely
  what that state means.

## Acceptance criteria

- A run started in Gritty asks to run a command, the ask appears in the queue,
  the user allows it, and **the command's side effect is observable**.
- The same flow denied: the side effect does **not** happen, and the run
  continues rather than dying.
- An ask left unanswered never resolves to allow — asserted, not assumed.
- Cancelling a blocked run leaves no `claude` process behind.
- An observed session's ask and an owned run's ask are answered the same way.
- `pnpm check` green.

## Tests required

- **Unit**: the verdict mapping both ways; the bounds on `tool_name` and input;
  the second-ask violation; the malformed request.
- **Process**, against a stub child that requests permission and reports whether
  it was allowed to proceed: allow, deny, cancel-while-blocked, die-while-asked.
  The assertion is the **side effect**, never the log.
- **Integration through the real IPC**: the ask reaches the queue, the answer
  reaches the run.
- **Manual**, in `pnpm dev`: allow a real edit, then deny one. Needs a real
  `claude`; say plainly if it was not run.

## Commands to run

```
cargo test -p gritty-agent-claude
cargo test -p gritty-core --lib agent_run
cargo test -p gritty-desktop --test commands
pnpm check
```

## Follow-ups this task files rather than builds

- **T310-GRANTS** — remembered grants for an owned run, if wanted, with the
  persistence and revocation story ADR-020's grants already imply.
- **T310-EDIT-INPUT** — editing `updatedInput` before allowing, if §"What is
  already verified" finds it possible.

## Completion report

### Summary

A run Gritty owns can now act with a human saying yes. When its `claude` child
sends `can_use_tool`, the ask is put on the run (`AgentRun.pending`, protocol
**2.27.0**) in the same `AgentRunChanged` that moves it to `BLOCKED`. It is drawn
by T335's own `PermissionPrompt` — at the foot of the run's console, and as a
`permission` item in the attention queue, the island count, the banner and the
status-bar count — and answered with `runs.answerPermission { runId, askId,
ALLOW_ONCE | DENY }`. The verdict goes back on the child's stdin; the run leaves
`BLOCKED` by the existing `RUN_RESUMED` when the child acknowledges.

Every way an ask ends without a yes is a denial: a user's `DENY`, a cancel or
shutdown (deny, then kill), an ask withdrawn with no answer, a second ask while
one is outstanding (the first stands, and the second's echo cannot unblock the
run), an ask naming no tool, and a `control_request` of another subtype
(refused with an error response). There is no timeout anywhere.

Landed in two commits: `084cafe` (adapter, core, command, protocol) and the
renderer commit that carries this report.

### What was established first (§"What is already verified" left two open)

Executed against `claude` **2.1.270**, darwin 25.6, `--model haiku --tools Bash`,
from a scratch directory, with a Python harness speaking the exact frames the
adapter writes:

1. **The denial shape** was already in the recorded fixture
   (`crates/agent-claude/fixtures/two-turns-denied.jsonl`, lines 10–12):
   `{"behavior":"deny","message":…}` is honoured and the message reaches the
   model as the `tool_result` content with `is_error: true`.
2. **`updatedInput` may be modified, and the CLI honours it.** Asked for
   `touch asked.txt`, allowed with `{"command":"touch edited.txt"}`: only
   `edited.txt` was created. Not built (non-goal); filed as `T310-EDIT-INPUT`.
3. **A bare `{"behavior":"allow"}` runs the call as issued** — so Gritty never
   echoes the input back and nothing it holds can alter what runs.
4. **Asks are serial.** Two parallel `Bash` calls in one assistant message gave
   one `can_use_tool`; the second arrived only after the first was answered
   (held 15 s). Required behaviour 4's "second ask is a protocol violation" is
   therefore accurate rather than a race.

### The seam, and where it was not a swap

The task said a rework would mean the seam was drawn wrong, "and that is worth
reporting rather than routing around". Half of it held and half did not.

- **Held:** `ClaudeVerdict` in the adapter. It serialises whatever it is given;
  nothing in it changed except two constructors and the reasons a model reads.
- **Did not hold:** `PermissionPolicy::decide(&ask).await`, called inline in the
  driver loop. A verdict that waits for a person cannot be awaited there: while
  the loop sat in it, it could observe neither a cancellation nor the child
  dying, so required behaviour 5 and the "child dies while asked" row were
  unreachable. And `RunInput`'s docs expected the verdict to travel the inbox,
  but an ask arrives mid-turn, exactly when the inbox arm is not polled.
  `PermissionPolicy` is gone; the manager hands the driver a per-ask
  `PendingVerdict` (a oneshot) and the loop has one more `select!` arm.
- **Also found:** the supervisor aborted a cancelled driver immediately, which
  cut the "deny, then kill" write off at its first await — the stub test caught
  it (the child was killed without ever being told no). It now waits one
  `CANCEL_JOIN_GRACE` (250 ms) for the driver to stop on its own before
  aborting. This changes every run's cancellation, not only Claude's: an
  uncooperative driver now ends up to 250 ms later than before. The orphan
  guarantee is unchanged.

### Scope widening, agreed before it was built

Required behaviour 1 ("reaches the same queue as a session's") could not be met
inside the recorded scope. Alexis chose "widen and deliver". Added to the scope:
`features/workstreams/{attention.ts, attention-queue.tsx, island.tsx}` and two of
their render tests, `features/agents/permission-{banner,model,prompt}` (after
W-turn-end released them), `App.tsx` (four props, claimed for the edit), and
`crates/gritty-core/src/{protocol/{mod,export}.rs, event/mod.rs}` (the version,
the exports, and one test literal), plus `docs/architecture/agent-runs.md` and
`docs/merge-queue.md`.

### Files changed

- Adapter: `crates/agent-claude/src/{message,observation,stream,request,process,lib}.rs`,
  `stream/tests.rs` — the ask carries its classified `intent` (and a one-line
  input only for an unclassified tool; a write's content never leaves the
  adapter), `RequestUnsupported`, the denial reasons, `refuse`.
- Core: `crates/gritty-core/src/agent_run/{wire,manager,request,claude_driver,mod}.rs`,
  `claude_driver/tests.rs`, `tests.rs`; `protocol/{mod,export}.rs`; `event/mod.rs`.
- Shell: `apps/desktop/src-tauri/src/{commands,lib}.rs`, `tests/commands.rs`.
- Protocol: `packages/protocol/src/{commands,client,types}.ts` and generated
  `AgentRun`, `AgentRunPermissionAsk`, `AgentRunPermissionVerb`,
  `AgentRunPermissionVerdict`, `AnswerRunPermissionRequest`, `version`.
- Renderer: `features/runs/{run-permission.ts (new), run-console.tsx,
  run-console-model.ts, use-runs.ts}`; `features/agents/permission-{model.ts,
  prompt.tsx, banner.tsx}`; `features/workstreams/{attention.ts,
  attention-queue.tsx, island.tsx}`; `App.tsx`; tests beside each.
- Docs: `docs/architecture/agent-runs.md`, `docs/merge-queue.md`, this file.

### Tests actually executed

- `cargo test -p gritty-agent-claude` — 82 passed.
- `cargo test -p gritty-core` — 549 passed (regenerates the protocol types). The
  permission tests include stub-child process tests asserting the **side
  effect** (a file the stub creates only when allowed): allow, deny, cancel while
  blocked (deny logged by the child, no child left), shutdown while blocked
  (`INTERRUPTED`, denied, no child), death while asked (ask withdrawn), a second
  ask (denied; the run stays `BLOCKED` on the first; the first is then answerable),
  malformed and unsupported requests (refused, nobody asked), and a paused-clock
  test that a day-old unanswered ask has still not resolved.
- `cargo test -p gritty-desktop --test commands run` — 10 passed, including a new
  IPC test: allow creates the file, `ALLOW_FOR_RUN` is `InvalidInput`, deny does
  not and the run parks `WAITING`, a stale answer is not an error, cancel with an
  ask on screen ends `CANCELLED`, the child logs a deny and is gone.
- `cargo clippy -p gritty-agent-claude -p gritty-core -p gritty-desktop
  --all-targets -D warnings` and `cargo fmt --all --check` — clean.
- `npx vitest run` (whole JS suite) — 126 files, 1346 tests passed.
  `tsc` for `apps/desktop` and `packages/protocol`, and `biome check .` — clean.
- **Mutations**, each failing the tests aimed at it, then restored: the user's
  `DENY` sent as an allowance (2 failures); the echo of a quietly denied ask not
  swallowed (2); the textarea keystroke guard removed (1). The immediate abort on
  cancel failed 2 process tests before it was fixed.
- **Not run:** full `pnpm check` as one command (the tree carries W-turn-end's
  uncommitted `AgentRoster.ts`, so `check:generated` cannot pass until they
  commit it; every other stage above was run separately). **The manual pass in
  `pnpm dev` was not run** — no screen — so a real edit allowed and denied
  through the UI against a real `claude` has not been seen. The wire facts it
  depends on were exercised against the real binary as described above.

### Architectural decisions

- The ask lives on `AgentRun` (additive, optional) rather than in a new event or
  command; its types are new run-lane enums spelled like the session lane's, so
  the lanes still never import each other.
- A oneshot per ask, not the inbox (see "The seam").
- Answering an ask that has already gone returns the run unchanged, not an error
  — the rule `agents.answerPermission` already holds.
- No `ALLOW_FOR_RUN` for runs; the card offers no scope, so `⌘⏎` does nothing.
- `isTypingInField` now ignores every `textarea` except Monaco's. Before, `⏎`
  typed into the run console's prompt box or into a terminal (xterm's hidden
  textarea lets keys bubble) while a card was on screen would have allowed the
  tool. T335's deliberate Monaco behaviour is unchanged.

### Known limitations

- A run's ask has no diff, no "outside its worktree" and no redaction marker:
  the core does not look at a write's content, and a run's paths are absolute and
  not classified against the workspace (the same limitation `AgentRunPath` has
  for runs today).
- A pipe that breaks while writing an answer ends the run `FAILED`, not
  `INTERRUPTED` as §"Failure behavior" asks — T310-B's contract collision (a
  driver cannot reach `INTERRUPTED`), unchanged here and still unfiled as a
  ticket.
- A malformed `can_use_tool` with no `request_id` cannot be answered at all; it
  is recorded as unknown and the run is left for the user to cancel.
- The error response for an unsupported subtype follows the Agent SDK's shape and
  was not observed against the binary — no release sends another subtype to a
  stdio permission tool.

### Follow-ups (filed here, not built)

- **T310-EDIT-INPUT** — an editable permission prompt; the CLI honours a modified
  `updatedInput` (verified).
- **T310-GRANTS** — remembered grants for a run.
- **T310-VERDICT-EVENT** (already on the board) — the verdict is not recorded in
  the run's ring; only `RUN_RESUMED` is.
- **Run activity cleared by an unmodelled line.** `claude_driver::apply` sets the
  activity to `None` for an `Unknown` observation (e.g. `rate_limit_event`), so a
  run `BLOCKED` on an ask can lose its `REQUESTING_PERMISSION` activity. Pre-existing.
- **Monaco-focused `⏎` allows an ask** (T335's choice). Worth revisiting now that a
  split view can show an editor and a card at once.

### Remaining risks

- The 250 ms cooperative window on cancel applies to every run. Tests pass, but
  a driver that ignores its token is now stopped 250 ms later than before.
- The textarea guard is a heuristic on `.monaco-editor`; a future editor surface
  that is a textarea outside that class would silence the card's chords, which is
  the safe direction.

