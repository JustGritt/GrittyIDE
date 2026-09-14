# Task EXPLORER-DISCARD-NOTICE — The discard notice replays only a dirty-document refusal

- **Phase:** maintenance (an ADR-028 follow-up)
- **Status:** review
- **Depends on:** T042, EXPLORER-FORCE-REPLAY

## Objective

"Discard them and continue" must appear only when the core refused an action
because documents under it are dirty, and it must replay exactly that action.
Right now it can appear after an unrelated error and replay an old delete with
`force`.

## Motivation

EXPLORER-FORCE-REPLAY gave the notice one refusal slot and made it replay that
refusal. Two things around the slot are still wrong on `6f074e6`:

1. **The notice ignores what the conflict was about.** `explorer-panel.tsx:138`
   renders it for every `error.code === 'Conflict'`. `Conflict` is also what the
   core returns for:
   - a move onto a taken name (`files.rs:738`);
   - an empty undo or redo (`files.rs:630`);
   - running out of copy names (`files.rs:912`);
   - every `AlreadyExists` (`workspace/mod.rs:587`), so a rename or a create
     onto an existing name.

   In all of those, the notice says "Documents have unsaved changes", which is
   false.
2. **The slot outlives the refusal.** `openFolder`, `closeWorkspace` and
   `deleteEntries` write the slot *before* sending, and clear it only on
   success (`use-explorer.ts:197-212`, `:327-334`). A delete that fails for any
   other reason, such as the Trash refusing or `PermissionDenied`, leaves it set.

Together: a Trash delete fails, then the user renames onto an existing name and
gets a `Conflict`. The notice offers "discard them and continue", and clicking
it sends the old `files.delete` with `force: true`. That is a destructive
action behind a sentence that describes something else.

## Required reading

- `docs/tasks/completed/EXPLORER-FORCE-REPLAY-dirty-document-replay.md`
- `crates/gritty-core/src/workspace/files.rs:521-535` (the refusal and its
  `details.dirtyDocuments`)
- ADR-028 §"Deleting something with a dirty document under it is `Conflict`
  unless forced", and hard rule 7

## Allowed scope

- `apps/desktop/src/features/explorer/**`
- this file

## Explicit non-goals

- No change to the core, the wire, or which commands can refuse.
- No new dialog and no new wording.
- The git discard UI (T102-U) is not built. This fixes the pattern it will copy.

## Required behavior

1. The slot is written only when the core's answer is a `Conflict` whose
   `details.dirtyDocuments` is a non-empty array. Any other outcome of open,
   close or delete, success or failure, leaves the slot empty.
2. The notice renders only while the current error is such a refusal and the
   slot holds the action that earned it. Every other `Conflict` shows the error
   envelope alone.
3. Replaying still sends the refused action once, with `force`, and the same
   arguments.

## Failure behavior

| Situation | Behavior |
|---|---|
| Delete refused over dirty documents | Notice offered; replay sends the same paths and mode with `force` |
| Delete fails with `Io` or `PermissionDenied` | Envelope only; the slot stays empty |
| Rename, move, create or undo `Conflict` | Envelope only, whatever an earlier action left behind |
| A malformed `details` (not an object, empty list) | Treated as not a dirty-document refusal |

## Acceptance criteria

- A rename `Conflict` with no `dirtyDocuments` renders no discard notice.
- A failed delete followed by any `Conflict` never leads to a `files.delete`
  with `force`.
- The EXPLORER-FORCE-REPLAY tests still pass.
- `pnpm check` green.

## Tests required

- `use-explorer-force-replay.dom.test.tsx`: the two scenarios above, through
  the hook, and `canDiscardAndRetry` false for a non-dirty `Conflict`.
- A panel-level assertion that the notice is absent for a plain `Conflict`.
- Mutation check: restore the write-before-send ordering and watch the
  failed-delete scenario fail.
- Manual, for Alexis: rename a file onto a name that already exists. The error
  shows, and nothing offers to discard anything.

## Commands to run

```
pnpm vitest run apps/desktop/src/features/explorer
pnpm check
```

## Completion report

### Summary

The discard notice now appears only for a dirty-document refusal, and replays
only the action that earned it. Three layers each close the hole on their own,
and each is covered by a test that fails without it:

- **The slot is written only by a dirty-document refusal.** `refusable` empties
  the slot before sending, and fills it only when the core answers `Conflict`
  with a non-empty `details.dirtyDocuments`.
- **The panel asks the controller, not the error code.** It renders the notice
  from `canDiscardAndRetry`.
- **The replay checks again.** `discardAndRetry` does nothing unless the
  refusal is the error on screen.

### Files changed

| File | What |
|---|---|
| `apps/desktop/src/features/explorer/use-explorer.ts` | `isDirtyDocumentRefusal`, `refusable`, `canDiscardAndRetry`; open, close and delete send through `refusable`; the replay is guarded |
| `apps/desktop/src/features/explorer/explorer-panel.tsx` | The notice is gated on `canDiscardAndRetry` |
| `apps/desktop/src/features/explorer/use-explorer-force-replay.dom.test.tsx` | 5 tests: a plain `Conflict` offers nothing; a failed delete then a `Conflict` never forces; a dirty refusal offers; a later error withdraws the offer; the predicate's edge cases |
| `apps/desktop/src/features/explorer/explorer-panel-notice.dom.test.tsx` (new) | 2 tests: the panel renders the notice from the controller, not from `error.code` |

### Tests actually executed

| Command | Result |
|---|---|
| `pnpm vitest run apps/desktop/src/features/explorer` | exit 0, 9 files, 105 tests |
| `apps/desktop`: `tsc --noEmit -p tsconfig.json` | exit 0 |
| `pnpm check` | **exit 1, not from this change.** `check:ts` passed (Biome, `tsc` across all 4 packages, vitest 127 files / 1353 tests). `check:rust` then stopped at `cargo fmt --check` on uncommitted debug `eprintln!`s that the WATCH-FLAKE session is using to instrument `crates/workspace/src/watch.rs`. This ticket changes no Rust and no generated types, so `check:rust` and `check:generated` have nothing of it to check |

**Mutation checks**, each run and then restored:

| Mutation | Result |
|---|---|
| A: panel back to `error.code === 'Conflict'` | `is absent for a Conflict that is not a dirty-document refusal` fails |
| B: slot armed before send and cleared on success, replay guard removed (the old behaviour) | `never forces a delete that failed for another reason` and `stops offering it once another error replaces the refusal` fail |
| C: `canDiscardAndRetry` from the code alone, with the old ordering | The same two fail |

### Architectural decisions

- **The predicate lives in the hook file, not in `@gritty/protocol`.** There is
  one consumer. T102-U (the git discard UI) will be the second, and that is the
  moment to move it next to `toCoreError`.
- **The replay guard duplicates the slot rule on purpose.** The slot is a ref
  that React does not see, so the render-time check is what makes the button and
  the replay agree.

### Known limitations

- Nothing was clicked. The panel test renders the real component on the Changes
  tab, so the tree is not mounted.

### Follow-up tasks

- When T102-U builds the git discard notice, move `isDirtyDocumentRefusal` into
  `packages/protocol/src/errors.ts` and use it there too. Its task file says to
  copy the explorer's pattern, and this is now that pattern.
