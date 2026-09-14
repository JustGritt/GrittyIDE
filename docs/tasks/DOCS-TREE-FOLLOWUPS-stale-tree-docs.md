# Task DOCS-TREE-FOLLOWUPS — Docs that still describe the tree before T070 and T042

- **Phase:** maintenance (ADR-025 and ADR-028 follow-ups)
- **Status:** review
- **Depends on:** FILES-PARTIAL, TRASH-FALLBACK, EXPLORER-RELIST-QUEUE (the behaviour these docs now describe)

## Objective

Make the architecture docs and two comments say what the tree does now: that
a watcher exists, what a partial move or delete leaves, what a Trash refusal
asks, and how stale directories are re-listed.

## Motivation

Recorded as follow-ups and left alone because they sat outside those tickets'
scope:

- **`docs/architecture/overview.md:39`** says `PROTOCOL_VERSION` is "currently
  `1.3.0`" (it is 2.27.0), and its command list stops at T060: no `files.*`,
  `git.*`, `search.*`, `runs.*`, `settings.*`. Both went stale the same way, a
  number and a list copied from a file that keeps changing (T042 follow-up).
- **`overview.md:78`** says a vanished root is noticed by "the command that
  notices", and never links `workspace.md` (T070 follow-up 7).
- **`apps/desktop/src/features/agents/open-touched-file.ts:26`**, and its test
  at `:86` and `:91`, say T070's watcher does not exist (T070 follow-up 8). The
  behaviour they justify is still right: a file rewritten a second ago may not
  have cleared the watcher's debounce.
- **`docs/architecture/workspace.md`** still says cancellation is "counted in
  `cancelledAfter`" (FILES-PARTIAL showed it never was), and knows nothing of
  partial failure, the Trash fallback or the re-list queue.

## Allowed scope

- `docs/architecture/{overview,workspace}.md`
- `apps/desktop/src/features/agents/open-touched-file.ts` and
  `open-touched-file.test.ts`: comments and one test name only
- this file

## Explicit non-goals

- **ADR-025's watch-degradation rows.** WATCH-DEGRADED writes them with the
  change.
- **The meaning of "ignored"** in `workspace.md`. ADR-034's IGNORE-SEAM rewrites
  that section.
- No behaviour change.

## Acceptance criteria

- No doc names a protocol version or a full command list that a later ticket
  has to remember to update.
- `pnpm check:ts` green (a test was renamed).

## Completion report

### Summary

The stale spots are fixed:

- **`overview.md`** no longer names a protocol version or copies the command
  list. It points at `COMMANDS` in `packages/protocol/src/commands.ts`,
  `protocol/mod.rs` and `docs/merge-queue.md`, which is what keeps them from
  going stale a third time. Its workspace section now says how a vanished root
  is noticed, names `WorkspaceEntriesChanged`, and links `workspace.md`.
- **`workspace.md`**'s `files.*` section says what a stopped batch leaves and
  how it is reported, drops the false `cancelledAfter` claim, and describes the
  Trash refusal. Its watch section describes the re-list queue.
- **`open-touched-file.ts`** and its test give the reason the status check
  still comes first (the watcher's debounce), not that the watcher does not
  exist. One test was renamed to match.

### Files changed

| File | What |
|---|---|
| `docs/architecture/overview.md` | Command-name and versioning rows; the workspace paragraph |
| `docs/architecture/workspace.md` | Title; the partial-settlement and Trash-refusal properties; the re-list queue |
| `apps/desktop/src/features/agents/open-touched-file.ts`, `.test.ts` | Comments and one test name |

### Tests actually executed

| Command | Result |
|---|---|
| `pnpm vitest run apps/desktop/src/features/agents/open-touched-file.test.ts` | exit 0, 5 tests |
| `pnpm exec biome check` on the two TS files | exit 0 |

### Known limitations

- `persistence.md:89` still titles a section "protocol 1.3.0". It is a
  historical heading for what that version added, not a claim about the current
  version, so it was left alone.
- `workspace.md`'s `is_ignored` paragraph ("T080's walker will call it") is
  stale too. It is left for ADR-034's IGNORE-SEAM, which replaces that predicate.
- The watch-degradation rows of the failure table wait for WATCH-DEGRADED.
