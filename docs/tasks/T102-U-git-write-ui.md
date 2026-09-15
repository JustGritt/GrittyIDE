# Task T102-U — Staging, discard and the commit box in the git panel

- **Phase:** 2
- **Status:** review
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
- `apps/desktop/src/features/explorer/use-explorer.ts` (`Refusal`,
  `discardAndRetry`) and `explorer-panel.tsx:128-139` — the
  `Conflict`-then-`force` replay, now correct (see below)
- `apps/desktop/src/features/explorer/use-explorer-force-replay.dom.test.tsx` —
  how that replay is tested through the hook
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

## The bug to copy from — fixed 2026-09-09, so copy it

This section used to say `deleteAnyway` was dead code and the notice replayed
the wrong action. **`EXPLORER-FORCE-REPLAY` (the dirty-document force replay)
landed in `d104d29` and fixed it**, so requirement 5 now has a correct pattern
to follow rather than a trap to avoid.

What it settled, and what this ticket should mirror: there is **one** refusal
slot, not one per command. `use-explorer.ts` holds a single `Refusal`
discriminated union over the three commands that can earn a dirty-document
`Conflict`, each writing it before the call and clearing it on success, and
`discardAndRetry` switches on the kind. Two slots meant the single notice
button had to guess which one to replay, and the guess was the defect. A git
discard adds a fourth case of exactly the same shape — resist giving it its own
slot.

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

### Summary

The Changes panel can now stage, unstage, discard and commit. Every one of
the four writes goes through `core.git.*`, and the status list takes its new
state from the write's response instead of reading `git.status` again.

- **Stage and unstage** buttons on each row and on each band header. Changes
  and Untracked offer Stage and Discard, Staged offers only Unstage, and
  Conflicts offer nothing (see limitations). The paths are always sent one by
  one: "Stage all" sends the band's own paths.
- **The commit box** sits above the list. It shows the branch the commit lands
  on and discloses that hooks run. The button stays disabled, with its reason
  shown, while the message is empty, while nothing is staged (unless amending),
  while a write is in flight, and while git cannot write. ⌘↩ commits.
