# Task TRASH-FALLBACK — A delete the Trash refuses offers permanent deletion

- **Phase:** maintenance (an ADR-028 follow-up)
- **Status:** review
- **Depends on:** FILES-PARTIAL (a move or delete that stops partway settles what it did), EXPLORER-DISCARD-NOTICE (the discard notice replays only a dirty-document refusal)

## Objective

When the OS Trash refuses an entry, the explorer asks whether to delete what
is left permanently, in the dialog it already has for that, instead of showing
a generic I/O error and leaving the user to find Shift+Delete.

## Motivation

T042's failure table promised this row and nothing built it
(`docs/tasks/completed/T042-explorer-file-operations.md:182`):

> Trash is unavailable (network or removable volume) | `Io` with the reason;
> the UI offers permanent deletion rather than failing silently

On `6f074e6`:

- `mutate::trash` (`crates/workspace/src/mutate.rs:376-382`) says, in its own
  doc comment, "the caller offers a permanent delete instead, and the user
  chooses".
- But it maps every `trash::delete` failure to `WorkspaceError::Io`, which
  reaches the wire as a plain `Io`. Nothing distinguishes it from any other I/O
  error.
- `requestDelete` (`explorer-tree.tsx:156-166`) sends `TRASH` directly. A
  failure lands in `run`'s `setError` and renders as an `ErrorEnvelope`.
  Nothing offers the permanent delete.

ADR-028 makes the Trash the recovery for a delete. A volume without one is
exactly where the user must choose, knowingly, to give that recovery up, which
is what the existing permanent-delete dialog asks.

## Required reading

- ADR-028 §"A delete moves to the OS Trash by default; permanent deletion is a
  separate, confirmed gesture"
- `crates/workspace/src/{error.rs,mutate.rs}`; `crates/gritty-core/src/workspace/mod.rs`
  `impl From<WorkspaceError> for CoreError`
- `apps/desktop/src/features/explorer/explorer-tree.tsx` (`requestDelete`,
  `confirmDelete`, `DeleteDialog`)
- `docs/tasks/FILES-PARTIAL-settle-what-was-done.md` (`details.completed`)

## Allowed scope

- `crates/workspace/src/{error.rs,mutate.rs}`
- `crates/gritty-core/src/workspace/mod.rs` (the one mapping arm, and its test)
- `apps/desktop/src/features/explorer/**`
- this file

## Explicit non-goals

- No new command, no new wire type, no protocol version bump. `details.reason`
  is free-form.
- No new dialog: `DeleteDialog` gains a cause, not a sibling (the
  CONFIRM-DIALOG ticket exists to stop a third copy).
- **No change to git discard.** `git/write.rs:250` inherits the richer details
  and nothing else. Its UI is T102-U.
- Nothing is permanently deleted without the dialog's explicit confirmation.

## Required behavior

1. **A dedicated error.** `mutate::trash` returns
   `WorkspaceError::TrashRefused { context, source }` for a `trash::delete`
   failure. Resolution errors (not found, outside the root) keep their own
   variants.
2. **Its mapping.** It becomes `CoreError { code: Io }` with
   `details.reason = "TRASH_REFUSED"` and the same message as today.
   FILES-PARTIAL's `partial_failure` adds `completed` and `requested` to the
   same object.
3. **The dialog instead of the envelope.** When `deleteEntries` in `TRASH` mode
   fails with that reason, the explorer opens the permanent-delete dialog for
   the entries the Trash did not take (`paths.slice(completed)`), naming why,
   and shows no error envelope.
4. **Confirm and cancel.** Confirming sends `PERMANENT` for exactly those
   paths, carrying the original request's `force`. Without `force`, the
   dirty-document guard runs again. Cancelling sends nothing.
5. **Everything else is unchanged.** Any other `Io`, or a `PERMANENT` failure,
   still shows the envelope.

## Failure behavior

| Situation | Behavior |
|---|---|
| The Trash refuses the first of 3 | Dialog for all 3; no envelope |
| The Trash takes 1, refuses the 2nd | Dialog for the last 2; the tree re-lists (FILES-PARTIAL) |
| Permanent delete then fails | The envelope, as for any permanent delete |
| A dirty document under the remaining paths, unforced | The `Conflict` and its discard notice, as today |
| The workspace closed while the dialog was open | The permanent request fails `NotFound`; the envelope |

## Acceptance criteria

- A stubbed `TRASH_REFUSED` after one completed entry opens the dialog naming
  the two remaining entries, and confirming sends `PERMANENT` for exactly those
  two.
- A generic `Io` still shows the envelope and no dialog.
- `pnpm check` green.

## Tests required

- A Rust mapping test: `TrashRefused` gives `Io` with
  `details.reason == "TRASH_REFUSED"` and keeps its message.
- In `explorer-tree.dom.test.tsx` (the real component over the stub transport):
  - the dialog, and its confirm;
  - cancel sends nothing;
  - a generic `Io` shows the envelope and no dialog.
- **Mutation check:** drop the reason check and see the generic-`Io` test fail.
- **Not claimed:** `trash::delete` refusing for real. No test runs it, and every
  suite deliberately avoids filling a developer's Trash.
