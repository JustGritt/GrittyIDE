# ADR-035: Orca parity on the core loop comes after cleanup and before Phases 3–4

- **Status:** Accepted
- **Date:** 2026-09-15
- **Accepted:** 2026-09-15 by Alexis, in conversation. Asked how to reconcile
  this order with the plan of record's forward sequence, Alexis chose
  "parity after cleanup": the plan of record's Stage 0 plus the work already in
  flight becomes M0, then M1–M3, and everything listed under *Deferred* waits
  until M3 exits.
- **Amends:** the forward sequence of the plan of record
  (`PLAN-OF-RECORD`, one project plan replaces the founding plan, its addendum
  and the roadmap), and how the founding priority chain is applied (below).
- **Builds on:** ADR-007 (a branch and worktree per workstream), which M1
  finally implements.

## Context

On 2026-09-15 Alexis found Orca (`stablyai/orca`, onorca.dev): an MIT,
Electron "agent development environment" whose unit of work is a task running
in its own git worktree, with any CLI agent launched into a terminal there,
status and notifications, a diff review whose line comments go back to the
agent, and push/PR in the app. It is roughly what Gritty's project plan
describes as its future, built by a funded team over six months (about 11,000
commits, a release every ~1.6 days).

Alexis decided Gritty continues as a **portfolio and learning project** whose
goal is a polished product **on par with Orca**. A read-only survey of both on
the same day found the gap is not in the foundation but in the loop:

| Step of the loop | Orca | Gritty on 2026-09-15 |
|---|---|---|
| Open a repository | yes | yes |
| Create a task: a worktree and a branch | yes | no worktree code; `git-write` allows only `add`, `reset`, `stash`, `commit` |
| Launch an agent in it | ~35 CLIs, in a terminal | a Claude run, from a panel mounted only under `import.meta.env.DEV` |
| Status, notification, permission | yes; permissions default to `--dangerously-skip-permissions` | Claude only; evidence-labelled status; verdicts with diff preview and deny by default |
| Review the diff against the base, comment back | yes | a unified diff against `HEAD` in the sidebar, whole-file staging, no comments |
| Commit, push, open a PR | yes | commit only; networked git is unwritten ADR-030 |
| Clean up the task | yes | no |

The same survey found that 237 of 346 commits touched only docs, scripts or the
board, and that none of the agent surfaces had been seen working in the running
app. At that ratio the loop is months away.

The plan of record (approved the same morning) sequences the dogfood gate, then
the owned-run lane, then Phases 3–4 — capabilities, context manifest, ChangeSet,
write interception — and puts worktrees last. That order builds depth under a
loop that does not exist yet.

## Decision

### 1. Parity means the core loop, not the feature list

Gritty reaches parity when a person can, in a release build:

1. open a repository;
2. create a task, which is a workstream owning a worktree and a branch cut from
   a base they chose;
3. launch an agent CLI into a terminal in that worktree;
4. see its status, be notified when it needs them, and answer its permission
   prompts;
5. review the task's diff against its base and send line comments back to the
   agent;
6. commit, push and open a pull request;
7. remove the task, with unmerged work surfaced before anything is deleted.

Orca's browser and design mode, mobile companion, SSH relay, cloud runtimes,
issue-tracker integrations, plugin marketplace and its catalogue of 35 agents
are **not** parity goals. They are revisited after M5.

### 2. Sequence

Each milestone ends with a feature check by a person in the running app, one
feature at a time, not a batched pass.

- **M0 — demoable.** The plan of record's Stage 0 (commit what sits at review
  uncommitted, drain the review queue, close dead rows), the work already in
  flight (T099-S split terminals, T080-U the search panel, T102-U staging and
  the commit box, T120-U/M/T/A the settings page), SIDECAR-FRESH-CHECKOUT (a
  fresh clone builds), a README that describes what exists, and RUN-LAUNCHER
  (start a Claude run from a release build).
