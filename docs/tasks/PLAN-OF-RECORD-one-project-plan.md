# Task PLAN-OF-RECORD — One project plan replaces the founding plan, its addendum and the roadmap

- **Phase:** docs
- **Status:** review
- **Depends on:** –

## Objective

Three documents in `docs/` claimed to be Gritty's plan, and none of them
matched the repository any more:

- `docs/Gritty IDE Project Plan.md`;
- `docs/Gritty IDE Architecture Refinements.md`;
- an untracked, shortened copy of the second, `docs/Gritty Architecture
  Refinements.md`, created on 2026-09-14.

Their content had also been restated in `docs/plans/roadmap.md`, `AGENTS.md`
and `docs/tasks/README.md`, and each copy drifted differently. This task writes
one plan of record that states what Gritty is, what is actually built, which
contracts hold, where each phase stands, the canonical dependency graph, the
forward sequence and the open decisions. Every other place then points to it.

## Dependencies

None. ADR-035 (Orca parity on the core loop comes after cleanup and before
Phases 3–4) was accepted while this task was in progress and owns the forward
sequence. The plan points to it.

## Required reading

- both founding documents, and the untracked copy (compared word by word);
- every ADR in `docs/decisions/`, and every file in `docs/architecture/`;
- `docs/plans/roadmap.md`, `execution-board.md`, `phase-0-spike.md`, `Gritty
  Agent Runtime Coordination.md`;
- `docs/tasks/README.md`, the board, `docs/merge-queue.md`;
- the code facts the plan quotes: `PROTOCOL_VERSION`, `SCHEMA_VERSION`, the
  workstream and run state enums, the crate list, `App.tsx`'s development-only
  panels.

## Allowed scope

- `docs/plans/project-plan.md` (new)
- `docs/plans/roadmap.md`, `docs/plans/execution-board.md`
- `docs/Gritty IDE Project Plan.md`, `docs/Gritty IDE Architecture
  Refinements.md` (banner only)
- `docs/Gritty Architecture Refinements.md` (delete)
- `README.md`
- `AGENTS.md` and `docs/tasks/README.md`, shared files claimed only for the
  minute of the edit
- this file

## Explicit non-goals

- Fixing the drifted architecture docs and ADR text found along the way. They
  are listed in the plan's Appendix B instead.
- Editing the founding documents' bodies. Code comments, ADRs and task files
  cite their section numbers.
- Deciding any open architectural question (capabilities, ChangeSet, context
  manifest, ADR-030 networked git).
- Creating board tickets for follow-ups.

## Required behavior

- **The plan's authority order:** accepted ADRs, then the plan, then the
  architecture docs, then the founding documents as history.
- **What the plan does not hold:** ticket status (the board), version numbers
  (`docs/merge-queue.md`) and agent rules (`AGENTS.md`).
- **Only one copy** of the phase table, the graph and the sequence exists.

## Failure behavior

Not applicable: documentation only.

## Acceptance criteria

- Every backticked repository path in the plan exists, or is deliberately
  absent (`packages/ui`, `packages/workstreams`, `.gritty/`).
- Every ADR cited exists, except the reserved or unwritten 027, 030 and 036,
  which the plan names as such.
- Every ticket id cited is on the board.
- The state enums and version numbers quoted match the code.
- `AGENTS.md` and `README.md` no longer name the founding documents as the
  authority.

## Tests required

A reference check over the plan (paths, ADRs, ticket ids against
`agent-board.py list`), and greps of the code facts it quotes.

## Commands to run

The reference check script (kept in the session scratchpad; its logic is
described in the completion report). `pnpm check` does not read Markdown.

## Completion report