- **Amend and `--no-verify` are unticked checkboxes** and go back to unticked
  after every commit. Amend states its cost from the upstream counts ("which
  origin/main already has … the branch will diverge"). No-verify says it skips
  the project's own checks.
- **Discard asks first, through the new shared `components/confirm-dialog.tsx`.**
  The wording differs for the two dispositions. A tracked file is stashed, and
  the dialog names `git stash list` and `git stash apply`. An untracked file
  goes to the Trash because Git holds no copy of it. When a file in the Changes
  band also has staged changes, the dialog says that `git stash push -- <path>`
  stashes and unstages the staged half too. After the discard, a notice names
  the stash by its commit id, because `stash@{0}` goes stale.
- **The dirty-buffer refusal** reuses the pattern from `EXPLORER-FORCE-REPLAY`
  and `EXPLORER-DISCARD-NOTICE`, with one slot. The refused request is stored on
  the write state itself, so a new write always replaces it. "Discard them and
  continue" appears only when that write is a discard *and*
  `isDirtyDocumentRefusal` accepts the error. It re-sends the same paths with
  `force`.
- **A commit is an operation.** While a pending commit's hooks run, the box says
  so and offers Stop. A hook's refusal is shown as an `ErrorEnvelope` plus git's
  own output as text, and the message stays in the box. A stopped commit
  re-reads the status, because whether HEAD moved is unknown.
- **git unavailable:** every verb is drawn disabled, with the probe's reason as
  a note and as each button's tooltip. A verdict that has not arrived yet
  counts as unavailable.
- **Drafts survive the panel unmounting.** The panel unmounts whenever the Files
  tab is showing, so a module-scoped map keyed by workspace holds the message
  until a commit succeeds. It is never written to disk.

### Files changed

- `apps/desktop/src/features/git/git-reducer.ts`: the `Write` state, `WriteRequest`,
  `CommitDraft`, `availability`, `WRITE_OPERATION_KIND` (`git.write`) routed
  through the existing `settle`/`claim`, and `converge`. Converge replaces the
  status and decides per verb what goes stale: the diff after a discard or a
  commit, the history after a commit.
- `apps/desktop/src/features/git/git-model.ts`: `bandVerbs`, `discardPlan`,
  `discardWording`, `discardReport`, `writeUnavailableReason`, `commitBlocker`,
  `amendWarning`, `gitRefusal`.
- `apps/desktop/src/features/git/use-git.ts`: `stage`/`unstage`/`discard`/`commit`,
  `changeDraft`, `canDiscardAndRetry`/`discardAndRetry`, `retryWrite`,
  `dismissWrite`, `cancelWrite`, and the draft map.
- `apps/desktop/src/features/git/changes-view.tsx`: `WriteControls`, `CommitBox`,
  `WriteFeedback`, `BandActions`, row actions, `DiscardDialog`.
- `apps/desktop/src/features/git/changes-panel.tsx`: the pending discard
  confirmation. History is now read by an effect while its half is showing, so
  a commit that resets it to idle reads it again.
- `apps/desktop/src/components/confirm-dialog.tsx` (new): focus starts on
  Cancel, Escape cancels, `alertdialog` labelled by the title and described by
  the body. Scope widened with the product owner's agreement (see decisions).
- `apps/desktop/src/app.css`: the git block only.
- Tests: `git-reducer.test.ts` (+17), `git-model.test.ts` (+17),
  `changes-view-render.test.ts` (+21), `use-git-writes.dom.test.tsx` (new, 8),
  `components/confirm-dialog.dom.test.tsx` (new, 4).

### Tests actually executed

- `pnpm exec vitest run apps/desktop/src/features/git apps/desktop/src/components/confirm-dialog.dom.test.tsx`:
  **159 passed**.
- `pnpm check:ts` (Biome, tsc, every vitest project): **green, 129 files, 1433 tests**.
- `pnpm check:generated`: **green** (no protocol change).
- `pnpm check:rust`: see "Rust gate" below.
- **Mutation checks:** seven deliberate breaks, each run against its tests and
  then restored. Each break was caught: the replay sending `force: false`,
  unknown availability read as "yes", a commit not clearing the draft, the
  draft map never saving, the dirty notice shown unconditionally, the
  dirty-document predicate replaced by `true`, and the double-send guard
  removed. The predicate break survived at first. It was caught only after
  adding the "discard refused over a held lock" test.

### Rust gate

T102-U changes no Rust. The first full `pnpm check` stopped at
`cargo fmt --check` on another session's uncommitted debug instrumentation in
`crates/workspace/src/watch.rs` (WATCH-FLAKE), so clippy and `cargo test` did
not run then. That instrumentation has since been removed and `check:rust` was
re-run:

- `cargo fmt --check` and `cargo clippy --workspace --all-targets -D warnings`:
  **green**.
- `cargo test --workspace`: **not green, for reasons outside this ticket.** The
  first run stopped at `gritty-agent-claude --test process`, where
  `a_child_that_dies_mid_turn_fails_and_carries_its_stderr` failed on an empty
  `stderr_tail`. Run alone, it failed once in five runs at load average 55–73,
  so it is flaky on its own, and no board ticket covers it (follow-up below).
  `cargo test --workspace --no-fail-fast` then ran all 42 binaries: **1279
  passed, 1 failed**. The one failure was
  `gritty-workspace --test watch::a_directory_created_after_the_watch_started_is_followed`,
  the test `WATCH-FLAKE` (two load-sensitive watcher tests) is fixing, with its
  fix still uncommitted in that live session's tree. The agent-claude test
  passed in that run.

**`pnpm check` is therefore not green on this tree**, and neither failure is in
code this ticket touched.

### Architectural decisions

- **The write state lives in the reducer, not in a `useRef` slot.** A failed
  write keeps its request, which makes the replay exactly one request, and a
  pure reducer test proves it.
- **One write at a time.** The reducer ignores a second start, and the hook
  also checks a synchronous `sending` ref. Without the ref, a double-click in
  one frame sends two requests, and with Amend ticked that means amending
  twice. This was found in review, then tested and mutation-checked.
- **The shared confirm dialog was extracted now** (the product owner picked
  "new shared component"). Only the git discard uses it. Moving `DeleteDialog`
  and `CloseDialog` onto it stays with `CONFIRM-DIALOG`.
- **`isDirtyDocumentRefusal` is imported from `features/explorer/use-explorer.ts`,
  not copied.** Moving it into `packages/protocol/src/errors.ts` means editing
  `use-explorer.ts`, which was under another session's lease (see follow-ups).

### Known limitations

- **Not exercised in the running app.** The agent cannot see or drive the Tauri
  window. The static-render screenshot pass also failed: headless Brave hangs
  here with no display, and the chrome-devtools browser was held by another
  session. **Layout (row actions appearing on hover and focus, the commit box
  height, the refusal output) is therefore unverified.** The acceptance
  criterion "work against a real repository, in the running app" needs a
  manual check: stage and unstage a row, discard a tracked and an untracked
  file (then `git stash list`), commit, and commit with a failing `pre-commit`
  hook.
- **Conflicted files offer no verb.** Staging one is how Git marks it resolved,
  and doing that from a button while conflict markers may still be in the file
  is not decided here.
- **A discarded submodule path** goes through `git stash push -- <path>`
  unchanged. The behaviour for a submodule has not been checked.
- After the dialog closes, focus is not returned to the button that opened it.
  `DeleteDialog` behaves the same way.

### Follow-up tasks

- `CONFIRM-DIALOG`: move `DeleteDialog` (explorer) and `CloseDialog` (editor)
  onto `components/confirm-dialog.tsx`, and restore focus to the opener when
  the dialog closes.
- Move `isDirtyDocumentRefusal` into `packages/protocol/src/errors.ts`. It is
  used by the explorer and git, and the explorer session already recorded this
  as its follow-up.
- A "mark resolved" verb for the Conflicts band, with a check for conflict
  markers before staging.
- A new flake ticket: `gritty-agent-claude --test process`
  `a_child_that_dies_mid_turn_fails_and_carries_its_stderr` sees an empty
  `stderr_tail` once in five isolated runs under load (probably the exit
  reported before the stderr drain finished). Not `CARGO-FLAKE`, which is the
  desktop command tests.

### Remaining risks

- A cancelled operation can race its own completion. If the cancel request
  itself fails, the write goes to `failed`, and the commit's later completion
  is parked unclaimed. Reads have the same shape. The status then shows the
  pre-commit list until Refresh.
