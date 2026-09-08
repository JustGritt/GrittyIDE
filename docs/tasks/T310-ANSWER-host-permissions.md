# Task T310-ANSWER — The user's verdict, behind the seam T310 built

- **Phase:** 4 (orchestration)
- **Status:** todo
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

- `docs/tasks/T310-claude-code-adapter.md` §7 — the seam this replaces
- ADR-020 — the verdict vocabulary, and why it is what it is
- `docs/tasks/T335-agent-permissions.md` — the queue, and "asked in place,
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

_To be filled by the implementing session: summary, files changed, tests
actually executed with results, architectural decisions, known limitations,
follow-up tasks, remaining risks._
