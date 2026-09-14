# ADR-028: Editing the file tree is a `files.*` command domain, and the Trash is the recovery

- **Status:** Accepted (implemented by T042)
- **Date:** 2026-09-08
- **Accepted:** 2026-09-14 by Alexis, as drafted. Checked against the tree that
  day: `trash` 5.2.8, the 32-entry journal (`workspace/journal.rs:21`),
  `rehome` announcing `MOVED`, `dragDropEnabled: true`, and a capability file
  that grants the WebView no filesystem access. One *Revisit when* trigger has
  moved closer: ADR-018A (accepted the same day) lets the hook refuse an
  *agent's* write into another workstream's lease once T330 ships, and says
  nothing about a *user's* `files.*` mutation there. That question is still
  this ADR's to answer when T330 lands.
- **Amends:** ADR-011 §"identity" — a document's path may now change without the
  document closing

## Context

Gritty can list a directory and cannot change one. `documents.save` is the only
filesystem-mutating command in the product, and `fs::rename` appears exactly once
in the tree as the last step of the atomic save. Every other editor lets you
create, rename, move and delete from its tree; a user of Gritty leaves Gritty to
do it.

Three constraints shape the answer.

**Path identity is already decided.** ADR-010 fixes the workspace root, the
relative `/`-separated wire path, and the lexical-then-physical resolution that
rejects a symlink escape. A mutation is a new *verb* over that representation,
not a new representation.

**Document identity is workspace id plus path** (ADR-011), keyed as
`by_path: HashMap<(WorkspaceId, WorkspacePath), DocumentId>`. Renaming a file
under an open tab therefore changes that tab's identity, and nothing in the
system can currently express that.

**The watch cannot report a rename.** ADR-025 fixed `WorkspaceEntriesChanged` as
a coalesced set of *directories*, deliberately, because FSEvents coalesces by
directory and "says nothing useful about which entry moved across a rename". A
rename is two changed directories, or one, and the renderer cannot tell the
difference. So the fact that `src/a.ts` became `src/b.ts` can only ever come from
the command that did it.

## Decision

- **The commands are a new `files.*` domain**, not verbs on `workspace.*`.
  `workspace.*` is lifecycle and listing; `documents.*` is buffers. A mutation is
  neither, and `workspace.delete` would read as "delete the workspace" — a name
  whose worst misreading is destructive. `files.create`, `files.rename`,
  `files.move`, `files.delete`, `files.undo`, `files.redo`. Protocol 2.20.0,
  additive.

- **A delete moves to the OS Trash by default; permanent deletion is a separate,
  confirmed gesture.** The priority order is correctness → recoverability →
  inspectability, and a mis-clicked delete that cannot be undone loses user data
  for an ergonomic saving of one keystroke. Shift+Delete deletes permanently
  behind a dialog that names what will go. This adds the `trash` crate, kept
  behind a small adapter inside `gritty-workspace` like every other
  platform-touching dependency (rule 11).

- **Undo covers create, rename, move, copy and paste. It does not cover a
  delete.** The `trash` crate's restore surface lives in `os_limited`, gated —
  checked against the crate source, `trash-5.2.8/src/lib.rs:346-350`, not
  assumed — as `#[cfg(any(target_os = "windows", all(unix, not(target_os =
  "macos"), not(target_os = "ios"), not(target_os = "android"))))]`. So `list`,
  `restore_all` and `purge_all` do not exist on macOS, which is the platform
  Gritty actually runs on (ADR-002). The available options were to lie, to build
  a Gritty-managed staging area that would become a second trash the user does
  not know about, or to say so. The Trash *is* the recovery for a delete, and the
  UI says that in place of a disabled ⌘Z. Rule 12: a recovery path that has not
  been tested is not claimed.

- **The journal is in the core, bounded, and not persisted.** It is a 32-entry
  stack per open workspace holding each operation's inverse, dropped when the
  workspace closes. In the core because the core owns the filesystem and is the
  only layer that can re-validate a precondition; bounded because an unbounded
  undo history of filesystem inverses is a memory leak with a user-facing name;
  not persisted because surviving a restart would make ⌘Z reach across a session
  boundary the user cannot see, and because ADR-014's database already holds
  buffer text and does not need filesystem inverses too.

- **Undo re-validates and refuses rather than overwrites.** Undoing a rename when
  something else now occupies the old name is `Conflict`, not a clobber. This is
  hard rule 7 — never silently resolve a conflict — applied to the one operation
  whose whole purpose is to move backwards.

