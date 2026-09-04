# Task T060 — Session persistence + recovery

- **Phase:** 0 / 1B
- **Status:** in-progress
- **Depends on:** T030, T040 (schema work can start after T010)

## Objective

Introduce SQLite with explicit migrations and make Gritty recoverable: persist
workspace, tabs, cursor/selection/layout, and dirty-buffer snapshots so a
force-kill and relaunch restores useful state and never loses unsaved content.

## Dependencies

T030 (documents, hashes), T040 (tabs/cursor), T010 (ids, events).

## Required reading

- Plan §11 (persistence and recovery), §7 (entities), §13 (workspace restore)
- Addendum §4.1 (source of truth), §11.1 dirty-buffer crash test, §11.4 renderer
  reload test, §17 (persistence migration conventions freeze)
- ADR-004, ADR-008 (identifier → app-data directory)

## Allowed scope

- new `crates/persistence/**` (SQLite connection, migrations, repositories)
- `crates/gritty-core` (session model, recovery reconciliation)
- `apps/desktop/src-tauri` (app-data path resolution on the Rust side, startup restore)
- `packages/protocol` (`session.*`, `recovery.*` commands and events)
- `apps/desktop/src/**` (restore flow, recovery prompt)
- dependency additions with justification (`rusqlite` with `bundled`, a migration approach:
  hand-rolled versioned SQL files or `refinery`)

## Explicit non-goals

- Workstream/decision/run tables (T200), terminal buffer persistence, settings sync,
  any cloud storage, persisting Monaco internals.

## Required behavior

1. Database at the platform app-data dir for `io.github.justgritt.gritty`
   (macOS: `~/Library/Application Support/io.github.justgritt.gritty/gritty.sqlite`),
   WAL mode, `schema_version` table, forward-only numbered migrations applied at startup
   inside a transaction; the schema is documented in `docs/architecture/persistence.md` (new).
2. Persisted session state: last workspace root, open tabs (path, order, active, view mode),
   per-document cursor/selection/scroll, window size/position, layout, editor preferences.
   Writes are debounced and never block the UI.
3. Dirty-buffer snapshots: for each dirty document, the full text plus `path`, `savedHash`,
   `currentVersion`, `updatedAt` stored ≤ 2 s after the last change; removed on save/close.
4. Startup reconciliation: restore the workspace and tabs immediately (UI first, content
   incremental); for each snapshot compare the file's current hash with `savedHash`:
   equal → offer "restore unsaved changes" (default yes); different → open as
   `CONFLICTED` with both versions available; file missing → `DELETED_EXTERNALLY` with the
   snapshot recoverable. Never apply a snapshot to disk without the user's action.
5. Renderer reload (dev or crash of the WebView) rehydrates from the core without
   touching SQLite consistency.
6. Previously RUNNING operations recorded as durable (none yet, but the field exists)
   become `INTERRUPTED` on startup.

## Failure behavior

- Corrupt or unopenable database → move it aside with a timestamp, start with a fresh
  one, show a notice with the path; never crash-loop.
- Migration failure → refuse to start the session layer, keep the editor usable without
  persistence, show the error; never partially migrate.
- Disk full or write error during snapshot → log, keep the buffer in memory, retry with backoff.
- Snapshot content never overwrites a repository file automatically.

## Acceptance criteria

- Addendum §11.1: edit without saving → `kill -9` → relaunch → unsaved content offered
  and restored correctly (automated where possible, manual otherwise, recorded).
- Restart restores workspace, tabs, active tab, cursor positions and window geometry.
- §11.4 renderer reload passes.
- A deliberately corrupted DB file leads to a clean start with a notice.

## Tests required

- Rust unit tests: migrations from empty and from each prior version; repository CRUD;
  reconciliation matrix (hash equal / different / missing).
- Rust integration test: crash simulation (write snapshot, drop process state, reload).
- Vitest: restore flow state machine.
- Manual: the force-kill test on macOS with results in the completion report.

## Commands to run

`pnpm check`, `pnpm dev`, `kill -9 $(pgrep -f Gritty)` for the crash test.

## Completion report

(fill in)
