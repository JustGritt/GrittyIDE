# The workspace (T020, T049, T070)

One workspace is open at a time. It is a canonicalized root directory, a path
policy that everything crossing the UI/core boundary obeys, a lister that reads
exactly one directory level per call, and — since T070 — a watch that keeps
both of those from going stale. Decisions are in
[ADR-010](../decisions/ADR-010-workspace-paths.md) (root, paths, listing) and
[ADR-025](../decisions/ADR-025-filesystem-watching.md) (watching); this
document is the map.

## Layers

```
apps/desktop/src/features/explorer    the tree, its reducer, the re-list on change
        │  workspace.open/close/current/list/recents, Workspace*, WorkspaceEntriesChanged
apps/desktop/src-tauri/commands.rs    adapters (parse, delegate, correlate)
        │
gritty_core::workspace                ids, lifecycle, events, the listing operation
        │                             workspace/watch.rs: the batch → one event
crates/workspace (gritty-workspace)   WorkspaceRoot, WorkspacePath, list_directory,
                                      watch.rs: notify, coalescing, bounds
```

`gritty-workspace` knows nothing about ids, events or Tauri; `gritty-core`
knows nothing about `notify`. That seam is what makes the watch survivable if
the desktop runtime changes: a different shell costs the command adapters, not
the domain.

## Root and paths

`WorkspaceRoot::open` canonicalizes an absolute path, requires a readable
directory, and keeps the canonical form. Every path that crosses the boundary
is a `WorkspacePath`: relative to the root, `/`-separated, no `.` or `..`, `""`
for the root itself. Resolution is lexical first and physical second — the
joined path is canonicalized and must still start with the root, which is what
rejects a symlink pointing outside (`security.md`).

`WorkspacePath::from_native` is the inverse, for paths that arrive from the
platform rather than from the protocol: a watcher event, a file dropped on the
window. It strips the canonical root prefix component-wise and refuses
anything that resolves outside it, or that is not UTF-8 in every component.
`WorkspaceRoot::relativize` is the same answer with the reason dropped.

## Listing

`workspace.list` reads one level, sorted (directories first, then by name) and
paginated with an opaque cursor. Nothing is filtered: hidden entries carry
`hidden`, generated directories carry `ignoredHint`, symlinks report where they
resolve without being followed. A listing that outlives its 100 ms budget
continues as a cancellable `workspace.list` operation whose completion carries
the page.

`is_ignored(&WorkspacePath)` — true when any segment is one of
`IGNORED_DIRECTORY_NAMES` — is the one shared answer to "should anything
descend into this": the watcher prunes its registrations with it, and T080's
walker will call it per entry.

## Watching (T070)

The watch starts when a workspace opens and stops when it closes — a user
close, a replacing open, or a root that disappeared. There is at most one, and
every close funnels through the same place, so no path can leave one running.

What it delivers is one event:

```
WorkspaceEntriesChanged { workspaceId, directories, overflowed }
```

`directories` are workspace-relative parents of changed entries, deduplicated
and sorted. Directories rather than entries, because that is the coarsest
signal every backend gives honestly and it is what the explorer keys nodes by.
Four bounds stand between a filesystem storm and the renderer:

| Bound | Value | Why |
|---|---|---|
| temp-file filter | `.<name>.gritty-<pid>-<n>.tmp` | Gritty's own atomic save must not report itself |
| ignore pruning | `IGNORED_DIRECTORY_NAMES` | a generated tree is the storm, and on Linux it is also the descriptor budget |
| coalescing | 200 ms quiet, 1 s ceiling | one batch per burst; a continuous flood still flushes |
| batch cap | 64 directories | above it `overflowed: true` with an **empty** list, never a partial one |

The renderer's side of the bargain is in `explorer-reducer.ts`: a named
directory is re-listed only when it is a loaded node, and `overflowed` re-lists
every loaded node. Either way the work is bounded by what the user expanded,
not by the size of the change — which is what lets a storm converge without
being forwarded.

How the tree is handed to the backend is a platform choice and a runtime value
(`WatchStrategy`), not a `cfg`, so the branch this machine does not run is
still testable: macOS registers the root once recursively (FSEvents is one
stream, and `notify` restarts it per added path), Linux registers one
non-recursive watch per non-ignored directory and follows new ones, because
that is where `fs.inotify.max_user_watches` exists.

The watch never decides anything about a document. A batch makes the shell call
`documents.status` — the same command the window-focus listener calls — and the
external-modification state stays that check's answer (ADR-011). Two checkers
that could disagree is the silent conflict resolution the hard rules forbid.

## Failure

| Situation | Behaviour |
|---|---|
| The backend refuses to start | The workspace still opens; logged once; every on-demand check is as correct as before, just not proactive |
| A watch descriptor is refused mid-tree (Linux) | The registrations that succeeded keep reporting; the rest of the tree behaves as it did before T070. Not claimed as tested |
| An event resolves outside the root | Dropped before it can become a `WorkspacePath` |
| The root is deleted | The existing `RootMissing` close runs at once instead of at the next command; confirmed with a stat before anything closes. A root recreated under the same name is a delete-then-close, never reconciled |
| The workspace closes with a batch pending | Dropped with the watch. No event ever names a workspace that is gone |
