# Task T070 — Watching the workspace turns two on-demand checks proactive

- **Phase:** 1 / 2
- **Status:** review
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

**ADR-024 was written before the code**, as "Motivation" asked, and the spec was
accepted rather than revised — except in one place, §9, where the ADR records
*how* the ignore pruning is achieved (see "Architectural decisions").

### Summary

The workspace now has a watch. It attaches to the canonicalized root when a
workspace opens, stops when it closes, and delivers one coalesced, bounded
event — `WorkspaceEntriesChanged { workspaceId, directories, overflowed }` —
that makes two on-demand checks proactive without becoming a second source of
truth for either. The explorer re-lists the loaded nodes a batch names; the
document store calls the same `documents.status` the focus listener calls. A
vanished root now runs the existing `RootMissing` close at once instead of at
the next command.

### Files changed

| File | What |
|---|---|
| `crates/workspace/src/watch.rs` (new) | `notify` backend, the two registration strategies, event classification, the coalescer, the pump thread |
| `crates/workspace/src/path.rs` | `WorkspacePath::from_native` (§4) and its three tests |
| `crates/workspace/src/root.rs` | `relativize` now delegates to `from_native`, so there is one answer, not two |
| `crates/workspace/src/listing.rs` | `is_ignored(&WorkspacePath)` (§9) next to `IGNORED_DIRECTORY_NAMES`, and its test |
| `crates/workspace/src/lib.rs`, `crates/workspace/Cargo.toml` | module and dependency |
| `crates/workspace/tests/watch.rs` (new) | 10 integration tests against a real backend, plus the `#[ignore]`d large-repo storm |
| `crates/gritty-core/src/workspace/watch.rs` (new) | `WorkspaceEntriesChangedPayload`, `start_watch`, `close_missing_root` |
| `crates/gritty-core/src/workspace/mod.rs` | the `watch` field, `Arc::new_cyclic` for the `Weak<Self>` the callback needs, start on `open`, stop in `emit_closed`, 3 tests |
| `Cargo.toml`, `Cargo.lock` | `notify = "8.2.0"` |
| `crates/gritty-core/src/event/mod.rs` | one `CoreEvent` variant, in `name()` and `operation_id()` |
| `crates/gritty-core/src/protocol/mod.rs` | `PROTOCOL_VERSION` 2.12.0 → **2.14.0** |
| `crates/gritty-core/src/session/tap.rs` | the new variant in the exhaustive match: not persisted, with the reason |
| `packages/protocol/src/types.ts`, `src/generated/**` | the payload export; generated by `cargo test -p gritty-core` |
| `apps/desktop/src/features/explorer/{explorer-reducer.ts,use-explorer.ts}` | the `WorkspaceEntriesChanged` case, `state.stale`, the effect that re-lists |
| `apps/desktop/src/features/explorer/explorer-reducer.test.ts` | 5 tests for the new case |
| `apps/desktop/src/features/explorer/use-explorer-watch.dom.test.tsx` (new) | 3 tests: the batch actually re-lists, counted per path — a loaded directory once, an unopened one never, every loaded node on `overflowed` |
| `apps/desktop/src/features/documents/use-documents.ts` | the branch (§13); `checkStatus` moved above the subscription that now uses it |
| `apps/desktop/src/features/documents/use-documents-watch.dom.test.tsx` (new) | 2 tests: the branch calls `documents.status` once per batch |
| `docs/decisions/ADR-024-filesystem-watching.md` (new), `docs/decisions/README.md` | the decision |
| `docs/architecture/workspace.md` (new) | the domain had no page; it does now |
| `docs/architecture/documents.md` | the stale "it is not watched (T070)" line, corrected |
| `docs/plans/phase-0-spike.md` | A6 and B5, filled with measurements |

**No `commands.rs` change**: the watcher adds no command, and the Tauri layer
forwards every `CoreEvent` generically. The wire surface is one event.

### Tests actually executed

| Command | Result |
|---|---|
| `cargo test -p gritty-workspace` | **74 passed** (39 lib incl. the new watch/ignore/path tests, 14 documents, 11 listing, 10 watch integration) |
| `cargo test -p gritty-core` | **322 passed**, plus 3 crash-recovery; regenerated the protocol types |
| `cargo test -p gritty-workspace --test watch -- --ignored --nocapture` | 1 passed; the A6/B5 measurement below |
| `pnpm check:rust` (`fmt --check`, `clippy --workspace --all-targets -D warnings`, `cargo test --workspace`) | exit 0, no warnings. `pty.rs::kill_terminates_the_whole_process_tree` passed |
| `pnpm check:ts` (biome, tsc, vitest) | exit 0: **75 files, 584 tests passed** |
| `pnpm check:generated` | exit 0 |
| `pnpm check` (the whole gate, run four times) | **exit 1 every time, on two suites this branch does not touch** — see below |

