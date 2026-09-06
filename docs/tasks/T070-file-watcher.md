# Task T070 — Watching the workspace turns two on-demand checks proactive

- **Phase:** 1 / 2
- **Status:** todo
- **Depends on:** T020 (workspace open/list, `WorkspaceRoot`, `WorkspacePath`),
  T030 (document/buffer contract, Gate B — the on-demand external-change check
  this task triggers early, never replaces)

## Objective

Nothing in Gritty observes the filesystem today. `DocumentManager` checks a
file's disk state only when asked (`documents.status`, before every save, on
reload — ADR-011), and the explorer lists a directory once and never refreshes
it unasked. This task adds a watcher, scoped to the open workspace, that makes
both checks proactive — an external edit noticed within a debounce window
instead of at the next save, a directory create/remove/rename under an
expanded explorer node refreshed unasked — without adding a second source of
truth for either fact; see "Required behavior."

## Motivation

Phase 0 deliberately left two rows blank. `docs/plans/phase-0-spike.md:16`
(A6, "Filesystem watching; rapid changes do not lock the UI") reads: "Deferred
to T070, deliberately… Becomes a T070 acceptance criterion." B5 (`:32`,
"Filesystem storm") is deferred for the same reason, pass condition "UI
responsive; events batched/bounded; final state converges." Both become
acceptance criteria below rather than being dropped.

No ADR exists yet for this design — unlike T097, whose ADR-017 was already a
draft to accept. The implementing session should write one (the next free
number after ADR-019) covering the crate/platform choice and the
ignore-subtree policy §9 fixes; treat this spec as a recommendation to accept
or revise, not a decision already made.

## Dependencies

- **T020** for `WorkspaceRoot::path()` (`crates/workspace/src/root.rs:70`),
  `WorkspacePath` (`Hash + Eq + Ord`, `crates/workspace/src/path.rs`) and
  `IGNORED_DIRECTORY_NAMES` (`crates/workspace/src/listing.rs:27`).
- **T030** for `is_temp_file_name` (`crates/workspace/src/file.rs:283`) and
  for the check this task makes proactive: ADR-011 (`docs/decisions/
  ADR-011-document-buffer-contract.md:44-51`) already promises
  "watcher-driven checks arrive with T070 through the same path" — the
  existing `documents.status` command, not a new one.

## Required reading

- `docs/plans/phase-0-spike.md:16,32` (rows A6/B5, quoted above) and
  `docs/plans/execution-board.md:71-88` (Lane W's scope, reserved protocol
  minor **2.11**).
- ADR-011; `docs/architecture/documents.md` (state matrix; the stale line at
  `:39`, "it is not watched (T070)", to correct); `docs/architecture/
  security.md:16,48-53` (the symlink-escape, component-wise root check a
  watched path is held to as well).
- `docs/tasks/T080-search.md`, its Dependencies section — T080 needs T070
  for "a cheap, synchronous check… a directory walker can call per entry" and
  says it cannot reuse watcher events alone for that; §9 satisfies this.
- `crates/gritty-core/src/session/writer.rs:41-60` (`WriterConfig`'s
  `debounce`/`max_wait`, the shape to copy) and `crates/terminal/src/batch.rs`
  (bounded, drops the oldest, reports the drop rather than lying about
  completeness — hard rule 8 governs this task the same way).
- `apps/desktop/src/features/explorer/{use-explorer.ts,explorer-reducer.ts}`
  (the pull-only model this task pushes into) and `apps/desktop/src/features/
  documents/use-documents.ts:237-252` (`checkStatus`, and the comment already
  on the focus listener naming this task — §13 fills that seam).

## Allowed scope

Lane W (`docs/plans/execution-board.md:76-84`):

- `crates/workspace/src/{watch.rs (new),path.rs,listing.rs,lib.rs,Cargo.toml}`
- `crates/gritty-core/src/workspace/**` (a new `watch.rs` submodule is fine)
- root `Cargo.toml` (the new dependency, one-line justification comment, style
  of the existing `sha2`/`nix`/`portable-pty` entries)
- `apps/desktop/src/features/explorer/**`
- `docs/architecture/documents.md` (correct the stale line), `docs/architecture/
  workspace.md` (new — the domain has no page yet), this file

Two more files are needed and claimed by **no** lane in `docs/plans/
execution-board.md`: `crates/gritty-core/src/event/mod.rs` (one additive
`CoreEvent` variant, §11) and `apps/desktop/src/features/documents/
use-documents.ts` (one new branch, §13). Both are small and additive; claim
them by name with `agent-lease.py` rather than folding them into the Lane W
glob silently or skipping them for sitting outside it.

## Explicit non-goals