- **Manual, optional, for Alexis:** on a volume without a Trash (an SMB mount,
  or a FAT disk image if macOS gives it none), Delete a file. The dialog asks.

## Commands to run

```
cargo test -p gritty-workspace
cargo test -p gritty-core workspace
pnpm vitest run apps/desktop/src/features/explorer
pnpm check
```

## Completion report

### Summary

When the OS Trash refuses an entry, the explorer now asks, in the
permanent-delete dialog it already had, whether to delete what is left
permanently. It no longer shows a generic I/O error.

- **Core.** `mutate::trash` returns a dedicated
  `WorkspaceError::TrashRefused`. It reaches the wire as the same `Io` with the
  same message, plus `details.reason: "TRASH_REFUSED"`.
- **Renderer.** `deleteEntries` turns that reason, on a `TRASH` request only,
  into `trashRefusal { paths: paths.slice(completed), force, reason }` instead
  of an error.
- **Dialog.** The tree shows `DeleteDialog` with a "The Trash refused …"
  heading. Confirming sends `PERMANENT` for exactly the remaining paths, with
  the original `force`.

### Files changed

| File | What |
|---|---|
| `crates/workspace/src/error.rs` | `WorkspaceError::TrashRefused { context, reason }` |
| `crates/workspace/src/mutate.rs` | `trash` returns it |
| `crates/gritty-core/src/workspace/mod.rs` | The mapping arm (`Io`, `details { kind, reason }`), `TRASH_REFUSED`, 1 test |
| `apps/desktop/src/features/explorer/use-explorer.ts` | `detailsOf` (now shared by all three detail readers), `TrashRefusal`, `trashRefusal`, `deletePermanentlyInstead`, `dismissTrashRefusal`; cleared on a new delete and on a workspace change |
| `apps/desktop/src/features/explorer/explorer-tree.tsx` | `PendingDelete.trashRefusedBecause`; the dialog mounted from `trashRefusal`; the variant heading and text |
| `apps/desktop/src/features/explorer/explorer-tree.dom.test.tsx` | 3 tests through the real component |
| `apps/desktop/src/features/explorer/use-explorer-force-replay.dom.test.tsx` | 2 hook tests |
| `docs/tasks/FILES-PARTIAL-settle-what-was-done.md` | Its manual step corrected: the batch stops at the missing entry, so only the first moves |

### Tests actually executed

| Command | Result |
|---|---|
| `cargo test -p gritty-core --lib workspace::tests::a_trash_refusal` | exit 0 |
| `pnpm vitest run apps/desktop/src/features/explorer` | exit 0, 9 files, 115 tests |
| `pnpm check` | **exit 2, not from this change.** `tsc` failed on 7 unused-declaration errors, all in `apps/desktop/src/features/git/use-git.ts`: the T102-U session's work in progress, uncommitted in the shared tree |
| The gate's halves, run separately | biome exit 0; `tsc` 0 errors outside `src/features/git/`; `pnpm vitest run` exit 0 (127 files, 1363 tests); `cargo fmt --all --check` exit 0; `cargo clippy --workspace --all-targets -D warnings` exit 0 |
| `cargo test --workspace --no-fail-fast` | **Not completed:** macOS killed it for low memory (load average about 40, several sessions building). Not re-attempted at workspace scale |
| `cargo test -j 2 -p gritty-workspace -p gritty-core` | exit 0 (556 + 3 core; 49 + 14 + 11 + 10 workspace, 1 ignored). The two crates this ticket changes, including git discard's `trash` caller |
| `pnpm check:generated` | exit 0 (no wire type changed) |

**Mutation checks**, each run and then restored:

| Mutation | Fails |
|---|---|
| The `reason` check dropped (any `Io` on a Trash delete asks) | `does not offer it for any other I/O failure` |
| The refusal offers every path again, not `paths.slice(completed)` | `offers a permanent delete of what the Trash did not take`, `becomes a question carrying what is left, not an error` |

### Architectural decisions

- **A variant, not a string match on the message.** "could not be moved to the
  trash" was the only thing that told a Trash failure apart. A `WorkspaceError`
  variant makes the distinction a type, and `details.reason` makes it a field.
  The message is unchanged, so nothing that displayed it changes.
- **The dialog's decision is the same one, so the dialog is the same one.** A
  refused Trash leaves the user where Shift+Delete starts, so `DeleteDialog`
  gains a cause rather than a sibling (CONFIRM-DIALOG exists to prevent a
  third copy).
- **A permanent request carrying the reason is not asked about.** Only a
  `TRASH` request can be refused by the Trash, and asking again after a
  permanent delete would loop.

### Known limitations

- **`trash::delete` refusing for real has not been observed.** No test runs it,
  and every suite deliberately avoids filling a developer's Trash. The path from
  the crate's error to the dialog is tested from `TrashRefused` onwards.
- **Nothing was clicked.**
- **Git discard** (`git/write.rs:250`) now gets the richer `details` too, and
  its UI (T102-U) does nothing with them.

### Follow-up tasks

- An `#[ignore]`d test, run by hand on a disposable volume, that makes
  `trash::delete` actually refuse and asserts `TrashRefused`, if a reliable
  trash-less volume can be scripted on macOS (`hdiutil` with a filesystem that
  gets no `.Trashes`).