- **The response carries the directories to re-list; the watch is not the
  contract.** A mutation returns the affected directories and the renderer marks
  them stale through the same path a watcher batch uses. The watch is
  best-effort — `workspace.md` records that a backend which refuses to start
  still opens the workspace — so a tree that only converges when FSEvents
  happened to be running is not a contract. The watcher's own batch arrives
  200 ms–1 s later and is idempotent. This adds **no** `CoreEvent` variant.

- **A rename re-keys open documents instead of closing them.** `rehome` rewrites
  the path of every open document at or under the old path, keeps buffer,
  versions, `savedHash` and dirty state, and emits `DocumentChanged { cause:
  MOVED }`. The alternative — close and reopen — would discard an unsaved buffer
  on a rename, which is the same data loss ADR-011 exists to prevent. This is the
  amendment to ADR-011 named above: a document's path is mutable by exactly one
  command, and the change is announced.

- **Deleting something with a dirty document under it is `Conflict` unless
  forced. Moving one is not.** The same shape `workspace.close` and a replacing
  `workspace.open` already use, with `details.dirtyDocuments`; the UI offers the
  choice and never picks one. The asymmetry is the point: a move is safe because
  `rehome` carries the buffer to the new path, so refusing it would be a prompt
  with no danger behind it. A delete is the one operation here that can lose
  unsaved work, because nothing carries it anywhere.

- **A copy invents a free name; a move never does.** Dropping `a.ts` where an
  `a.ts` already lives copies it as `a copy.ts`, the way every file manager
  does. A *move* onto a taken name is `Conflict` instead: silently landing as
  `a copy.ts` would be a rename nobody asked for, and — found while building
  this — dragging a file onto the folder it already lives in would have done
  exactly that. That case is now `InvalidInput`.

- **Undo's `Remove` step tolerates an entry that is already gone.** Undoing a
  create whose file something else deleted is that step's goal already reached,
  not a failure. This is not a hole in the rule above: removing nothing
  overwrites nothing, so it cannot lose work. Every other step still refuses
  when its precondition moved.

- **A recursive copy or delete that outlives its budget becomes a cancellable
  operation** — the hand-over `workspace.list` already makes (rule 10).
  **Cancellation leaves partial state**: it stops before the next entry, what was
  already written stays, and the response names it. There is no rollback, and the
  message says so rather than implying an atomicity the filesystem does not offer.

- **Drag-and-drop inside the tree uses HTML5 drag events, and `dragDropEnabled`
  stays `true`.** Tauri's own documentation for that flag reads "Disabling it is
  required to use HTML5 drag and drop on the frontend **on Windows**" — macOS,
  the only supported platform (ADR-002), is unaffected. Windows is Phase 8 and
  inherits this as a known cost, recorded here rather than discovered then.

## Consequences

- The WebView gains no filesystem capability. `capabilities/default.json` is
  unchanged, and every mutation resolves through the same `resolve_for_write`
  that `documents.save` uses, so a symlink escaping the root is
  `PermissionDenied` before anything is written.
- The explorer becomes a real tree widget — `role="tree"`, roving `tabIndex`,
  arrow navigation, multi-select — which T020 and T030 both deferred and neither
  built. That is a prerequisite here, not a bonus.
- `applyPage` currently drops a node's whole subtree on every non-append page, so
  a re-list collapses what the user expanded. Tolerable while re-lists were rare;
  every mutation now causes one, so `expanded` must survive a re-list.
- A delete cannot be undone from inside Gritty on macOS. This is the sharpest
  consequence and it is deliberate; if the `trash` crate gains a macOS restore,
  or Gritty ever ships on a platform that has one, revisit.
- `files.move` with `COPY` on a large tree can run for a long time. It is
  cancellable, and it is not atomic.

## Revisit when

- `trash` gains a macOS restore, or a platform with one becomes supported —
  undo of a delete becomes possible and the UI's message is wrong.
- Gritty targets Windows: `dragDropEnabled` and the in-tree drag collide, and
  either the flag becomes conditional or the tree moves off HTML5 drag.
- T330 makes ownership enforced rather than observed: a user-initiated mutation
  inside another workstream's lease becomes a question this ADR does not answer.
- Anything needs undo to survive a restart — that is a persistence design, and it
  needs ADR-014's database and its own decision.