- **No new document event, no second external-modification source of truth.**
  The watcher makes `documents.status` fire sooner; it never decides
  `MODIFIED_EXTERNALLY` and never touches `crates/gritty-core/src/document/**`
  — two disagreeing checkers is the silent resolution hard rule 7 forbids.
- **No live `size`/`modifiedAt` update** for a file changed in place without
  its directory's entry set changing — the row goes stale until next listed,
  rather than this task building a second listing representation.
- **No settings toggle.** Runs whenever a workspace is open, like
  `documents.status` on focus already does, and needs no OS permission grant
  unlike T223's notifications; an opt-out is a follow-up if needed.
- **No enforcement, no assumption that T326 consumes this.** T070 is an
  observer (`docs/architecture/ownership.md`'s "OBSERVED" side); T326's own
  doc says it plans to use T230's timeline, not raw filesystem events.
- **No Windows** — not exercised or claimed; compiling is not evidence (hard
  rule 11), matching T097's identical exclusion.
- **No `crates/platform` crate** — pre-assigned to Lane A (T224); the OS
  dispatch stays inside `crates/workspace/src/watch.rs` so migrating it later
  touches one call site.

## Required behavior

### Watching the open workspace

1. `WorkspaceManager` owns at most one live watch, matching the
   single-open-workspace model `current_open` (`:344`) assumes: it starts on
   `open`'s success and stops on `close` (`:314`), a replacing `open`, and the
   root-missing path inside `current` (`:350-360`).
2. It attaches to `WorkspaceRoot::path()`, the canonicalized root — the same
   root every other check resolves against, not the pre-canonicalization
   candidate.
3. New dependency: **`notify`**, the cross-platform watch crate (inotify /
   FSEvents; `ReadDirectoryChangesW` unused per the Windows non-goal).
   Justification: nothing in the tree observes the filesystem, and hand-
   rolling three platform backends violates hard rule 11 worse than a
   dependency does; it lives entirely inside `gritty-workspace`.
4. `WorkspacePath` has no native-path constructor today (only `to_native`).
   Add one that strips the canonicalized root prefix and validates
   component-wise, as `join` already does, so an event path resolving outside
   the root (symlink race, bind mount) is rejected, not trusted
   (`docs/architecture/security.md:48-53`).

### Filtering, coalescing and bounds

5. Every event matching `is_temp_file_name` is dropped before coalescing.
   Otherwise T030's atomic write (temp file created, then renamed over the
   destination) reports a changed directory on every autosave — a
   self-inflicted storm from Gritty's own debounced document updates.
6. Remaining events coalesce per workspace on a `WriterConfig`-shaped timer: a
   quiet-period debounce plus a `max_wait` ceiling so a continuous flood still
   flushes periodically. Durations are named constants a test can shrink, as
   `WriterConfig`'s test constructor does.
7. A batch reports **directories** (the changed entry's parent
   `WorkspacePath`), deduplicated and sorted — the coarsest signal every
   backend can honestly give (FSEvents coalesces by directory, no per-entry
   fidelity across a rename), and what the explorer already keys nodes by.
8. A batch is capped at a fixed directory count; above it, it reports
   `overflowed: true` with an empty list — never a partial list presented as
   complete, `OutputBatch.truncated_bytes`'s pattern applied to paths.
9. The watch does not recurse into an `IGNORED_DIRECTORY_NAMES` entry (its own
   create/remove/rename is still reported). Export the predicate as
   `pub fn is_ignored(path: &WorkspacePath) -> bool` next to the constant in
   `crates/workspace/src/listing.rs`, so the watch and T080's directory
   walker share one answer — a generated tree is exactly what exhausts a
   platform's per-user watch-descriptor budget (`fs.inotify.max_user_watches`;
   `fixtures/large-repo` is depth 25).

### Root disappearance and the explorer

10. A vanished root is today noticed only when a command touches `current` or
    `require` (`:350`, `:448`) — lazy. The watcher calls the existing
    close-with-`RootMissing` path as soon as it observes the root gone,
    instead of waiting: no new closing logic (hard rule 2), just triggered
    eagerly. A root recreated with the same content is still a
    delete-then-close, opened fresh through the ordinary `open` flow.
11. New `CoreEvent::WorkspaceEntriesChanged { workspaceId, directories:
    Vec<WorkspacePath>, overflowed: bool }` in `crates/gritty-core/src/event/
    mod.rs`, added to both `name()` and `operation_id()` (`None` — not an
    operation). Protocol version bumps MINOR (additive): take the next free
    value from `docs/merge-queue.md`, which is the record, immediately before
    the PR opens. The execution board's per-lane reservation is a planning
    hint and has already been overtaken once.
