# Task T042 — Editing the file tree: create, rename, move, delete, undo

- **Phase:** 1A
- **Status:** done
- **Depends on:** T020 (`WorkspaceRoot`, `WorkspacePath`, the path policy every
  mutation resolves through), T030 (the document/buffer contract, Gate B — a
  rename re-keys an open document rather than orphaning it), T040 (the tab strip
  a renamed document's tab belongs to), T070 (the watch whose re-list this task
  triggers deliberately rather than waiting for)

> The ID is an addition to the project plan's §17 numbering, not from it, the
> same way T041 and T045 are. It sits in the T040 family because the explorer is
> where it is used, but its weight is in `crates/workspace` and
> `gritty_core::workspace`: §17's T020 deliverable is "lazy directory listing",
> read-only by construction, and nothing downstream ever added the write half.

## Objective

The tree lists files and cannot change one. This task gives it the operations
every other editor has — new file and folder, rename, delete to the Trash,
cut/copy/paste, duplicate, drag-to-move, multi-select and undo — through new
`files.*` core commands, with Gritty's path, conflict and recovery rules intact.

## Motivation

`documents.save` is the only filesystem-mutating command in the product.
`fs::rename` appears exactly once in the entire tree
(`crates/workspace/src/file.rs:272`) and it is the last step of the atomic save.
`crates/workspace/src/` is `detect / error / file / fixtures / lib / listing /
path / root / watch` — there is no mutation module, and no command anywhere
creates, renames, moves, copies or removes an entry. A user who wants to rename
a file leaves Gritty to do it.

Two facts make this more than a set of adapters over `std::fs`:

**The explorer has no selection.** `ExplorerNode` carries `expanded`, `loading`,
`children`, `cursor` and `error`, and nothing about what the user has picked.
`explorer-panel.tsx` has no `onKeyDown`, no `tabIndex` and no `role="tree"` — it
is the one interactive surface in the application with no keyboard handling at
all. Every operation here needs an answer to "which rows is this about", so the
tree widget is a prerequisite, not a decoration. T020 and T030 both logged it as
a follow-up and neither picked it up
(`docs/tasks/completed/T020-workspace-open-list.md:241`).

**A rename would orphan an open document.** Document identity is workspace id
plus relative path — `Table::by_path: HashMap<(WorkspaceId, WorkspacePath),
DocumentId>` in `crates/gritty-core/src/document/mod.rs:440-441`. ADR-025 fixes
the watch as directory-grained precisely because FSEvents cannot say which entry
moved, so no event can ever report "X became Y". Left alone, renaming a file
from inside Gritty would leave its tab pointing at a path that no longer exists,
and `documents.status` would report `DELETED_EXTERNALLY` for a file the user did
not delete. The command knows both paths; it must re-key the document itself.

## Dependencies

- **T020** for `WorkspaceRoot::resolve_for_write` (`crates/workspace/src/root.rs:108`),
  `WorkspaceRoot::resolve` (`:91`), `WorkspacePath` and its `Deserialize` impl,
  which rejects traversal before a handler runs.
- **T030** for `is_temp_file_name` (`crates/workspace/src/file.rs:283`), the
  dirty/`Conflict`/`force` idiom, and `DocumentManager`'s table.
- **T070** for `WorkspaceEntriesChanged` and the `stale` → re-list path in
  `explorer-reducer.ts`, which this task reuses rather than duplicating.

## Required reading

- `AGENTS.md` hard rules 1 (typed protocol only), 7 (never silently resolve
  dirty-buffer versus external-file conflicts), 10 (long work uses the operation
  model), 11 (platform code behind an abstraction), 12 (no unverified recovery
  claims), 14 (do not broaden scope).
