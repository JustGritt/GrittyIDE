# Task T045 — Drag and drop files and folders onto the window

- **Phase:** 1A
- **Status:** doing
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

### Summary

Dropping a file the workspace contains opens it as a tab; dropping a folder
opens it as the workspace. The window receives the OS drop, the core
canonicalizes and classifies each path against the open workspace, one event
carries the result, and the renderer performs it with `workspace.open` and
`documents.open` — the commands that already existed. No new command.

The design turned on one existing rule: documents are keyed workspace root plus
relative path, so a file from outside any open workspace has no identity in
Gritty. Rather than inventing one, the UI offers to open its folder and waits.

### Files changed

- `crates/workspace/src/root.rs`: `WorkspaceRoot::relativize`, the inverse of
  `WorkspacePath::to_native`, for paths arriving from the platform rather than
  the protocol. Non-UTF-8 components yield `None` instead of a lossy path.
- `crates/gritty-core/src/file_drop.rs` (new): `DroppedKind`, `DroppedPath`,
  `classify`, and `FileDropService`.
- `crates/gritty-core/src/event/mod.rs`: `PathsDropped`, `DropHoverChanged`.
- `crates/gritty-core/src/session/tap.rs`: both events explicitly not persisted.
- `crates/gritty-core/src/workspace/mod.rs`: `WorkspaceManager::current_open`.
- `crates/gritty-core/src/protocol/mod.rs`: 1.4.0 → 1.5.0.
- `apps/desktop/src-tauri/src/file_drop.rs` (new), `lib.rs` (chained window
  event handlers), `core.rs` (`Core.file_drop`), `tauri.conf.json`
  (`dragDropEnabled` explicit).
- `apps/desktop/src/features/drop/` (new): `drop-policy.ts`, `use-file-drop.ts`,
  `drop-overlay.tsx`; `App.tsx`, `app.css`.
- `packages/protocol/src/types.ts` and the generated types.

### Tests executed

`pnpm check` green. New:

- `gritty-workspace` (2): `relativize` round-trips `to_native`; refuses paths
  outside the root, including a sibling sharing a string prefix.
- `gritty-core` (7): file inside, file outside, no workspace open, directories,
  a path that stopped resolving, order and one-item-per-path, and a symlink
  resolved before it is judged.
- `apps/desktop` (9): every `planDrop` rule — folder wins over files, one
  folder at a time, the 20-file cap, an outside file never setting
  `openWorkspace`, unresolvable paths reported not ignored.

### Architectural decisions

- **The drop is classified in the core, not the WebView.** Deciding whether a
  path is inside the workspace is path identity (ADR-010), and the WebView has
  no filesystem access to answer it. It also keeps the classification testable
  without a window.
- **The renderer never sees Tauri's own `tauri://drag-drop` events**, which
  would mean importing `@tauri-apps/api` outside `transport/tauri.ts` and
  breaking rule 1.
- **`Over` is never forwarded.** It fires continuously during a drag; enter and
  leave are all the overlay needs (rule 8).
- **Classification runs on the blocking pool.** It stats every path, and the
  window event handler is on the UI thread.
- **Neither event is persisted.** A drop is a gesture; what it leads to is
  persisted by the events for those things. `Intent::from_event` is exhaustive
  on purpose, so this had to be stated rather than defaulted.
- **A folder wins over files in the same gesture**, because the files'
  workspace-relative identity would change under them the moment the root did.

### Known limitations

- **The drop gesture itself has never been performed.** Classification and the
  plan are tested; nothing has dragged a file onto the window. Manual pass
  steps 2.13–2.18.
- Whether OS file drops break the tab strip's HTML5 drag is still unknown —
  T040 raised it, step 2.18 decides it.
- `use-file-drop` is untested, following the existing convention: the decision
  lives in a pure tested module and the hook that calls `core` is thin, the
  same shape as `use-explorer`.
- No loose files, no multi-root, no drop onto a specific tab or split.

### Follow-up tasks

- T100 could offer the file's *repository* root rather than its immediate
  parent, which is usually what someone dragging from a project means.
- If step 2.18 shows the two drag mechanisms conflict, either the tab strip
  moves off HTML5 drag or `dragDropEnabled` becomes conditional.