12. `explorer-reducer.ts` re-lists, for each directory in the batch that is a
    currently loaded node (`children !== null`), exactly as `reload` already
    does — same `load` call, no new loading state; an unexpanded node is
    untouched. On `overflowed: true`, re-list every loaded node instead of
    trusting an absent list — bounded by how many directories the user has
    expanded, not the storm's size, which is what makes B5's "final state
    converges" achievable without forwarding the storm to the renderer.
    `use-explorer.ts` needs no new subscription: every event already flows
    through `dispatch({ type: 'event', event })` (`:46`); only the reducer's
    `onEvent` switch grows a case.

### External change detection

13. `use-documents.ts`'s `subscribeAll` handler (`:189-204`) gains a branch:
    on `WorkspaceEntriesChanged`, call `checkStatus()` (`:237`) — the same
    function the focus listener at `:247-252` calls — replacing that
    listener's comment with one naming this branch. No per-path filtering is
    added on the TypeScript side; the core already bounded the batch (§6-8),
    and no code in `crates/gritty-core/src/document/**` changes — this is the
    whole of "the same path."

## Security

A native path from the watch backend is validated like a wire path (§4):
canonicalized-prefix check before it becomes a `WorkspacePath`. Names in
`WorkspaceEntriesChanged` are untrusted repository content (hard rule 13),
exactly as `DirEntry.name` already is; nothing new is built from them.

## Failure behavior

| Situation | Behavior |
|---|---|
| Watch backend fails to start | Workspace still opens; `tracing::warn!`; on-demand checks stay correct, just not proactive |
| Linux inotify watch limit hit mid-tree | Established watches keep reporting; the rest degrades to pre-T070 behavior — not claimed as tested without lowering the real limit (hard rule 12), a documented risk |
| Event resolves outside the canonicalized root, or inside an ignored subtree | Dropped before becoming a `WorkspacePath` (§4), or not observed except the ignored directory's own entry (§9) |
| Root deleted, or replaced atomically under the same name | `RootMissing` close runs immediately (§10); a replacement is delete-then-close, never reconciled |
| Batch exceeds the directory cap | `overflowed: true`, empty list (§8) |
| Workspace closes with a batch pending, or the watcher thread panics | Dropped with it; no event for a gone workspace, nothing else affected, logged once |
| Gritty's own atomic save | Filtered by `is_temp_file_name` (§5); no batch from it alone |

## Acceptance criteria

- Rapid create/remove inside `fixtures/large-repo` (10,501 files, depth 25)
  keeps the UI responsive with batched, bounded events — A6 and B5.
- An external edit while unfocused surfaces `MODIFIED_EXTERNALLY`/
  `CONFLICTED` within one debounce window, against the existing state matrix.
- Create/delete inside an expanded, loaded explorer directory updates it
  unasked; the same inside a collapsed or ignored directory does nothing, and
  a storm exceeding the batch cap still converges once it stops.
- Deleting the open root closes it with `RootMissing` within one debounce
  window, not at the next user action.
- `pnpm check` green; generated protocol types fresh; `docs/architecture/
  documents.md:39` no longer says "it is not watched."

## Tests required

- **Unit (Rust):** coalescing/debounce over a synthetic, clock-driven event
  source — no real filesystem or timing, per the phase-0-spike lesson that a
  threshold calibrated on one Mac fails in CI: quiet-period vs. `max_wait`,
  the directory cap and `overflowed`, dedup/sort, `is_temp_file_name`
  filtering, `is_ignored` by name and by nested path.
- **Unit (Rust):** native-path-to-`WorkspacePath`, including a path resolving
  outside the root (rejected), reusing `TempTree`
  (`crates/workspace/tests/listing.rs:71-84`); root-missing triggers
  `RootMissing` without a command.
- **Integration (Rust):** a real `notify` watch against a scratch directory
  (`std::env::temp_dir()`, unique per test — no new `tempfile` dependency)
  proves a real create/modify/delete/rename yields one coalesced
  `WorkspaceEntriesChanged`, and the same inside `node_modules` yields none.
- **Unit (TS):** `explorer-reducer.ts`'s new case, loaded vs. unloaded nodes,
  `overflowed`; `use-documents.ts`'s branch calls `checkStatus` once per event.
- **Manual:** the large-repo storm; an external edit while unfocused; the
  inotify-limit row, recorded as run or not attempted, never asserted
  without having run it (hard rule 12).

## Commands to run

```
cargo test -p gritty-workspace
cargo test -p gritty-core
cargo clippy --workspace --all-targets -- -D warnings
pnpm check
```

## Completion report

_(filled in by the implementing session — including the `notify` justification
the dependency policy requires, the chosen debounce/`max_wait`/cap constants
and why, confirmation that `docs/architecture/documents.md:39` and the A6/B5
rows in `docs/plans/phase-0-spike.md` were updated with real results, and
whether the ADR recommended in "Motivation" was written before the code)_
