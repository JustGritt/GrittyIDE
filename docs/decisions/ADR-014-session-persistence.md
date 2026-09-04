# ADR-014: Session persistence and crash recovery

- **Status:** Accepted
- **Date:** 2026-09-04

## Context

ADR-004 chose SQLite with explicit migrations for application metadata and
asked for recovery snapshots that never replace repository files. ADR-011
made the core the owner of every buffer. T060 turns both into code, and the
persistence migration conventions freeze after Phase 0 (addendum §17), so
the concrete choices are recorded here: the library, the migration scheme,
what is persisted under which key, how snapshots are taken and when they are
removed, how a relaunch reconciles them with the disk, and how the layer
fails.

## Decision

- **Library and layout.** `rusqlite` 0.40 with the `bundled` feature (one
  SQLite version compiled into the binary on every platform) behind a
  `Mutex<Connection>` in a new crate, `gritty-persistence`, that knows nothing
  about the core: repositories take and return plain rows. Every database call
  runs on the blocking pool; command handlers and the event loop never touch
  SQLite. `refinery` was rejected (a versioned `include_str!` list is ~60 lines
  and keeps "forward-only, one transaction each" explicit),
  `tauri-plugin-window-state` was rejected (its own JSON file; window geometry
  belongs to the session database).
- **Migrations** are numbered SQL files applied in order, each in its own
  transaction together with its `schema_version` row; the runner owns
  `schema_version`, migrations own domain tables; nothing is ever applied
  partially, and a database with a newer version than the build knows is
  refused, never downgraded. `PRAGMA journal_mode = WAL` (set outside any
  transaction), `synchronous = NORMAL` (durable against a process death, the
  case this task defends against; not against power loss), `foreign_keys =
  ON`, `busy_timeout = 5 s`.
- **Location.** `<app data dir>/gritty.sqlite` for the bundle identifier
  (macOS: `~/Library/Application Support/io.github.justgritt.gritty/`),
  resolved on the Rust side (`tauri::path`); `GRITTY_DATA_DIR` overrides it
  for development and manual crash tests. The WebView never learns a path it
  can act on.
- **Keys.** Everything is keyed by the canonical workspace root and the
  workspace-relative path. `WorkspaceId` and `DocumentId` are per process
  (ADR-009, ADR-010) and never persisted. Tabs are stored per root, so
  returning to a workspace restores its tabs.