- ADR-010 (workspace root and path representation), ADR-011 (document/buffer
  contract — the state matrix a delete moves a document into), ADR-025
  (filesystem watching; §"a rename inside a watched directory is two changed
  directories, or one, and the renderer cannot tell the difference").
- `docs/architecture/workspace.md` (layers, listing, the watch's four bounds,
  the failure table), `docs/architecture/documents.md` (state matrix, atomic
  write, `DocumentChangeCause`), `docs/architecture/security.md:74-99` (writes
  only through resolved paths; no silent overwrite).
- `crates/gritty-core/src/workspace/mod.rs:382-450` — `WorkspaceManager::list`,
  the budget hand-over an operation-backed copy or delete must copy verbatim;
  `:565` — `impl From<WorkspaceError> for CoreError`, which gains arms.
- `crates/workspace/src/root.rs:91-140` and `crates/workspace/src/path.rs:73`
  (`join` accepts a single segment only — a create takes a whole `WorkspacePath`).
- `apps/desktop/src/features/explorer/explorer-reducer.ts` (the flat
  `Record<path, node>`, `dropSubtree` at `:161`, `applyPage` at `:168`) and
  `use-explorer.ts` (the `run()` wrapper, the `refused`/`discardAndRetry` replay).
- The three UI primitives to reuse rather than reinvent:
  `apps/desktop/src/components/menu.tsx`, the inline-rename pattern in
  `apps/desktop/src/features/terminal/terminal-pill.tsx`, the destructive dialog
  in `apps/desktop/src/features/editor/close-dialog.tsx`, and the HTML5 drag
  idiom in `apps/desktop/src/features/editor/tab-strip.tsx:10` (`TAB_MIME`).
- `docs/tasks/completed/JSDOM-VITEST-component-tests.md` § "Architectural decisions"
  before writing any `.dom.test.tsx`.

## Allowed scope

Lane W's paths, plus the wire and the two shell features that consume it:

```
crates/workspace/**
crates/gritty-core/src/workspace/**
crates/gritty-core/src/document/**
crates/gritty-core/src/{shell.rs,protocol/{mod,export}.rs}
apps/desktop/src-tauri/src/{commands,lib}.rs
apps/desktop/src/features/explorer/**
apps/desktop/src/features/documents/**          (the MOVED cause only)
apps/desktop/src/{App.tsx,app.css}              (shared magnets: claim, edit, release)
packages/protocol/src/{client,commands,types}.ts + generated
Cargo.toml                                       (the `trash` dependency)
```

`packages/protocol/**` and `apps/desktop/src-tauri/src/**` are `PROTO-W3`'s lease
while that ticket is in flight. T042 is therefore built in a worktree and the two
must not be open at the same time; whichever lands second rebases.

## Explicit non-goals

- **No ownership check.** `OwnershipManager::check` (T325) observes and does not
  enforce, and whether a *user-initiated* mutation should consult a workstream's
  lease is undecided anywhere in the repository. A user editing their own tree is
  not an agent writing into someone else's lane. Follow-up, not this task.
- **No dragging out of Gritty, and no drop from the OS onto a specific folder.**
  T045 owns the window-level drop; changing where a dropped file lands is a
  change to its contract.
- **No multi-root workspaces**, no operations outside the root, no `workspace.outsideRoot`
  capability.
- **No persisted undo.** The journal lives as long as the workspace is open. A
  restart is not an undo boundary the user can see, and persisting filesystem
  inverses across a crash is a recovery claim this task will not have tested.
- **No file-content operations** — no templates, no scaffolding, no "new file
  from…".
- **No `git mv` awareness.** T100 reads the repository; a rename here is a
  filesystem rename and Git will detect it as it detects any other.

## Required behavior

1. **Create.** `files.create { workspaceId, path, kind }` takes a whole
   `WorkspacePath`, so `a/b/c.ts` creates the intermediate directories. An
   existing target is `Conflict` — never a silent truncate.
2. **Rename** renames in place and **re-keys every open document at or under the
   old path**, keeping buffer, `currentVersion`, `savedRevision`, `savedHash` and
   dirty state, and emitting `DocumentChanged { cause: MOVED }`.
3. **Move and copy** are one command over N paths with a target directory.
   Dragging onto a folder moves; Option-drag, and paste after copy, copies. A
   **copy** onto a taken name becomes `name copy.ext`, `name copy 2.ext`; a
   **move** onto a taken name is `Conflict`, and a move into the directory the
   entry already lives in is `InvalidInput`. Never an overwrite, and never a
   rename nobody asked for.
4. **Delete** moves to the OS Trash by default. Shift+Delete deletes permanently,
   behind a confirmation that names what will go. Both take N paths.
5. **Deleting something with a dirty document under it is `Conflict`** with
   `details.dirtyDocuments`, unless the request carries `force`. The UI offers
   the choice and never picks one. A **move** is not refused: `rehome` carries
   the buffer to the new path, so there is nothing to lose.
6. **Undo** (`⌘Z`) reverses create, rename, move, copy and paste, and refuses
   with `Conflict` when the world moved under it rather than overwriting what
   appeared. **A delete is not undoable in-app** — the Trash is where it went and
   the UI says so. Redo is `⇧⌘Z`.
7. **Every mutation response carries the directories to re-list**, and the
   renderer marks them stale through the same path a watcher batch uses. The
   watch is best-effort (`workspace.md` § Failure: a backend that refuses to
   start still opens the workspace), so convergence cannot depend on it.
8. **A recursive copy or delete that outlives its budget becomes a cancellable
   operation**, the same hand-over `workspace.list` makes.
9. **The tree is a tree.** `role="tree"` / `role="treeitem"`, `aria-level`,
   `aria-selected`, `aria-multiselectable`, a roving `tabIndex`, arrow-key
   navigation, Home/End, typeahead, and Cmd/Shift multi-select.
10. **Copy Path, Copy Relative Path, Duplicate and Reveal in Finder** are on the
    context menu. The first two need no command — `Workspace.root` is already on
    the wire.

## Failure behavior

| Situation | Behaviour |
|---|---|
| A path resolves outside the root through a symlink | `PermissionDenied` before anything is written, from the existing `resolve_for_write` check |
| Target exists on create | `Conflict`; the inline input keeps focus and says so |
| Target exists on move | The copy is renamed (`name copy.ext`); a *move* onto an existing name is `Conflict` |
| A directory is moved into its own descendant | `InvalidInput` — checked lexically before any syscall |
| `fs::rename` across devices | Copy-then-delete fallback; a failure part-way leaves the source intact and names what was copied |
| A recursive copy is cancelled | Stops before the next entry. What was already written **stays**, and the response names it. Nothing is rolled back and nothing pretends it was |
| A recursive delete is cancelled | Same: partial, and named |
| Trash is unavailable (network or removable volume) | `Io` with the reason; the UI offers permanent deletion rather than failing silently |
| An open document is dirty under a delete or move | `Conflict` with `details.dirtyDocuments`; `force` proceeds |
| An open document's file is deleted | It becomes `DELETED_EXTERNALLY` — the existing state, the existing banner, no new machinery |
| Undo's precondition moved | `Conflict`; the journal entry is kept, not silently dropped |
| The workspace closes | The journal is dropped with it. No undo ever names a workspace that is gone |
| A name that is not valid UTF-8 in a component | Refused at the boundary, as `WorkspacePath` already refuses it |

## Acceptance criteria

- In `fixtures/small-repo`: create `a/b/c.ts` in one action; rename `src/index.ts`
  while its tab is open **and dirty**, and the tab follows with the buffer intact;
  drag `docs/` onto `src/`; ⌘X/⌘V a file across directories; Delete a folder and
  find it in Finder's Trash; Shift+Delete and confirm it is gone; ⌘Z after each of
  the undoable ones.
- Arrow keys, Home/End, F2, Delete, ⌘X/C/V and typeahead work with the mouse never
  touched, and a screen reader reads level and selection.
- No path reaches the WebView except through the core protocol, and
  `capabilities/default.json` is unchanged.
- The tab strip's own drag-to-reorder still works while the tree's drag is armed.
- `pnpm check` green.

## Tests required

- `crates/workspace`: symlink escape, create onto an existing name, rename onto an
  existing name, a directory moved into its own descendant, the cross-device
  fallback, a permission failure mid-recursive-copy leaving a named partial
  result, and refusal to operate on a `.gritty-*.tmp` name. Against
  `fixtures/weird-files`, which already has the spaces, Unicode, symlinks, the
  mode-000 file and the 10-level path.
- `gritty-core`: one test per command; the dirty-document `Conflict` and its
  `force`; `rehome` keeping version and dirty state and re-keying `by_path`; the
  journal's refuse-if-changed rule and its bound; cancellation of a recursive copy.
- `apps/desktop/src-tauri/tests/commands.rs`: the traversal/absolute/backslash
  table, an unknown workspace id, a `NotFound` target — the shape
  `workspace_list_rejects_traversal_unknown_ids_and_bad_pages` already has.
- `packages/protocol`: a `files-client.test.ts` over `createFakeTransport()`.
- `apps/desktop`: pure unit tests for the selection algebra, the keymap and name
  validation/collision naming; reducer tests for the new actions and for
  `expanded` surviving a re-list; one `.dom.test.tsx` for the hook.
- Manual, in the running app: everything under Acceptance criteria. Nothing about
  the drag gesture or the Trash is claimed from a unit test.

## Commands to run

`pnpm check`, then `pnpm dev` and the acceptance list above.

## Completion report

### Summary

The explorer can change the tree. `gritty_workspace::mutate` does the
filesystem work, `files.*` puts five commands and an undo journal on top of it,
the wire carries them, and the explorer became a real tree widget to drive them
— selection, keyboard, context menu, inline rename, drag-to-move.

Built in a worktree because `PROTO-W3` held the wire's files for the first half
of the session, then rebased onto it once it landed (`1ed3505`); the only real
conflict was `PROTOCOL_VERSION`, resolved to the 2.20.0 this task had claimed.

### Files changed

| File | What |
|---|---|
| `crates/workspace/src/mutate.rs` (new) | `create`, `rename`, `copy`, `remove`, `trash`, `exists`; the two resolution rules |
| `crates/workspace/src/error.rs`, `lib.rs`, `Cargo.toml`, root `Cargo.toml` | `AlreadyExists`, the module, the `trash` dependency |
| `crates/gritty-core/src/workspace/files.rs` (new) | five commands, wire types, `DocumentTreeGuard`, the transfer planner, the copy-name rule |
| `crates/gritty-core/src/workspace/journal.rs` (new) | the bounded undo/redo stacks, pure |
| `crates/gritty-core/src/document/mod.rs` | `DocumentChangeCause::Moved`, `rehome`, `dirty_under`, `relocate` |
| `crates/gritty-core/src/shell.rs` | `RevealPathRequest`, `reveal_path`, and `reveal` on `ExternalOpener` |
| `crates/gritty-core/src/protocol/{mod,export}.rs` | 2.20.0, eight new root types |
| the Tauri composition root (`commands`, `lib`, `core`, `shell`) | seven adapters, the handler table, `FileService` + its document guard, and the one platform branch |
| `packages/protocol/src/{commands,client,types}.ts` + generated | the `files` namespace and `shell.revealPath` |
| `apps/desktop/src/features/explorer/explorer-{names,selection,keymap}.ts` (new) | the three pure modules |
| `apps/desktop/src/features/explorer/explorer-tree.tsx` (new) | the tree widget: roles, roving `tabIndex`, menu, inline input, drag, dialog |
| `apps/desktop/src/features/explorer/{explorer-panel.tsx,explorer-reducer.ts,use-explorer.ts}` | the frame keeps the frame; selection state, `invalidate`, the `applyPage` fix; the six calls, the clipboard, undo labels |
| `apps/desktop/src/app.css` | selection, focus ring, cut, drop target, inline input |
| `docs/` | this file, ADR-028, `workspace.md`, `documents.md`, the ADR index, `merge-queue.md` |

### Tests executed

`pnpm check` — **exit 0**: Biome, tsc, vitest, `cargo fmt --check`,
`clippy -D warnings`, `cargo test`, `check:generated`.

- `gritty-workspace` (8): nested create and no truncate; the root, a temp name,
  a file-as-parent; rename onto an occupied name; a directory into its own
  descendant; recursive copy and what a cancellation left; a symlink moved,
  copied and removed **as itself**; a symlink escape refused with nothing
  outside touched; recursive remove.
- `gritty-core` (20): 6 journal, 14 `files` — the copy-name rule, dotfiles and
  double extensions, a separator refused, create/undo/redo, rename telling the
  document layer, the dirty-document conflict and its force, a delete never
  being undoable, a move never inventing a name, an undo refused when the
  destination was taken and still undoable after, an empty journal.
- The command adapters (1): every command through the real Tauri adapters on
  the mock runtime — the traversal/absolute/backslash table, an unknown
  workspace, a `NotFound` target, the `ready` arm of a move, and undo/redo
  walking the journal to empty and back.
- `packages/protocol` (6): the request shapes, both arms of a move, the
  dirty-document refusal surfacing as a `CoreError` that still names what is
  unsaved, and `force` omitted rather than sent as `undefined`.
- `apps/desktop` (46): name policy, selection algebra, keymap, the reducer's
  new actions, `expanded` surviving a re-list, a tab following a rename — and
  10 jsdom tests driving the real component (see below).

### Architectural decisions

In ADR-028. Four were made *while building* and changed the spec:

- **A move never invents a name.** The copy-suffix rule is copy-only. Written
  the other way it produced a real bug the tests caught: dragging a file onto
  the folder it already lived in silently renamed it to `main copy.rs`.
- **The dirty-document guard is delete-only.** A move is safe because `rehome`
  carries the buffer, so refusing it would be a prompt with no danger behind it.
- **Undo's `Remove` step is idempotent.** Undoing a create whose file something
  else deleted is that step's goal reached, and removing nothing loses nothing.
- **The documents layer needed no change.** `withDocument` already replaces the
  path without bumping `bufferGeneration`, so a renamed file's tab follows and
  Monaco keeps its undo stack. A test now pins that rather than leaving it
  accidental.

### Known limitations

- **Nothing has been clicked.** This machine has no screen or assistive access,
  so the manual pass in "Acceptance criteria" has **not** been performed. What
  stands in for it is `explorer-tree.dom.test.tsx`: ten tests through the real
  component and the real typed client over a stub transport. That covers the
  wiring; it does not cover what the thing looks like, whether the drag feels
  right, or whether an entry really lands in Finder's Trash.
- **`trash::delete` has never actually run.** Every delete test uses
  `PERMANENT`, deliberately, so the suite cannot fill a developer's Trash.
- **A delete cannot be undone on macOS.** Verified against the crate source:
  `trash-5.2.8/src/lib.rs:346-350` `#[cfg]`-excludes `os_limited`.
- **Cancelling a copy or delete leaves partial state**; `cancelledAfter` counts
  it and nothing is rolled back.
- **Only the macOS arm of `reveal` has been run.** Windows and Linux compile and
  are not claimed (rule 11).
- `files.move` plans every destination before moving, so a batch where an early
  move frees a name a later one wants will not notice.
- The Windows-reserved-name check warns rather than refuses (macOS-first).

### Follow-up tasks

- The manual pass: the acceptance list, performed by a human on a machine with a
  screen. Until then this is "tested", not "seen working".
- Whether a user-initiated mutation should consult a workstream's ownership
  lease (T325/T330) — deliberately out of scope, undecided anywhere in the repo.
- `docs/architecture/overview.md`'s command list is stale (it still says
  `PROTOCOL_VERSION` is "currently 1.3.0") and gains no `files.*` row here.
- A cross-device rename falls back to copy-then-remove; that branch has no test,
  because it needs a mount point inside a workspace root and no fixture has one.
- The explorer's `.dom.test.tsx` shims nothing for `CSS.escape` or
  `navigator.clipboard`; a jsdom without them would fail differently. The
  `JSDOM-VITEST` ticket's "move the shims to a shared setup file at the second
  consumer" is now due.