- **Summary.**
  - Wrote `docs/plans/project-plan.md`, the plan of record, with 14 sections
    and two appendices:
    - how to read it;
    - what Gritty is (updated for ADR-035's Orca-parity goal and for the two
      agent lanes);
    - principles;
    - platform;
    - architecture as built;
    - a domain model status table;
    - gates A–D plus the ownership gate O;
    - cross-cutting policies with the measured performance baseline;
    - phase status;
    - the canonical dependency graph;
    - the forward sequence, pointing to ADR-035;
    - a crosswalk of the founding open questions;
    - open decisions;
    - success criteria;
    - how to keep the plan true;
    - Appendix A, the three sources and how their contradictions were settled;
    - Appendix B, documentation drift.
  - Froze the founding documents behind a banner.
  - Deleted the untracked copy.
  - Reduced `roadmap.md` to a pointer.
  - Marked `execution-board.md` as a historical snapshot.
  - Repointed `AGENTS.md` (reading list and condensed graph) and `README.md`
    (status, stack, map, documentation). The README status no longer says
    "Phase 0, nothing usable".
  - Replaced the hand-maintained graph in `docs/tasks/README.md` with a pointer.
- **Files changed.** `docs/plans/project-plan.md` (new), `docs/plans/roadmap.md`,
  `docs/plans/execution-board.md`, `docs/Gritty IDE Project Plan.md`,
  `docs/Gritty IDE Architecture Refinements.md`, `README.md`, `AGENTS.md`,
  `docs/tasks/README.md` (prose and graph section only; the table is
  `render --index`), this file. Deleted: the untracked `docs/Gritty Architecture
  Refinements.md`, which was never committed.
- **Tests actually executed.**
  - **Reference check** (Python, scratchpad). It extracts backticked paths,
    `ADR-NNN` and ticket ids from the plan and checks them against the tree,
    `docs/decisions/` and `agent-board.py list`.
    - 8 paths reported: all deliberate non-paths (`files.rename/move`,
      `CANCELLED/FAILED`) or deliberately absent (`packages/ui`,
      `packages/workstreams`, `.gritty/`).
    - 3 ticket ids reported: regex truncations of `T310-RUN-PATHS` and
      `T310-VERDICT-EVENT`, and the `T130-*` family. All three are on the board.
    - No ADR missing beyond the reserved and unwritten ones.
  - **Code facts, by grep.**
    - `WorkstreamState` is `Draft Ready InProgress Blocked InReview Done
      Abandoned`.
    - `AgentRunState` has the 10 states quoted.
    - `PROTOCOL_VERSION` is 2.27.0 and `SCHEMA_VERSION` is 4.
    - `MAX_CONCURRENT_RUNS` is 4 and `MAX_SESSIONS` is 16.
    - `AllowForRun` exists for observed sessions.
    - `RunsPanel` mounts only under `import.meta.env.DEV`.
    - CI `paths-ignore` covers `docs/**` and `**/*.md`.
  - **`pnpm check` was not run.** The change is Markdown only and no gate reads it.
- **Results.** The reference check is clean apart from the deliberate exceptions
  above.
- **Architectural decisions.** None new. The plan records existing ones and
  follows ADR-035 for sequence. One judgement is written into the plan rather
  than an ADR: run persistence (ADR-021) belongs in M2, because a run launched
  from a task that vanishes on restart fails recoverability. ADR-035's deferred
  list does not mention it.
- **Known limitations.**
  - The plan's phase status is a dated snapshot (2026-09-15) and will go stale.
    §14 says when to update it.
  - The Coordination plan (`docs/plans/Gritty Agent Runtime Coordination.md`)
    was not reconciled line by line. The plan subordinates it to ADRs.
- **Follow-up tasks.**
  - A ticket to implement ADR-021 run persistence (M2).
  - The documentation drift in the plan's Appendix B: `overview.md` events,
    `security.md` missing sections, `terminal.md`, `ownership.md`,
    `agent-sessions.md`, `agent-runs.md`, `persistence.md`, `documents.md`, the
    decisions README amendment index, stale ADR text (021, 024, 032, 008), and
    the broken `docs/tasks/docs/tasks/…` links in the tasks index.
  - **The broken links' cause.** This ticket reproduced it:
    - `add --link docs/tasks/<file>.md` renders a broken row, and `render
      --index` never updates that task file's Status line
      (`task_file_candidates` in `scripts/agent-board.py` ignores any link
      containing `/`).
    - Here it was fixed with `add --force` and a bare file name, passing every
      field.
    - The other nine rows need the same fix. Either `add` should refuse a link
      with a directory, or the tasks index should say so.
  - Update the platform matrix as M0–M3 feature checks land.
