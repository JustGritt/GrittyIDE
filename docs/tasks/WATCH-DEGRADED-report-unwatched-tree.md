# Task WATCH-DEGRADED — Say when part of the tree isn't being watched

- **Phase:** maintenance (ADR-025's *Revisit when*, amending ADR-025)
- **Status:** review
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

### Summary

A watch that cannot cover the whole workspace now says so, once per reason, and
the explorer shows it with a **Refresh tree** action.

- **The watcher crate reports reasons.** `gritty-workspace` reports
  `Degraded(RegistrationRefused | DirectoryLimit | BackendError)` from the pump
  thread, each at most once per watch. It covers the initial walk, directories
  that appear later, and backend errors.
- **A rescan is an overflow.** A backend rescan event now makes the batch
  `overflowed`.
- **The core keeps the set.** `gritty-core` keeps the reasons for the open
  workspace, adds `BACKEND_UNAVAILABLE` when the backend will not start, and
  emits `WorkspaceWatchStatusChanged` only when a reason is new.
  `workspace.watchStatus` answers the same set.
- **The explorer merges both.** It asks on open, merges the event by union,
  and shows the notice.
- **Paperwork.** ADR-025 carries the amendment, and protocol 2.28.0 is spent.

### Files changed

| File | What |
|---|---|
| `crates/workspace/src/watch.rs` | `Degradation`, `WatchEvent::Degraded`, `WatchConfig.max_watched_directories` (+ `with_max_watched_directories`), `Pump::degrade` (once per reason), `initial` flushed from the pump, `register_subtree` over an injected registrar returning `SubtreeOutcome`, `watchable_directories` returning whether it was cut short, `Coalescer::overflow` on `need_rescan()`; 4 unit tests |
| `crates/workspace/src/lib.rs` | exports `Degradation` |
| `crates/workspace/tests/watch.rs` | `drain` and the storm test handle `Degraded`; the root-removal test tolerates one; `start_with`; 1 integration test |
| `crates/gritty-core/src/workspace/watch.rs` | `WatchDegradedReason`, `WorkspaceWatchStatus`, `WatchStatusRequest`, `WatchHealth::report` (record, then emit if new), `watch_status`, the `BACKEND_UNAVAILABLE` arm, the `WatchSetup` test seam |
| `crates/gritty-core/src/workspace/mod.rs` | the `watch_health` slot, reset in `emit_closed`; 4 tests |
| `crates/gritty-core/src/event/mod.rs`, `session/tap.rs` | the variant in `name()`/`operation_id()`; not persisted, with the reason |
| `crates/gritty-core/src/protocol/{mod,export}.rs` | 2.28.0; three roots exported |
| `apps/desktop/src-tauri/src/{commands,lib}.rs`, `tests/commands.rs` | `workspace_watch_status` adapter and registration; 1 adapter test |
| `packages/protocol/src/{commands,client,types}.ts`, `workspace-client.test.ts`, generated | `workspace.watchStatus`; the types; 1 client test |
| `apps/desktop/src/features/explorer/{explorer-reducer,use-explorer,explorer-panel}.ts(x)` | `watchDegraded`/`watchNoticeDismissed`, union merge, the query on open, `dismissWatchNotice`, the notice |
| `…/explorer-reducer.test.ts`, `use-explorer-watch.dom.test.tsx`, `explorer-panel-notice.dom.test.tsx` | 4 reducer, 2 hook, 3 panel tests |
| `docs/decisions/ADR-025-filesystem-watching.md`, `docs/decisions/README.md`, `docs/architecture/workspace.md`, `docs/merge-queue.md` | the amendment and its row; the watch section and five failure rows; the 2.28.0 claim (`d64f7e7`) |

### Tests actually executed

| Command | Result |
|---|---|
| `cargo test -p gritty-workspace watch` | exit 0 (4 new unit tests, the new integration test) |
| `cargo test -p gritty-core --lib workspace::tests` | exit 0, 17 passed including the 4 new |
| `cargo test -p gritty-desktop --test commands workspace_watch_status` | exit 0 |
| `cargo test -p gritty-core --lib protocol::export` | exit 0; regenerated `CoreEvent.ts`, `version.ts` and three new files |
| `pnpm vitest run packages/protocol` | exit 0, 18 files, 115 tests; `tsc` exit 0 |
| `pnpm vitest run apps/desktop/src/features/explorer` | exit 0, 9 files, 127 tests; `apps/desktop` `tsc` exit 0 |
| `pnpm check`, first run | exit 101 on this ticket: clippy `type_complexity` on a test helper's return type. Fixed with a type alias. `check:ts` had passed (130 files, 1473 tests) |
| `pnpm check`, second run | exit 1, **not from this change**: biome formatting in the T080-U session's uncommitted search feature, a directory this ticket does not touch |
| The gate's halves, run separately after that | biome on `features/explorer` and `packages/protocol/src` exit 0; `apps/desktop` `tsc` 0 errors; `pnpm check:rust` (fmt, clippy `-D warnings`, `cargo test --workspace`) exit 0, all 42 test binaries ok |
| `pnpm check:generated` | Run after the commit below, because it requires the generated files to be committed |

**Mutation checks**, each run and then restored:

| Mutation | Fails |
|---|---|
| `degrade` delivers every time | `a_degradation_is_delivered_once_per_reason` |
| Rescan events not treated as overflow | `a_rescan_event_overflows_the_batch` |
| The walk never reports it was cut short | `the_walk_says_when_it_stopped_at_its_limit` |
| No `BACKEND_UNAVAILABLE` when the backend refuses | `a_backend_that_refuses_reports_unavailable` |
| Reducer replaces instead of merging | `merges the query and the event by union, in either order`, `stays dismissed until a reason it has not shown arrives` |
| No query on open | `asks on open, so a reason reported before it subscribed still shows` |

### Architectural decisions

The four in the amendment (reasons, once, event and query, rescan as overflow),
plus these:

- **The health slot is keyed by workspace id**, and reset in `emit_closed`,
  where every close already funnels. A report from a watch that is going away
  cannot land on the next workspace.
- **Record, then emit, with the lock released before emitting.** A query after
  subscribing sees what the event said. The pump thread never holds the health
  lock while calling into the event bus, and never takes `current`.
- **A test seam, not a trait, for the refusing backend.** `WatchSetup` is
  `#[cfg(test)]`, the same shape as `listing_delay`. A backend trait would be an
  abstraction with one production implementation.
- **The query failing is silent in the explorer.** The notice is advisory, and
  a failure to ask says nothing about the workspace.

### Known limitations

- **Not observed on Linux.** A real inotify refusal and `ENOSPC` have never
  happened on this machine, and Linux CI is dark (hard rule 12). The counting
  is tested through an injected registrar, the limit through a real
  `PerDirectory` watch on macOS.
- **`BACKEND_ERROR` is reported but never triggered by a test.** No backend
  error can be provoked on demand.
- **Unreadable directories are not degradations**: the explorer cannot list
  them either.
- **The initial `PerDirectory` walk still runs inside `open`**, under the
  `current` lock. That predates this ticket.
- **Nothing was clicked.**

### Follow-up tasks

- **WATCH-PERDIR-NEWDIR** (filed by WATCH-FLAKE): a file created in a new
  directory before its watch attaches is missed on Linux. The fix is to record a
  newly registered directory as changed. It was left out here to keep this
  ticket to the amendment.
- **ADR-034's IGNORE-SEAM** will edit `crates/workspace/src/watch.rs` too, and
  rebases on this.
