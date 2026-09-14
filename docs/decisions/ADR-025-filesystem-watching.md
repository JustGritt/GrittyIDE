# ADR-025: Filesystem watching is one coalesced, bounded, directory-grained signal

- **Status:** Accepted (implemented by T070)
- **Date:** 2026-09-06
- **Accepted:** 2026-09-14 by Alexis, as drafted. Checked against the tree that
  day: the timings and the cap match `crates/workspace/src/watch.rs:47-54`, and
  a watcher batch reaches documents only through the existing status check
  (`use-documents.ts:206`). Context's "ADR-001 is still `Proposed`" is
  historical: ADR-001 has since been accepted, and the reason for a domain crate
  stands anyway. Linux remains the documented risk in *Consequences*: no inotify
  descriptor has been consumed, and Linux CI is dark.

## Context

Until T070 nothing in Gritty observed the filesystem. `DocumentManager` checks
a file's disk state only when asked — `documents.status`, before every save, on
reload (ADR-011) — and the explorer lists a directory once and never refreshes
it unasked. ADR-011 already wrote down where the watcher would attach: "watcher-
driven checks arrive with T070 through the same path", meaning the existing
check, not a second one.

Two rows of `docs/plans/phase-0-spike.md` were deferred to this task rather
than answered vacuously: A6 ("filesystem watching; rapid changes do not lock
the UI") and B5 ("filesystem storm… UI responsive; events batched/bounded;
final state converges"). They are the acceptance criteria, and they are what
shapes this ADR: a watcher is a firehose pointed at a renderer, and hard rule 8
forbids pointing it there.

Three constraints frame the design.

- **A backend's fidelity is not the same on every platform.** FSEvents
  coalesces by directory and, across a rename, does not tell you which entry
  moved. inotify is per-entry but costs one watch descriptor per directory,
  against a per-user budget (`fs.inotify.max_user_watches`) that a
  `node_modules` tree exhausts on its own.
- **The renderer already has a correct, cheap way to converge**: re-list the
  directories it has loaded. It does not need per-entry deltas, and a per-entry
  delta protocol would be a second representation of a listing.
- **ADR-001 is still `Proposed`** (roadmap amendment, 2026-09-06). Whatever is
  built here must survive a change of desktop runtime, which means it belongs
  in a domain crate behind the typed protocol, not in the Tauri layer.

## Decision

- **The crate is `notify` 8.2.0**, pinned exactly, used only inside
  `gritty-workspace`. Nothing in the tree observes the filesystem, and
  hand-rolling FSEvents and inotify would mean two unreviewed platform
  backends where hard rule 11 wants one small adapter. Its own dependency
  surface is small (`bitflags`, `libc`, `log`, `walkdir`, and the one
  platform `-sys` crate). Windows is neither exercised nor claimed, matching
  T097's exclusion; `ReadDirectoryChangesW` compiles and is not tested.

- **The signal is a batch of directories, never a per-entry delta.**
  `WorkspaceEntriesChanged { workspaceId, directories, overflowed }` reports
  the *parent* `WorkspacePath` of every changed entry, deduplicated and sorted.
  That is the coarsest thing every backend can say honestly, and it is what
  the explorer already keys nodes by. A file changed in place without its
  directory's entry set changing therefore leaves a stale `size`/`modifiedAt`
  in the explorer row until it is next listed: accepted, because the
  alternative is a second listing representation for a cosmetic field.

- **The batch is coalesced on a quiet period with a ceiling**, the shape
  `WriterConfig` already uses for snapshots: `DEBOUNCE` 200 ms of quiet, and
  `MAX_WAIT` 1 s so a continuous flood still flushes periodically instead of
  starving. 200 ms is below the threshold at which an unfocused external edit
  feels unnoticed and well above the burst width of an editor's own atomic
  save; 1 s bounds how long a `git checkout` can withhold the whole batch.
  Both are named constants a test shrinks, and both are timings, not
  correctness: a wrong value costs latency, never a missed change.

- **The batch is capped at `MAX_DIRECTORIES` = 64, and above it reports
  `overflowed: true` with an empty list.** Never a partial list presented as
  complete — the pattern `OutputBatch.truncated_bytes` already sets. The
  renderer's answer to `overflowed` is to re-list every directory it has
  loaded, which is bounded by how many the user expanded rather than by the
  size of the storm. That is what makes B5's "final state converges" reachable
  without forwarding the storm.

- **Gritty's own temporary files are dropped before coalescing.** T030's
  atomic save creates `.<name>.gritty-<pid>-<n>.tmp` and renames it over the
  destination; without this filter every debounced autosave would report its
  own directory as changed — a storm Gritty inflicts on itself.

- **Ignored subtrees are pruned, and `is_ignored` is one shared answer.**
  `gritty_workspace::listing::is_ignored(&WorkspacePath)` sits next to
  `IGNORED_DIRECTORY_NAMES`, is true when *any* segment is an ignored
  directory name, and is used both to prune the watch and (by T080) to prune a
  directory walk. An ignored directory's own creation or removal is still
  reported, because that is a change to its *parent*, which is not ignored.

- **How the pruning is achieved is a platform choice, and it is a runtime
  value rather than a `cfg`.** `WatchStrategy::Recursive` hands the root to the
  backend once; `WatchStrategy::PerDirectory` walks the tree, skipping ignored
  subtrees, and registers one non-recursive watch per directory, adding one
  when a new directory appears. macOS defaults to `Recursive` because FSEvents
  is one stream for the whole subtree and `notify` restarts that stream on
  every added path — per-directory registration would be quadratic for no
  benefit, so there the pruning is a filter on events. Linux defaults to
  `PerDirectory` because that is where the descriptor budget exists and where
  an unpruned `node_modules` is the documented failure. Making the strategy a
  value, not a `cfg`, is what lets the Linux behaviour be *tested* on a Mac
  instead of merely compiled (hard rule 11).

- **The watcher is a background observer, not an operation.** It has no start,
  no completion and no result, so the operation model (hard rule 10) has
  nothing to model; it is stopped by dropping its handle, exactly like the
  terminal flusher and the session writer. Drop sets a stopped flag and drops
  the backend rather than joining the pump thread, so a workspace can close
  while a batch is pending without the close path ever waiting on the thread
  that might be trying to call back into it.

- **A vanished root closes the workspace eagerly** through the existing
  `RootMissing` path (`WorkspaceManager::current`/`require`), never through new
  closing logic. The pump wakes at least once per `ROOT_CHECK_INTERVAL` (1 s)
  to confirm the root is still a directory, because a backend that loses its
  root does not reliably say so. A root recreated under the same name is a
  delete-then-close and is opened fresh; nothing is reconciled.

- **A native path is validated before it becomes a `WorkspacePath`.**
  `WorkspacePath::from_native(root, native)` strips the canonicalized root
  prefix and validates component-wise, rejecting anything that resolves
  outside the root (a symlink race, a bind mount) and anything that is not
  UTF-8. `WorkspaceRoot::relativize` now delegates to it, so there is one
  answer to "is this path inside the root" rather than two.

- **The watcher never decides `MODIFIED_EXTERNALLY`.** It makes the existing
  on-demand check fire sooner: the renderer's `WorkspaceEntriesChanged`
  handler calls the same `documents.status` the focus listener already calls.
  Nothing in `crates/gritty-core/src/document/**` changes. Two checkers that
  could disagree is the silent resolution hard rule 7 forbids.

## Consequences

- The explorer converges without polling, and an external edit surfaces within
  one debounce window instead of at the next save or window focus.
- T080 inherits `is_ignored` as the cheap per-entry predicate its walker needs;
  it does not have to reuse watcher events for that.
- A storm's cost to the renderer is bounded by what the user expanded, not by
  the storm. The cost to the *core* is one `BTreeSet` of at most 64 paths.
- Linux is a documented risk, not a claim: `PerDirectory` is exercised on macOS
  by inverting the default, but no inotify descriptor was ever consumed on this
  machine, and the ENOSPC path is not tested (hard rule 12).
- A rename inside a watched directory is two changed directories, or one; the
  renderer re-lists both and cannot tell the difference, which is the point.
- No settings toggle: the watch runs whenever a workspace is open, like the
  focus check already does. An opt-out is a follow-up, not a default.

## Revisit when

Per-entry fidelity is actually needed (a live `size` column, or T326 wanting
raw filesystem events rather than T230's timeline); the 64-directory cap
overflows in ordinary use rather than during a storm; Windows is claimed; or a
real inotify budget failure is observed and the `PerDirectory` walk needs to
report its own degradation rather than logging it.