**The two gate failures are not this branch, and that is provable rather than
argued.** `git diff main...HEAD --name-only -- packages/editor packages/markdown`
is **empty**: both packages are byte-identical to `main`, and the only new
dependency here is a Rust crate, which cannot reach a Vitest run. They are
`packages/editor/src/monaco/model-eol.dom.test.ts` (its 10 s `beforeAll` times
out) and `packages/markdown/src/render.test.ts` ("renders a long document
within budget", measured 2037–3988 ms against a 2000 ms bound). Both are
contention-sensitive: they fail when all 75 test files run in parallel on a
machine where five agent sessions are building, and both pass in isolation here
(`pnpm vitest run packages/editor` → 41/41). The editor one was reproduced with
this branch **stashed**, and another session in this wave has already filed it
on the board as `MONACO-JSDOM-FLAKE`. The markdown budget deserves the same
ticket and does not have one.

So: `pnpm check:ts` was observed **exit 0 with 581/581 passing**,
`pnpm check:rust` **exit 0**, and `pnpm check:generated` **exit 0** — all three
halves green, and the composite red only through those two pre-existing flakes.
A reviewer on a quiet machine should see it green; that is stated as a claim,
not as an observation (hard rule 12).

Note also that `cargo test --workspace` needs
`pnpm --filter @gritty/desktop sidecar` to have staged
`binaries/gritty-agent-hook-*` first — a pre-existing T225 prerequisite, not
something this branch introduced.

### A6 and B5, measured

`cargo test -p gritty-workspace --test watch -- --ignored --nocapture`, macOS 26
arm64, against `target/fixtures/large-repo` (10 501 files, 103 directories,
depth 25):

- **Attaching the watch: 18–52 ms** for the whole tree.
- **590 filesystem operations across 75 directories in 65 ms produced exactly
  one event**, `overflowed: true` with an **empty** list.
- A narrower storm (350 operations, 50 directories, under the cap) produced one
  batch naming all 50.
- The single change *after* the storm was reported on its own: it converges
  rather than latching.

The renderer's half is bounded by construction rather than by measurement: a
batch re-lists only directories the user has expanded, and `overflowed`
re-lists all of those — never the storm's size. Asserted in
`explorer-reducer.test.ts`.

### Architectural decisions

1. **Directories, not entries** (ADR-024). FSEvents coalesces by directory and
   cannot say which entry moved across a rename, so a per-entry protocol would
   be a promise one platform cannot keep. The cost is the accepted non-goal: a
   file changed in place leaves a stale `size`/`modifiedAt` in its explorer row
   until that row is next listed.
2. **`notify` 8.2.0**, pinned, used only inside `gritty-workspace`. Nothing in
   the tree observed the filesystem, and hand-rolling FSEvents *and* inotify
   would be two unreviewed platform backends where hard rule 11 wants one small
   adapter. Its transitive surface is small and ordinary — `bitflags`, `libc`,
   `log`, `walkdir`, `notify-types` and one platform `-sys` crate — and it
   overlaps nothing already in the tree. 9.0.0-rc.5 exists; a release candidate
   is not a pinned dependency.
3. **The constants — `DEBOUNCE` 200 ms, `MAX_WAIT` 1 s, `MAX_DIRECTORIES` 64.**
   200 ms is below the point where an unfocused external edit feels unnoticed
   and comfortably above the burst width of one atomic save; 1 s bounds how long
   a `git checkout` can withhold a whole batch; 64 is well above what an
   ordinary edit touches and well below the point where re-listing every
   expanded node is the cheaper answer anyway. All three are timings, not
   correctness: a wrong value costs latency, never a missed change. Shrinkable
   through `WatchConfig::for_tests`, the way `WriterConfig` already does it.
4. **§9's mechanism is where this revises the spec.** "Does not recurse into an
   ignored entry" is achieved differently per platform, and the choice is a
   **runtime value** (`WatchStrategy`), not a `cfg`. macOS registers the root
   once recursively — FSEvents is one stream for a subtree and `notify` restarts
   that stream on *every* added path, so per-directory registration would be
   quadratic for no benefit — and filters ignored subtrees out of the event
   stream. Linux registers one non-recursive watch per non-ignored directory and
   follows new ones, because that is where `fs.inotify.max_user_watches` exists
   and where the failure table's "established watches keep reporting" needs
   per-directory registration to be true at all. Making it a value is what lets
   `PerDirectory` be **tested on this Mac**
   (`the_per_directory_strategy_reports_the_same_changes`,
   `a_directory_created_after_the_watch_started_is_followed`) instead of only
   compiled — hard rule 11.
5. **Not an operation, and drop never joins.** A watch has no start, completion
   or result, so the operation model has nothing to model (hard rule 10); it is
   the terminal flusher's shape. Dropping the handle sets a flag and wakes the
   pump rather than joining it, which is what lets the pump itself trigger the
   `RootMissing` close — that close drops the handle the pump is running under —
   without deadlocking against its own close path.
6. **Stopping lives in `emit_closed`.** Every close (user, replaced, root
   missing) funnels through it, so no path can leave a watch running and no
   event can name a workspace that is gone.
7. **The reducer names, the hook lists.** `explorer-reducer.ts` is pure, so it
   records stale paths in `state.stale` and `use-explorer.ts` runs the same
   `load` a manual reload runs. No new subscription: the event already reaches
   the reducer through the single `subscribeAll`.
8. **`close_missing_root` confirms before it closes.** The watcher's observation
   is a reason to look, not a verdict: an `is_present()` check runs under the
   lock, so a root renamed away and back does not close a workspace the user
   cannot then explain.

### Known limitations

- **Linux is compiled and logically tested, not exercised.** `PerDirectory` is
  driven by two tests on macOS, but no inotify descriptor has ever been consumed
  by this code, and the ENOSPC path is not tested — lowering
  `fs.inotify.max_user_watches` needs a Linux box, and CI is dark until the
  Actions allowance resets (`CI-LINUX-DARK`). Recorded as a risk, not a claim
  (hard rules 11 and 12).
- **Windows: not exercised, not claimed**, matching the spec's non-goal.
- **The in-app pass is pending a reviewer.** An external edit surfacing while the
  window is unfocused is asserted at the core (`cargo test -p gritty-core`) and
  at the renderer (`use-documents-watch.dom.test.tsx`), but the agent cannot see
  the WebView; watching a tab actually turn `MODIFIED_EXTERNALLY` without
  touching anything belongs in `docs/testing/phase-0-manual-pass.md`.
- **Root-missing latency is up to `ROOT_CHECK_INTERVAL` (1 s)**, not one debounce
  window as §10's acceptance criterion says. A backend that loses its root does
  not reliably report it, so the pump also stats the root once a second; the
  event path is usually faster, but 1 s is the honest bound.
- **A file changed in place does not refresh its row's `size`/`modifiedAt`** —
  the stated non-goal, restated because it is the thing a user might notice.
- **No opt-out.** The watch runs whenever a workspace is open.

### Follow-up items (not done, deliberately)

1. **PROTO-W2 owns the wire.** This branch spent `PROTOCOL_VERSION` **2.14.0**,
   as claimed in `docs/merge-queue.md`, and added exactly one event:
   `WorkspaceEntriesChanged` / `WorkspaceEntriesChangedPayload
   { workspaceId, directories: WorkspacePath[], overflowed: boolean }`. No
   command, no `commands.rs` change, no `SCHEMA_VERSION`, no migration.
2. **T080 should consume `is_ignored`** as the per-entry predicate its walker
   needs; it is exported and tested for exactly that.
3. **An opt-out setting**, if watching ever proves unwelcome on a network mount
   or a very large tree.
4. **Report degradation instead of only logging it.** When a `PerDirectory` walk
   cannot register part of a tree the user is told nothing; a `watchDegraded`
   flag on the workspace, or a notice, would make the failure table's second row
   visible rather than merely true.
5. **`MAX_WATCHED_DIRECTORIES` (8 192) is a guess.** It bounds the registration
   walk on Linux; nothing has measured what value actually matters.
6. **Two magnet files were claimed for the minute and released**
   (`use-documents.ts` and its new test); `crates/gritty-core/src/event/mod.rs`,
   `protocol/mod.rs`, `session/tap.rs` and `packages/protocol/src/types.ts` were
   each edited exactly once, in one commit, per the wave rules.
7. **`docs/architecture/overview.md:78` is now half-stale** and was left alone:
   the allowed scope names `documents.md` and `workspace.md`, not the overview.
   It says a root that disappears is noticed by "the command that notices",
   which is no longer the only way, and its workspace section has no link to the
   new `workspace.md`. One sentence and one link for whoever owns that file.
8. **One comment in another lane's file is now stale and was left alone**
   (rule 14): `apps/desktop/src/features/agents/open-touched-file.ts:26` says
   "T070's watcher does not exist", and `open-touched-file.test.ts:86` says the
   same. The behaviour they describe is still right — `documents.status` before
   opening is still the correct move, because a file rewritten a second ago may
   not have cleared the debounce yet — so only the parenthetical is wrong.
   Whoever next touches T232's lane should drop that clause.
9. **The `overflowed` fan-out is not throttled.** Every loaded node is re-listed
   in one pass; a user with many directories expanded gets that many
   `workspace.list` calls at once. Bounded and correct, but a queue would be
   kinder.

### Remaining risks

- **The Linux registration path is the real one.** It is the branch most likely
  to be wrong and least likely to be caught, because the one platform that would
  catch it has no CI right now. A reviewer with a Linux box should run
  `cargo test -p gritty-workspace` there before this is trusted.
- **Timing-sensitive tests.** `crates/workspace/tests/watch.rs` waits on a real
  backend. Timeouts are generous (10 s for a delivery, 2 s to believe silence)
  and no test asserts a *speed*, but a heavily loaded machine could still make
  one flake — the same class as
  `pty.rs::kill_terminates_the_whole_process_tree`.
- **The batch cap interacts with the explorer's expansion state.** A user with a
  hundred directories expanded gets a hundred `workspace.list` calls on an
  `overflowed` batch. That is bounded and it is the correct answer, but it is
  the one place where a storm still costs something proportional to the UI.
