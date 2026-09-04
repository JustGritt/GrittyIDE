# Task T045 — Drag and drop files and folders onto the window

- **Phase:** 1A
- **Status:** in-progress
- **Depends on:** T020 (workspace), T030 (documents), T040 (editor shell)

> The ID is an addition to the project plan's §17 numbering, not from it. It
> sits between T040 and T050 because it is a T040-family editor-shell feature:
> the plan predates the request.

## Objective

Dropping a file onto the Gritty window opens it in the editor; dropping a
folder opens it as the workspace. The gesture every other IDE has, with
Gritty's rules kept intact.

## Dependencies

`workspace.open` and `documents.open` already do the work. This task adds the
gesture, not a new way to open things.

## Required reading

- `AGENTS.md` rules 1 (typed protocol only), 8 (no unbounded event forwarding),
  13 (repository content is untrusted input)
- ADR-010 (workspace paths), ADR-011 (document/buffer contract) — why a file
  outside the workspace has no identity
- `docs/architecture/documents.md`, `crates/workspace/src/root.rs`

## Allowed scope

- `crates/workspace/src/root.rs` (a path helper)
- `crates/gritty-core/src/file_drop.rs` (new), `event/`, `workspace/`, `protocol/`
- `apps/desktop/src-tauri/src/file_drop.rs` (new), `lib.rs`, `core.rs`, `tauri.conf.json`
- `apps/desktop/src/features/drop/` (new), `App.tsx`, `app.css`
- `packages/protocol/src/types.ts`

## Explicit non-goals

- **No loose files.** Opening a file with no workspace would change document
  identity, which is frozen after Phase 0 and would need an ADR.
- **No multi-root workspaces.**
- No dragging *out* of Gritty, no drop onto a specific tab or split, no reorder
  semantics beyond what the tab strip already has.
- No repository-root detection (walking up to `.git`): that is T100's.

## Required behavior

1. A dropped **directory** becomes the workspace. Several at once: the first,
   and the rest are named as skipped.
2. A dropped **file inside the open workspace** opens as a tab. Up to 20 per
   gesture; beyond that the remainder is reported, not opened.
3. A dropped **file outside** any open workspace is never opened silently and
   never silently swaps the workspace. The UI offers to open its folder.
4. A folder and files in the same gesture: the folder wins, and the files are
   reported as skipped — their workspace-relative identity would change under
   them the moment the root changed.
5. Dropping a folder while documents are dirty is refused with `Conflict`, and
   the UI offers "discard changes and open" — the same rule the picker follows.
6. A drop target is visible while a drag is over the window.
7. Every dropped path produces exactly one classified item: nothing disappears
   without being named.

## Failure behavior

- A path that no longer resolves between drag and drop, or that is not a
  regular file or directory, is classified and reported, never opened.
- Symlinks are canonicalized before being judged, so a link outside the
  workspace pointing inside it opens the target.
- A component that is not valid UTF-8 yields no wire path rather than a lossy
  one that would not round-trip.
- Classification stats every path, which is filesystem I/O; it runs off the UI
  thread so a dropped directory tree cannot freeze the window.

## Acceptance criteria

- Dropping `fixtures/small-repo` opens it; dropping `src/index.ts` from it
  opens a tab; dropping `~/Downloads/whatever.md` offers `~/Downloads`.
- The tab strip's own drag-to-reorder still works with OS drops enabled.
- No path reaches the WebView except through the core protocol.

## Tests required

- Core: classification of inside/outside/directory/missing/unsupported/symlink,
  order and one-item-per-path preserved, and the no-workspace case.
- `crates/workspace`: `relativize` round-trips `to_native` and refuses paths
  outside the root.
- Renderer: `planDrop` rules, including the folder-wins rule, the cap, and that
  an outside file never sets `openWorkspace`.
- Manual: `docs/testing/phase-0-manual-pass.md` steps 2.13 and 2.14.

## Commands to run

`pnpm check`, then the app: drop a file, a folder, and a file from outside.

## Completion report

(filled at review)
