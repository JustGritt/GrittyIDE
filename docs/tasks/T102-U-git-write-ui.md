# Task T102-U — Staging, discard and the commit box in the git panel

- **Phase:** 2
- **Status:** blocked (behind ADR-029 and T102-R)
- **Depends on:** T102-R, T101-U

## Objective

The affordances: stage and unstage per row and per band, discard behind a
confirmation that says exactly what it will do, and a commit box above the list.

## Motivation

The panel already draws **Staged** and **Changes** as separate bands, because
`groupEntries` reports what the repository says — a partially staged file is in
both, "because that is what it is". The bands are there; only the verbs are
missing.

## Required reading

- `docs/decisions/ADR-029-git-write-backend.md`
- `apps/desktop/src/features/git/git-model.ts:51-80` — `groupEntries` and why a
  file can be in two bands
- `apps/desktop/src/features/explorer/explorer-tree.tsx:660-705` — `DeleteDialog`
  and its policy: only an *irreversible* delete asks
- `apps/desktop/src/features/explorer/use-explorer.ts:309-334` and
  `explorer-panel.tsx:128-139` — the `Conflict`-then-`force` replay, **and the
  bug beside it** (see below)
- `apps/desktop/src/components/error-envelope.tsx`

## Allowed scope

- `apps/desktop/src/features/git/**`
- `apps/desktop/src/app.css` (the git block only)

## Explicit non-goals

- No branch switcher, no push/pull buttons — there is nothing on the wire for
  either, deliberately.
- No commit amend of a *pushed* commit without saying so; amend is offered, and
  the danger is stated rather than prevented.
- No staging of individual hunks. A real feature, a much larger one, and a
  separate ticket — the wire takes whole paths.

## Required behavior

1. Stage/unstage on a row, and on a band header for the whole band. The paths
   are always explicit; there is no "all" on the wire to call.
2. **A commit box** with the message, a commit button disabled while the message
   is empty or nothing is staged, and the branch it will commit to in view.
3. **Amend and no-verify are visible choices, never defaults.** `--no-verify`
   must say that it skips the project's own hooks; ADR-029 decision 4 makes
   running them the point.
4. **Discard asks, and says what will happen** — and the two cases are worded
   differently, because they are different:
   - tracked modifications are **stashed** before being restored, so the dialog
     says where the work went and how to get it back;
   - untracked files go to the **Trash**, which is `files.delete`'s existing
     disposition and is recoverable outside Gritty.
   This is the one irreversible corner of the feature and recoverability sits
   second in the priority order.
5. **The dirty-buffer refusal is wired correctly.** A discard over an open dirty
   buffer comes back `Conflict` with `details.dirtyDocuments`; the notice offers
   to discard those buffers and retry, and that button calls the *discard*
   replay.
6. **A commit is an operation**: while a hook runs, the box says so and offers to
   stop it. A hook's refusal is shown with its output, not swallowed.
7. The response's changed paths refresh the list; no re-poll of `git.status`.

## A bug to fix first, not to copy

`use-explorer.ts:309-334` defines `deleteAnyway` and returns it, and **nothing
calls it**: the notice at `explorer-panel.tsx:128-139` wires `discardAndRetry`,
which is the *open/close* force replay. So `files.delete`'s dirty-document
`Conflict` currently offers a button that does something else. Requirement 5 is
the same pattern, so `EXPLORER-FORCE-REPLAY` should land first.

## Failure behavior

- Every refusal renders as an `ErrorEnvelope`, never swallowed.
- `git` unavailable → the write affordances are shown **disabled with the
  reason**, never absent (which would look like a feature that does not exist)
  and never enabled-and-inert.
- A commit that a hook refused leaves the message in the box. Losing a commit
  message to a failed hook is the small cruelty this must not commit.

## Acceptance criteria

- Stage, unstage, discard and commit work against a real repository, in the
  running app.
- `pnpm check` green.

## Tests required

- Render tests for every arm, the way `changes-view-render.test.ts` and
  `history-render.test.ts` already mount theirs without a transport.
- The confirmation's wording for both discard cases.
- The disabled state when the `git` probe found nothing.

## Commands to run

```
pnpm exec vitest run apps/desktop/src/features/git
pnpm check
```

## Completion report

_Blocked; not started._
