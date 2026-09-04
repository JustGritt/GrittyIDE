# Session persistence and recovery

Status: fixed by T060 (ADR-004, ADR-014). `crates/persistence`
(`gritty-persistence`) owns the database; `gritty_core::session` owns the
domain; `apps/desktop/src-tauri/src/{core,window_state}.rs` resolve the data
directory and the window; `apps/desktop/src/features/session` drives the
WebView side.

## Where

| | |
|---|---|
| File | `<app data dir>/gritty.sqlite` (+ `-wal`, `-shm`); macOS: `~/Library/Application Support/io.github.justgritt.gritty/` |
| Override | `GRITTY_DATA_DIR=<dir>` (development, manual crash tests) |
| Moved-aside copies | `gritty.sqlite.corrupt-<unix-ms>` next to it, kept for inspection |
| Mode | WAL, `synchronous = NORMAL`, `foreign_keys = ON`, `busy_timeout = 5 s` |

## Schema (version 1)

Owned by the runner: `schema_version (version PK, applied_at)`. Migrations
(`crates/persistence/src/migrations/NNNN_*.sql`) are applied in order, one
transaction each together with their version row; a newer version than the
build knows is refused.

| Table | Key | Columns | Written by |
|---|---|---|---|
| `workspace` | `root` | `last_opened_at` | writer, on `WorkspaceOpened`; read by `workspace.recents`, one row deleted by `workspace.forget` (T049) |
| `session` | single row | `workspace_root` (NULL after a user close), `editor_preferences` (JSON), `layout` (JSON), `updated_at` | writer (root), `session.update` (blobs) |
| `window_state` | `label` | `x`, `y`, `width`, `height` (physical px), `scale_factor`, `maximized`, `updated_at` | writer, from the desktop's window events |
| `workspace_tab` | `(workspace_root, path)` | `position`, `active`, `view_mode`, `cursor_line/column`, `anchor_line/column`, `scroll_line`, `updated_at` | `session.update` (the whole set of a root, one transaction) |
| `buffer_snapshot` | `(workspace_root, path)` | `text`, `encoding`, `saved_hash`, `current_version`, `updated_at` | writer (debounced), removed per the rules below |
| `operation_record` | `id` | `kind`, `workspace_root`, `status`, `correlation_id`, `created_at`, `started_at`, `finished_at`, `updated_at` | writer, for `durable` operations; `INTERRUPTED` at startup |

The `workspace` table is the startup screen's history (T049).
`workspace.recents` reads it newest-first under a caller-supplied limit and
`stat`s each root, so an entry whose directory is gone is shown as unavailable
rather than dropped. `workspace.forget` removes one row, and clears
`session.workspace_root` when it names the same root — otherwise the next
launch would keep retrying a workspace the user just dismissed. It removes
nothing else: the `workspace_tab` and `buffer_snapshot` rows under that root
stay, because unsaved work is never discarded as a side effect (ADR-014).

`workspace_root` is `Workspace.root` (canonical, native form, ADR-010);
`path` is the wire `WorkspacePath`. Ids are never stored. The JSON blobs carry
a producer `version` and are parsed with defaults by the UI
(`packages/editor/src/editor-options.ts`, `features/session/persisted-layout.ts`).

## Startup

```
Core::with_options
 ├─ SessionStore::open(path)        sync: create dir → open → pragmas → migrate
 │    ├─ NOTADB / CORRUPT → move aside → fresh → notice DATABASE_MOVED_ASIDE
 │    ├─ newer schema / migration error / fresh fails → Unavailable (+ notice)
 │    └─ operation_record: CREATED|RUNNING → INTERRUPTED (+ notice when > 0)
 ├─ SessionService::start          writer task + deferred quick_check
 └─ setup: restore_main_window     window_state row → fit_to_monitors → set_size/position/maximize → show()
WebView
 ├─ session.restore                reopen the last root once; tabs, blobs, status, notices
 ├─ documents.list + open           rehydration (always, ADR-009 §11.4)
 ├─ documents.open per persisted tab (active first), then order / view modes / cursors
 ├─ writer enabled; session.update with the actual tabs
 └─ session.candidates per WorkspaceOpened → auto-recover changed/missing, prompt for same-as-disk
```

