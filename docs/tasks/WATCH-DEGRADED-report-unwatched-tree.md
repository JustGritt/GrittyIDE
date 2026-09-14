# Task WATCH-DEGRADED — Say when part of the tree isn't being watched

- **Phase:** maintenance (ADR-025's *Revisit when*, amending ADR-025)
- **Status:** todo
- **Depends on:** WATCH-FLAKE (the two load-sensitive watcher tests; it holds `crates/workspace/tests/watch.rs`), FILES-PARTIAL (`refreshAll`), EXPLORER-RELIST-QUEUE (the Refresh re-lists through the queue)

## Objective

When the watch cannot cover the whole workspace, the explorer says so, once,
with a way to refresh the tree by hand. Today the failure is only a
`tracing::warn!`, and the explorer goes quietly stale in exactly the subtrees
nobody is told about.

## Motivation

ADR-025's *Revisit when* names this: "the `PerDirectory` walk needs to report
its own degradation rather than logging it". T070 recorded it as follow-up 4.
On `a07694c`, every degradation is swallowed:

- **A refused registration.** `register_subtree` (`crates/workspace/src/watch.rs`)
  counts refused registrations and logs a warning.
- **The directory limit.** `follow_tree` returns silently at
  `MAX_WATCHED_DIRECTORIES`, and drops a failed `backend.watch` for a new
  directory without a log. `watchable_directories` truncates at its limit
  silently too.
- **Backend errors** are warned once per watch.
- **A backend that fails to start** is warned in
  `gritty_core::workspace::watch::start_watch`, which returns `None`, and the
  workspace opens as if watched.
- **A backend "rescan" event** (`notify::Event::need_rescan()`: inotify
  `Q_OVERFLOW`, which has no path; FSEvents `MUST_SCAN_SUBDIRS`, which names
  only a parent) is recorded as an ordinary change, or as nothing. ADR-025's
  convergence rule (`overflowed` when the list cannot be trusted) is not
  applied to it.

## The amendment to ADR-025

Accepted by Alexis on 2026-09-14 through the approved plan for this work, which
put these four decisions to them. Recorded in ADR-025 when this ticket lands.

1. **Reasons, not counts.** A degradation is a reason from a small set:
   `BACKEND_UNAVAILABLE`, `REGISTRATION_REFUSED`, `DIRECTORY_LIMIT`,
   `BACKEND_ERROR`. It carries no count: `watchable_directories` stops at its
   limit without knowing how many remain, so a count would be a lower bound
   presented as a total.
2. **Bounded.** Each reason is reported at most once per watch, so at most four
   events per workspace (hard rule 8).
3. **An event and a query.** A new event, `WorkspaceWatchStatusChanged`, and a
   new command, `workspace.watchStatus`. A startup restore answers before the
   WebView subscribes, so an event alone would be lost. The core records a
   reason before emitting it, so a query after subscribing always sees it.
4. **A rescan is `overflowed`.** A rescan event sets the batch to `overflowed`
   instead of being trusted as a partial batch.

## Required reading

- ADR-025 (all), `docs/architecture/workspace.md` § Watching and § Failure
- `crates/workspace/src/watch.rs`, `crates/gritty-core/src/workspace/{watch,mod}.rs`
- `crates/gritty-core/src/event/mod.rs` (`name()`, `operation_id()`),
  `crates/gritty-core/src/session/tap.rs` (the exhaustive persist match)
- `docs/merge-queue.md` (claim **2.28.0** before the first wire edit)
- ADR-034: its IGNORE-SEAM ticket will also edit `watch.rs`; whichever of the
  two lands second rebases

## Allowed scope

- `crates/workspace/src/watch.rs`, `crates/workspace/tests/watch.rs`
- `crates/gritty-core/src/workspace/{watch,mod}.rs`, `crates/gritty-core/src/event/mod.rs`,
  `crates/gritty-core/src/session/tap.rs`, `crates/gritty-core/src/protocol/{mod,export}.rs`
- `apps/desktop/src-tauri/src/{commands,lib}.rs`, `apps/desktop/src-tauri/tests/commands.rs`
- `packages/protocol/src/{commands,client,types}.ts`, `packages/protocol/src/generated/**`
- `apps/desktop/src/features/explorer/**`
- `docs/decisions/ADR-025-filesystem-watching.md`, `docs/decisions/README.md`,
  `docs/architecture/workspace.md`, `docs/merge-queue.md`, this file

## Explicit non-goals

- **No per-directory report** of what is unwatched.
- **No retrying a refused registration**, and no raising the inotify limit.
- **No settings toggle.** The opt-out stays a follow-up.
- **The watcher still never decides `MODIFIED_EXTERNALLY`.**
- **Not WATCH-PERDIR-NEWDIR** (a file created in a new directory before its
  watch attaches), filed separately by WATCH-FLAKE.

## Required behavior

1. **`gritty-workspace`.**
   - `WatchEvent::Degraded(Degradation)`, with
     `Degradation { RegistrationRefused, DirectoryLimit, BackendError }`.
   - `WatchConfig.max_watched_directories`, defaulting to
     `MAX_WATCHED_DIRECTORIES`.
   - The pump delivers each reason once, including those found by the initial
     walk, from the pump thread.
   - A rescan event sets `overflowed`.
2. **`gritty-core`.**
   - `WorkspaceWatchStatus { workspaceId, degraded: WatchDegradedReason[] }`,
     sorted, growing only while the workspace is open.
   - A backend that fails to start records `BACKEND_UNAVAILABLE`.
   - `CoreEvent::WorkspaceWatchStatusChanged` is emitted only when a reason is
     new, after it is recorded.
   - `workspace.watchStatus` answers the current set, and `NotFound` for a
     workspace that is not open.
   - The status is dropped with the watch.
   - The event is not persisted.
3. **Explorer.**
   - It asks `workspace.watchStatus` when a workspace opens and merges the event
     by union; failures of that query are ignored (the notice is advisory).
   - A `role="status"` notice reads "Some folders aren't watched, so changes made
     outside Gritty may not show here.", or, when the backend is unavailable,
     "Changes made outside Gritty aren't being watched."
   - **Refresh tree** re-lists every loaded directory; **Dismiss** hides the
     notice until a new reason arrives.

## Failure behavior

| Situation | Behavior |
|---|---|
| The backend refuses to start | `BACKEND_UNAVAILABLE`; the workspace opens; notice shown |
| Registrations refused mid-tree (Linux) | `REGISTRATION_REFUSED` once; watched parts keep reporting |
| Tree larger than the registration limit | `DIRECTORY_LIMIT` once |
| A backend error stream | `BACKEND_ERROR` once |
| A rescan event | That batch is `overflowed`; not a degradation |
| The query fails, or races the event | Union merge; an empty query answer never clears a reason the event added |
| Workspace closes | Status dropped; no event names a gone workspace |

## Acceptance criteria

- A `PerDirectory` watch with a tiny `max_watched_directories` reports
  `DIRECTORY_LIMIT` exactly once on macOS, and changes in the directories it
  did watch are still reported.
- `workspace.watchStatus` returns it; a fully watched workspace returns `[]`.
- The explorer shows the notice from the query alone, and from the event alone.
- `pnpm check` green; generated types committed; ADR-025 amended; 2.28.0 spent.

## Tests required

- `watch.rs` unit tests:
  - `a_refused_registration_is_counted_not_fatal`, with an injected registrar
  - `the_walk_says_when_it_stopped_at_its_limit`
  - `a_degradation_is_delivered_once_per_reason`
  - `a_rescan_event_overflows_the_batch`
- `tests/watch.rs`: `a_per_directory_watch_over_its_directory_limit_reports_degraded_once`.
- `workspace/mod.rs`:
  - `a_watch_over_its_limit_is_announced_and_queryable`
  - `a_fully_watched_workspace_reports_no_degradation`
  - `a_backend_that_refuses_reports_unavailable`
  - `watch_status_of_a_closed_workspace_is_not_found`
- A `commands.rs` adapter row.
- Reducer merge tests, and a dom test covering the notice and Refresh.
- **Not claimed (hard rule 12):** a real inotify refusal or ENOSPC, and a real
  `BackendError`.

## Commands to run

```
cargo test -p gritty-workspace
cargo test -p gritty-core
pnpm vitest run apps/desktop/src/features/explorer
pnpm check
```

## Completion report

_Not started._
