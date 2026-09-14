# Task EXPLORER-RELIST-QUEUE — Stale directories re-list a few at a time

- **Phase:** maintenance (an ADR-025 follow-up)
- **Status:** review
- **Depends on:** T070, FILES-PARTIAL (a move or delete that stops partway settles what it did)

## Objective

When many loaded directories go stale at once, the explorer re-lists them
through a small, bounded number of concurrent `workspace.list` calls, shallow
directories first, instead of firing one call per directory in the same tick.

## Motivation

T070 recorded this as follow-up 9 and as a remaining risk
(`docs/tasks/completed/T070-file-watcher.md:465-468`, `:481-484`):

> The `overflowed` fan-out is not throttled. Every loaded node is re-listed in
> one pass; a user with many directories expanded gets that many
> `workspace.list` calls at once. Bounded and correct, but a queue would be
> kinder.

Two things make it worth doing now:

- **More callers.** FILES-PARTIAL added two more ways to mark every loaded
  node stale: a partial failure, and a files-kind operation that failed or was
  cancelled. `WATCH-DEGRADED`'s Refresh button will add a third.
- **The drain today** (`use-explorer.ts`, the effect after `load`) is
  `for (const path of stale) void load(workspace.id, path, null)`. A hundred
  expanded folders are a hundred concurrent IPC calls and a hundred blocking
  directory reads competing on the core's pool. That is the one place ADR-025's
  "a storm's cost is bounded by what the user expanded" is still proportional
  to the UI, all at once.

## Required reading

- ADR-025 §"The batch is capped" (the `overflowed` answer)
- `apps/desktop/src/features/explorer/use-explorer.ts` (`load`, the stale
  drain) and `explorer-reducer.ts` (`staleNodes`, `refreshing`)
- `apps/desktop/src/features/explorer/use-explorer-watch.dom.test.tsx`

## Allowed scope

- `apps/desktop/src/features/explorer/**`
- this file

## Explicit non-goals

- No core change, no wire change, and no debounce on top of the watcher's own.
- No prioritising of visible rows over offscreen ones. Depth order is enough to
  avoid listing a child its parent's re-list is about to remove.
- No cancelling of an in-flight listing that went stale again. It finishes, and
  the path is listed once more.

## Required behavior

1. **A concurrency cap.** At most `RELIST_CONCURRENCY` (4) re-lists from the
   stale queue are in flight at any time.
2. **Shallowest first.** Paths are taken by depth (`''` first, then by segment
   count), ties in the order they became stale.
3. **Each taken path leaves `stale` as it starts** (`refreshing`), so a batch
   naming it again while it is in flight puts it back and lists it once more
   after, never dropped and never listed twice at once.
4. **A slot frees on every outcome.** A listing that completes, fails, or turns
   into a pending operation frees its slot. A pending listing's operation is
   already tracked by the node, so the queue does not wait for it.
5. **A workspace change abandons the queue.** Nothing is listed for a
   workspace that is no longer open.

## Failure behavior

| Situation | Behavior |
|---|---|
| A listing fails | Error on the node as today; the slot frees; the queue continues |
| A listing becomes a pending operation | Slot frees at the pending response |
| Workspace closed mid-drain | Remaining paths are dropped with the state; in-flight answers are ignored by the reducer |
| A stale path is removed from the tree by its parent's re-list | Not listed; `staleNodes` only names loaded nodes, and the taken set is re-checked |

## Acceptance criteria

- `overflowed` with 10 loaded directories never has more than 4
  `workspace_list` calls in flight, and lists each of the 10 exactly once.
- A batch naming an in-flight directory lists it again after it settles.
- The existing watcher tests still pass.
- `pnpm check` green.

## Tests required

- `use-explorer-watch.dom.test.tsx`, with a transport whose listings resolve
  only when the test releases them:
  - the in-flight maximum and once-each;
  - the re-stale-while-in-flight case;
  - root before children.
- **Mutation check:** set the cap to `Infinity` and watch the in-flight
  assertion fail.
- **Manual, for Alexis:** expand a dozen folders, then `git checkout` a distant
  branch. The tree settles without a visible stall.

## Commands to run

```
pnpm vitest run apps/desktop/src/features/explorer
pnpm check
```

## Completion report

### Summary

The explorer's stale-directory drain is a bounded queue:

- at most `RELIST_CONCURRENCY` (4) `workspace.list` calls from it are in flight;
- the root and shallow directories go first;
- each settled listing frees a slot;
- a directory named again while its listing is in flight waits for that
  listing, then lists once more.

### Files changed

| File | What |
|---|---|
| `apps/desktop/src/features/explorer/use-explorer.ts` | `RELIST_CONCURRENCY`, `shallowestFirst`; the drain effect takes up to the free slots, skips in-flight paths, frees a slot in `load(...).finally`; a per-workspace in-flight set |
| `apps/desktop/src/features/explorer/use-explorer-watch.dom.test.tsx` | 3 tests over a transport that holds listings until released |

### Tests actually executed

| Command | Result |
|---|---|
| `pnpm vitest run apps/desktop/src/features/explorer` | exit 0, 9 files, 118 tests |
| `pnpm check:ts` (Biome, `tsc` in all packages, vitest) | exit 0, 129 files, 1432 tests |
| `check:rust`, `check:generated` | Not run for this ticket: it changes no Rust and no wire type. TRASH-FALLBACK, committed just before it, ran them |

**Mutation checks**, each run and then restored:

| Mutation | Fails |
|---|---|
| `RELIST_CONCURRENCY = Infinity` | `never runs more than four at once, and lists each directory once` |
| No depth order | `lists the root before any folder under it, whatever order the batch named them in` |
| In-flight paths not skipped | `lists a directory again when a batch names it while its listing is in flight` |

The first draft of the root-first test used an `overflowed` batch, whose order
already starts at the root, so it could not fail. It now names `d3, d1, ''`.

### Architectural decisions

- **Skipping in-flight paths also covers StrictMode's double effect.** The old
  identity guard on `stale` is gone. A second run for the same state finds its
  paths in flight and takes nothing.
- **The in-flight set is replaced when the workspace changes,** so a listing
  still settling for a closed workspace frees its slot in the set it was
  counted in and never holds the new workspace's slots.
- **Four** is a judgement, not a measurement. It is enough that a handful of
  expanded folders refresh together, and small enough that a checkout with a
  hundred expanded does not become a hundred concurrent reads. It is a named
  constant.

### Known limitations

- A listing that becomes a pending operation frees its slot when the pending
  response arrives, not when the operation finishes. A very slow tree could
  have more than four listings running core-side, each bounded by the
  operation model.
- No manual pass. The storm-then-settle behaviour is asserted with held
  listings, not observed in the app.

### Follow-up tasks

- None found.