## Commands and events (protocol 1.3.0)

| Command | Request | Returns | Notes |
|---|---|---|---|
| `session.restore` | – | `{ workspace?, tabs, preferences?, layout?, persistence, notices }` | never rejects for persistence reasons; the first call per process may reopen the last root (`Actor: SYSTEM`, `WorkspaceOpened` follows) |
| `session.candidates` | `{ workspaceId }` | `RecoveryCandidate[]` `{ path, state, updatedAt, size }` | `SAME_AS_DISK \| DISK_CHANGED \| FILE_MISSING`; open + dirty documents are skipped |
| `session.recover` | `{ workspaceId, path }` | `DocumentContent` | buffer replaced, disk untouched; `Conflict` for an open dirty document; `NotFound` without a row |
| `session.discard` | `{ workspaceId, path }` | – | idempotent |
| `session.update` | `{ workspaceId?, tabs?, preferences?, layout? }` | – | `tabs` needs `workspaceId` (`NotFound` for a closed one, ignored by the UI); blobs ≤ 64 KiB, ≤ 500 tabs |

`PersistenceStatusChanged { status: { available, healthy, databasePath?,
message? }, notice? }` is emitted on transitions: writes failing / succeeding
again, a database replaced at runtime, persistence turned off.
`DocumentChanged { cause: RECOVERED }` follows a recovery.

## Snapshot writer

One tokio task; every database call on `spawn_blocking`, one at a time, in
intent order.

- **Trigger**: `DocumentOpened`, `DocumentChanged`, `DocumentSaved` schedule the
  document; `debounce` 1 s after the last change, `max_wait` 2 s after the
  first unwritten change.
- **Write**: `DocumentManager::snapshot(id)` at fire time; dirty → upsert
  (skipped when `current_version` was already written); clean → delete, if
  this process owns the row.
- **Ownership**: a row is owned once this process wrote it or recovered from
  it. Inherited rows survive a clean open and a clean close.
- **Close**: a dirty document closed with `force` deletes its row, except
  when its workspace closed because the root disappeared (`ROOT_MISSING`
  precedes the `DocumentClosed`s in sequence order).
- **Workspace**: `WorkspaceOpened` touches `workspace` and sets the last root;
  a `USER` close clears it; `REPLACED` is followed by the next open;
  `ROOT_MISSING` keeps it.
- **Failure**: the job stays at the head of the queue, backoff 1, 2, 4 … 30 s,
  `PersistenceStatusChanged { healthy: false }` once per streak and `{ healthy:
  true }` on the first success; buffers stay in memory. `Core::shutdown`
  flushes (2 s) before cancelling operations.
- **Overflow**: the tap's channel holds 1024 intents; when it is full the
  writer resynchronizes from `documents.list()` instead of blocking the bus.

## Reconciliation matrix

| Disk vs. snapshot base | `candidates` | `recover` result | Then |
|---|---|---|---|
| same hash | `SAME_AS_DISK` (prompt, default Restore) | buffer = snapshot, dirty, `UNCHANGED` | Save writes it; the row goes with the save |
| different hash | `DISK_CHANGED` (recovered automatically) | buffer = snapshot, `savedHash` = snapshot base, `CONFLICTED` | Reload shows the disk and drops the snapshot; `save { force }` overwrites |
| file missing | `FILE_MISSING` (recovered automatically) | document from the snapshot alone, `DELETED_EXTERNALLY` | `save { force }` recreates the file |
| document open and dirty | skipped | `Conflict` | the live buffer wins; Discard removes the row |

## Not covered

- Up to one UI debounce interval of typing (300 ms; 1 s in large-file mode)
  that has not reached the core yet (ADR-012).
- Snapshots under a root that moved: kept, never offered, never collected.
- `INTERRUPTED` records are reconciled but not listed by `operations.list`.
- Power loss between a commit and the OS flush (`synchronous = NORMAL`).
