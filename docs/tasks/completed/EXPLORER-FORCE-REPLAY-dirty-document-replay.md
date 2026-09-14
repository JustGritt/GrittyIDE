# Task EXPLORER-FORCE-REPLAY — The dirty-document `Conflict` replays the action it refused

- **Phase:** 2
- **Status:** done
- **Depends on:** T042

## Objective

Make the "discard them and continue" button replay *the* action the core
refused. It replayed a different one, and usually none at all.

## Motivation

`files.delete` refuses a delete over a dirty open buffer with `Conflict` and
`details.dirtyDocuments`, and offers `force` as the escape — hard rule 7, via
`DocumentTreeGuard::dirty_under`. The explorer had the replay for it,
`deleteAnyway`, and never called it:

```
use-explorer.ts:330:  const deleteAnyway = useCallback(async () => {   ← definition
use-explorer.ts:398:    deleteAnyway,                                   ← returned
```

Two occurrences in the whole repository, and no third. The one `Conflict`
notice in the UI (`explorer-panel.tsx:131-137`) was unconditional on which
command produced the conflict and wired `discardAndRetry`, the *open/close*
replay, which reads a different `useRef`.

The user-visible failure had two shapes, and the milder one is why nobody saw
it: the open/close slot is cleared on success, so at the moment a delete is
refused it is normally `null` and the button hits `if (!last) return` and does
**nothing**. Had the user previously had an open or close refused, the button
would instead have replayed *that* — re-opening a folder when they asked to
delete a file.

`T102-U` requirement 5 is the same `Conflict`-then-`force` pattern for
`git.discard`, so this is fixed before it is copied.

## Required reading

- `crates/gritty-core/src/workspace/files.rs:521-535` — the refusal and its
  `force` escape
- `docs/tasks/T102-U-git-write-ui.md` — "A bug to fix first, not to copy"

## Allowed scope

- `apps/desktop/src/features/explorer/**`

## Explicit non-goals

- No change to the wire, the core, or `files.delete`'s semantics.
- No new dialog. The notice's wording already covers all three cases.
- Not a general "undo the last refusal" history — one outstanding refusal.

## Required behavior

1. **One refusal slot, not one per command.** The notice is a single button, so
   it has to replay *the* refusal; two slots meant it had to guess which, and
   the guess was the defect. `Refusal` is a discriminated union over the three
   commands that can earn a dirty-document `Conflict` — `workspace.open`,
   `workspace.close`, `files.delete` — carrying the arguments each replay needs.
2. Every one of those actions writes the slot before it calls and clears it on
   success, so last-writer-wins means "what the core most recently refused".
3. `discardAndRetry` switches on the kind and replays with `force`. It is
   declared after every action it calls.
4. `deleteAnyway` is gone from the hook's returned surface; nothing called it.

## Failure behavior

- No outstanding refusal → the replay does nothing, silently. It is reachable
  only from a notice that a `Conflict` rendered, so this is defence, not a path.
- A replay that is itself refused leaves the slot set, so the notice stays
  truthful and the button stays live.
- A *succeeded* action clears the slot: replaying it would run a second delete
  from a button the user reached for another reason.

## Acceptance criteria

- A delete refused over a dirty buffer, then "discard and continue", issues
  `files.delete` again with the same paths and mode and `force: true`.
- The same for a refused open, keeping its path.
- `pnpm check` green.

## Tests required

- The replay for each kind, through the hook (the refusal is a `useRef` and the
  replay is a callback — neither is reducer state, and the defect was in the
  wiring between them).
- Both refused → the more recent one replays.
- No refusal, and a succeeded action, both replay nothing.

## Commands to run

```
pnpm exec vitest run apps/desktop/src/features/explorer
pnpm check
```

## Completion report

**Summary.** The two `useRef` slots became one, typed `Refusal`. The bug is now
structurally impossible rather than routed around: there is no second slot for
the notice to pick wrongly, and no second replay to leave uncalled.

**Files changed.**

- `apps/desktop/src/features/explorer/use-explorer.ts` — added the `Refusal`
  union; `refused` holds all three kinds; `refusedDelete` and `deleteAnyway`
  deleted; `deleteEntries` writes the shared slot; `discardAndRetry` moved
  below `deleteEntries` (it calls it, so the old position was a TDZ error) and
  gained the delete arm.
- `apps/desktop/src/features/explorer/use-explorer-force-replay.dom.test.tsx`
  — new, 5 tests.

`explorer-panel.tsx` is **unchanged**: the fix is at the hook, and the call
site was already `discardAndRetry`. That is the argument for fixing it here
rather than by teaching the panel which slot to read — the panel should not
know the hook has slots.

**Tests actually executed.**

- `pnpm exec vitest run apps/desktop/src/features/explorer` — **7 files, 84
  tests, all passed.**
- `pnpm typecheck` — clean across all four projects.
- `pnpm exec biome check apps/desktop/src/features/explorer` — 14 files, no
  findings.
- **The regression test was verified to fail without the fix**: with the delete
  arm of `discardAndRetry` stubbed back out, 2 of the 5 fail and 3 pass. A
  regression test that passes on the unfixed code proves nothing, so this was
  run rather than assumed (rule 12).

**Architectural decisions.**

- *One slot rather than a router.* The alternative — keep both refs and have
  `discardAndRetry` consult `refusedDelete` first — fixes the symptom and
  leaves the ambiguity. With one slot, "the refusal" is a single fact and the
  ordering question ("which wins when both are set?") stops existing.
- *Last-writer-wins is the semantics, not an accident.* Documented on the ref,
  because it is the one thing a reader would otherwise have to derive.

**Known limitations.** The notice's wording ("Documents have unsaved changes.
Save or close them first, or discard them and continue") is generic across all
three kinds. It is accurate for each, but a delete could say what it will
delete. Not changed here: `T102-U` requirement 4 is about to establish how this
project words a destructive confirmation, and this should follow that rather
than pre-empt it.

**Follow-up tasks.** None. `T102-U` may now copy this pattern.

**Remaining risks.** None identified. The change is local to one hook, has no
wire or persistence surface, and every caller of the removed export was
enumerated before removing it (there were none).