- **What is persisted** (schema in `docs/architecture/persistence.md`): the
  last workspace root and every root ever opened; per root the open tabs with
  order, active flag, view mode, cursor, selection anchor and first visible
  line (plain numbers, never Monaco's view-state object); editor preferences
  and the explorer layout as UI-owned JSON blobs with a producer `version`
  (parsed with defaults by the UI, outside the SQL migration convention);
  window geometry per label in physical pixels; durable operation records.
- **Snapshots** are taken from the core's buffer, never from the editor: a
  `FanOutSink` delivers every event to the WebView and to a `SessionTap`
  that turns document, workspace and durable-operation events into small
  intents on a bounded channel (overflow triggers a resynchronization from the
  managers instead of blocking under the bus lock). A single writer task
  debounces per document (1 s quiet, at most 2 s while typing continues),
  reads the buffer at write time and upserts the full text with `savedHash`,
  `currentVersion` and `encoding`; a version already written is skipped. Up to
  one UI debounce interval (300 ms, 1 s in large-file mode, ADR-012) of typing
  lives only in Monaco and is not covered.
- **Removal rules.** A row is removed when the document it belongs to becomes
  clean (save, reload) and this process *owns* the row (it wrote it, or the
  user recovered from it), when a dirty document is closed with `force` (an
  explicit discard, including a forced workspace close), or through
  `session.discard`. A row inherited from a previous process survives a clean
  open and a clean close of the same path: it is a recovery candidate until
  the user answers. A workspace whose root disappeared keeps its rows and
  stays the last workspace (the drive may come back); a user close clears it.
- **Restore protocol** (`1.3.0`, additive): `session.restore` reopens the last
  root once per process (`Actor::SYSTEM`; later calls report what is open, so
  a renderer reload never re-opens anything) and returns tabs, preferences,
  layout, the persistence status and notices; it never rejects for
  persistence reasons, so the UI's rehydration from the core always runs.
  `session.candidates` compares every snapshot of a workspace with the disk by
  hash (`SAME_AS_DISK | DISK_CHANGED | FILE_MISSING`), skipping documents that
  are open and dirty (the live buffer is authoritative). `session.recover`
  puts a snapshot back into the buffer: same base → dirty and `UNCHANGED`;
  changed base → `savedHash` is the snapshot's base and the disk is observed as
  modified, so the document is `CONFLICTED` with both versions reachable
  (reload drops the snapshot, `save { force }` overwrites); missing file → a
  document built from the snapshot alone, `DELETED_EXTERNALLY` until `save {
  force }` recreates it; an open dirty document refuses with `Conflict`.
  Nothing ever touches the disk without a save. `session.update` stores what
  the UI owns; `session.discard` drops a row.
- **UI policy.** The WebView runs one restore per lifetime (StrictMode-safe),
  opens persisted tabs active-first and incrementally, and only then enables
  its coalescing writer (tab order at 100 ms, cursor at 1 s, settings at
  500 ms), so a kill during restoration cannot shrink the next session. After
  every `WorkspaceOpened` it loads the candidates: changed and missing ones are
  recovered automatically (the existing conflict and deleted banners are the
  UI for them, and nothing is written to disk), same-as-disk ones are offered
  in a non-modal prompt whose default is *Restore*.
- **Failure policy.** A file that is not a database or is corrupt at open is
  renamed `gritty.sqlite.corrupt-<unix-ms>` (with its `-wal`/`-shm`) and a
  fresh database takes its place, announced by a notice; `PRAGMA quick_check`
  runs after startup on the blocking pool and a failure replaces the file the
  same way at runtime. A migration failure, a newer schema, or a fresh database
  that fails too leaves the session layer *unavailable*: the editor works,
  every `session.*` command reports it, nothing crash-loops. A failed write
  keeps its job, retries with exponential backoff (1 s … 30 s) while buffers
  stay in memory, and flips `PersistenceStatusChanged` once per streak.
- **Operations.** `OperationStatus::INTERRUPTED` is terminal and assigned only
  by startup reconciliation to durable operations (`OperationSpec.durable`,
  core-side; none is durable yet) that were `CREATED` or `RUNNING` when the
  previous process died. `Operation.durable` does not cross the wire.
- **Window geometry** is captured from `Resized`/`Moved` (a maximized window
  keeps the geometry it returns to and only its flag changes), restored in
  `setup` before the hidden main window is shown, and applied only when the
  window would still be visible on a connected monitor. Claimed for macOS;
  Linux is recorded as untested (Wayland ignores positions, GTK's
  maximize-before-show is unreliable).

## Consequences

- Recovery snapshots persist repository content (whatever the user edited,
  `.env` files included) in plaintext under the application data directory;
  `docs/architecture/security.md` records it.
- A large dirty buffer is rewritten every 2 s while typing continues (a
  12 MiB document costs about 6 MB/s of writes); `current_version` skipping
  removes idle rewrites. Deltas are the follow-up if this shows.
- `INTERRUPTED` rows are reconciled but not yet surfaced in
  `operations.list`; the first durable operation (T200+) adds that.
- Rows under a root that moved are never offered and never garbage-collected
  (never silently discard); a "recent workspaces" view over the `workspace`
  table can expose them.
- Cursor persistence after Cmd+Q relies on the writer's last flush and a
  best-effort `pagehide`; at most one second of cursor movement can be lost.

## Revisit when

Delta updates land (snapshot cost), the first durable operation exists, a
second window appears (geometry per label is ready), or measurements on real
databases move the debounce or the quick_check placement.