- **M1 — a task is a worktree.** T400-R (worktree create and remove), T400 (a
  workstream owns a worktree and a branch), WORKTREE-SETUP (a setup command
  when a task's worktree is created). This amends ADR-029's allowlist to admit
  `worktree` and `branch`, by an amendment written with T400-R.
- **M2 — agents in terminals.** AGENT-TERMINAL (launch an agent CLI into a
  terminal in a task's worktree), T222-SEAM (a provider registry) and
  T222-CODEX (Codex as one provider file), RUN-NOTIFY (a run Gritty started
  notifies like a session does).
- **M3 — the review loop.** DIFF-BASE (a task's diff against its base), T101-D
  (the diff as an editor tab), HUNK-STAGE (stage and discard by hunk),
  REVIEW-COMMENTS (line comments sent to the agent as one prompt). The diff
  against a base is the first ChangeSet in ADR-007's sense: base revision,
  files and a diff summary, not raw `git diff` text. **M3 exits when one Gritty
  ticket is built with Gritty's own loop**: task, agent, review, commit.
- **M4 — ship code, and polish.** ADR-030 and push, pull and PR creation through
  `gh`; terminal WebGL renderer, search, links and scrollback restore; replace in
  files and include/exclude globs; a keybinding registry and a real command
  palette; an autosave setting; IGNORE-RULES → IGNORE-SEAM → IGNORE-WIRE →
  IGNORE-UI (ADR-034's meaning of "ignored"); usage display; T110 diagnostics.
- **M5 — release.** Signing and notarization, an updater, a release workflow,
  Linux packages once CI returns, a landing page and a demo.

Tickets for M4 and M5 are added when M3 exits, not before.

### 3. Deferred until M3 exits

T305 (agent capability model) and the capabilities ADR, T320 (context
manifest), a ChangeSet spec beyond what M3 needs, T330-MEASURE / T330-R /
T330-U (write interception), T405 (ownership-aware scheduler), T310-RESUME,
T300-ANSWER and T300-PANEL, T097 (shell integration and command blocks), and
the plan of record's five-day dogfood gate, which M3's exit replaces. Nothing
here is cancelled; ADRs already accepted for these stay accepted.

### 4. How the priority chain applies

`correctness → recoverability → inspectability → responsiveness → ergonomics →
orchestration power` still governs **how** a feature is built: no feature ships
that loses work, lies about state or blocks the renderer. It no longer governs
**which** feature is built next; the milestones above do.

Where Gritty differs from Orca on purpose, it keeps the difference:
permission prompts are answered, never bypassed by default; a status names its
evidence; the core owns document state (an autosave setting is allowed, a
Monaco-owned buffer is not); process trees are killed and output is bounded.

### 5. Process, lighter

- **A ticket is something a person can see work, or a defect.** Split one only
  when the parts are built by different sessions in parallel (the
  contract-first fan-out), not to stage measurement, backend and UI serially.
- **An ADR is for a decision that constrains code beyond one ticket**: a
  contract, a dependency, a policy. A choice local to one ticket goes in its
  completion report.
- **A flaky test gets a ticket only when it blocks `pnpm check` for another
  session**; otherwise it is a line in the report of the ticket that saw it.
- **From M0's exit on, no new `take` while more than 10 tickets sit at
  review** (the plan of record's proposed rule, adopted). M0 itself drains the
  36 at review on 2026-09-15; only Alexis can flip them, so M0's own tickets
  are exempt.
- **Board and lease tooling changes only when a failure blocked work**, and the
  ticket says which.

## Consequences

- The plan of record's forward sequence follows this ADR; its Stages 2–4 are
  re-read as M0–M3 plus *Deferred*.
- The owned-run tickets already specified (T310-VERDICT-EVENT,
  T310-U-CANCEL, T310-RUN-PATHS, T310-U-FILES) are not deferred; they fold into
  M2 where they serve a run launched from a task, and are otherwise dropped at
  M2's start.
- M1 touches two frozen-adjacent surfaces: the workspace model must admit a
  second root per repository (ADR-007 anticipated it), and ADR-029's allowlist
  grows. Both need their amendments before code.
- Being behind Orca on surface features is accepted until M4. A portfolio
  reviewer sees a working loop with visible reliability, not a long feature
  list.

## Revisit when

- M3 exits: re-sequence M4 and M5 against what dogfooding the loop found.
- Alexis drops the parity goal, or Gritty stops being a portfolio project.
- Orca's core loop changes shape (for example, tasks stop being worktrees).
