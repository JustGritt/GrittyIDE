# Task T060 — Session persistence + recovery

- **Phase:** 0 / 1B
- **Status:** review
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

### Summary

Gritty is recoverable. A new `gritty-persistence` crate opens a SQLite
database (`rusqlite` 0.40.2, `bundled`) at the platform app-data directory
(`GRITTY_DATA_DIR` overrides it), in WAL mode, with forward-only numbered
migrations applied one transaction each; a file that is not a database or
fails its deferred `quick_check` is moved aside as `gritty.sqlite.corrupt-<ms>`
and replaced, a newer schema or a failed migration leaves the session layer
unavailable with the editor fully usable, and every failure is a notice, never
a crash. `gritty_core::session` observes the event bus through a fan-out sink
and a bounded intent channel, snapshots every dirty buffer from the core
(1 s debounce, 2 s max while typing), retries failed writes with backoff and
announces `PersistenceStatusChanged`, and only ever deletes rows it owns or
the user discarded. `session.restore` reopens the last workspace once per
process and returns tabs (order, active, view mode, cursor, anchor, scroll),
preferences and layout; `session.candidates` compares each snapshot with the
disk; `session.recover` puts a snapshot back into the buffer as dirty,
`CONFLICTED` (both versions reachable) or `DELETED_EXTERNALLY`, never onto
the disk; `session.discard` and `session.update` complete the domain.
`OperationStatus::INTERRUPTED` exists and startup reconciliation assigns it
to durable operation records. The desktop shell captures window geometry from
window events, restores it before showing the hidden main window (position
only when still on a monitor) and records the post-show geometry. The WebView
runs one restore per lifetime (StrictMode-safe), always rehydrates from the
core, opens persisted tabs active-first, restores view modes and cursors,
persists what it owns through a coalescing writer enabled only after the
restore settled, recovers changed and missing snapshots automatically and asks
about same-as-disk ones in a non-modal prompt whose default is Restore.
Protocol `1.3.0` (additive), ADR-014, `docs/architecture/persistence.md`.

Verified on macOS 26 arm64: `pnpm check` green, the crash simulation and the
adapter round trips in `cargo test`, and two consecutive `pnpm dev` launches on
a scratch data directory where the second one restored the window geometry
the first one recorded and both completed `session.restore`. The agent cannot
see or click the WebView, so the real `kill -9` while typing, the recovery
prompt, tab and cursor restoration after a quit, the corrupted-database notice
and a Cmd+R with dirty tabs are recorded as pending the human pass.

### Files changed

- Root: `Cargo.toml` (`rusqlite`), `Cargo.lock`.
- `crates/persistence` (new, `gritty-persistence`): `Cargo.toml`,
  `src/{lib,db,error,migrations,store,testing}.rs`, `src/migrations/0001_initial.sql`,
  `tests/database.rs`.
- `crates/gritty-core`: `Cargo.toml`, `src/lib.rs`, `src/session/{mod,tap,writer,wire}.rs` (new),
  `src/event/mod.rs` (`FanOutSink`, `PersistenceStatusChanged`), `src/document/mod.rs`
  (`BufferSnapshot`, `RecoveredBuffer`, `snapshot`, `find_by_path`, `open_recovered`,
  `Record::recovered`, `DocumentChangeCause::Recovered`), `src/operation/{state,mod,registry,demo}.rs`
  (`Interrupted`, `durable`), `src/workspace/mod.rs` (`durable: false`),
  `src/protocol/{mod,export}.rs` (`1.3.0`, export rows), `tests/crash_recovery.rs` (new).
- `crates/workspace`: `src/file.rs` (`ContentHash::parse`), `src/detect.rs` (`DocumentEncoding::parse`).
- `apps/desktop/src-tauri`: `Cargo.toml`, `tauri.conf.json` (`visible: false`),
  `src/{core,lib,commands}.rs`, `src/window_state.rs` (new), `tests/commands.rs`.
- `packages/protocol`: `src/{commands,client,types}.ts`, `src/session-client.test.ts` (new),
  `src/{documents,shell}-client.test.ts` (version), `src/generated/*` (13 new, 4 updated).
- `packages/editor`: `src/editor-options.ts` (`parseEditorPreferences`, `serializeEditorPreferences`),
  `src/view-state.ts` (new), `src/index.ts`, `src/monaco/{editor-host,index}.ts`
  (`onSelectionChange`, `restoreView`), tests.
- `apps/desktop/src`: `App.tsx`, `app.css`, `features/session/{use-session.ts,restore-flow.ts,
  session-reducer.ts,session-writer.ts,persisted-layout.ts,recovery-prompt.tsx,session-notices.tsx}`
  (new, with tests), `features/documents/use-documents.ts` (`rehydrate`, `adopt`),
  `features/editor/{editor-pane.tsx,use-editor-binding.ts,editor-preferences.ts}`,
  `features/explorer/{explorer-panel.tsx,use-explorer.ts}`,
  `features/operations/operations-reducer.ts` (+ test).
- Docs: `docs/decisions/ADR-014-session-persistence.md` (new, + README row), ADR-009/012/013 notes,
  `docs/architecture/{persistence.md (new),overview.md,documents.md,security.md}`,
  `docs/testing/platform-matrix.md`, `docs/plans/phase-0-spike.md` (A9, B1, B4),
  `docs/tasks/README.md`, this file.

### Tests executed (macOS 26 arm64, Rust 1.98.0, pnpm 11.25, Node 24)

- `pnpm check`: Biome clean (149 files); `tsc` for the root, `@gritty/protocol`, `@gritty/editor`,
  `@gritty/markdown`, `@gritty/desktop`; Vitest 36 files / 198 tests (+29: session reducer,
  writer with fake timers, layout blob, restore flow with fakes incl. fresh start, reload path,
  failed restore and failed rehydration, protocol session client, preference parsing, view
  state); `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`:
  `gritty-persistence` 7 unit + 11 integration (migrate from empty and from each prior version,
  rollback of a failing migration, newer schema refused, garbage file moved aside with its
  journal, damaged page → `quick_check` → replaced at runtime, two handles on one file,
  repository CRUD incl. `mark_interrupted`); `gritty-core` 90 lib (+21 session: debounce and
  max-wait, save removes the row, forced close removes it, `ROOT_MISSING` keeps rows and the last
  root, inherited rows survive a clean open and close, restore reopens once and reports a missing
  root, candidates matrix incl. dirty-open skip, recover in all three states plus force overwrite
  and reload, dirty-open refusal and discard, write failures back off with exactly one status
  transition per streak, unavailable store, durable operation → `INTERRUPTED` on the next start,
  garbage database reported by restore, damaged database replaced by the integrity check and
  announced, window geometry round trip, request validation) + 3 in `tests/crash_recovery.rs`
  (process state dropped without close, flush or checkpoint; same / changed / missing);
  `gritty-desktop` 10 unit (+4 `fit_to_monitors`) + 19 integration (+3: restore after a restart on
  the same data dir incl. idempotence and the `WorkspaceOpened` event, recovery round trip with
  `RECOVERED` and `CONFLICTED`, request validation without side effects); `check:generated` clean.
- `GRITTY_DATA_DIR=<scratch> GRITTY_LOG=debug pnpm dev`, twice (2026-09-04): run 1 `applied
  migration version=1`, `database open`, `quick_check` passed, `health requested by webview`,
  `session restored workspace=None tabs=0 available=true notices=0`; run 2 `restoring window
  geometry width=2560 height=1600 position=Some((232, 142)) maximized=false monitors=1`, then the
  same health and restore lines; the data directory held `gritty.sqlite` (+`-wal`, `-shm`),
  `window_state` = `main|232|142|2560|1600|2.0|0`, `schema_version` = 1, `journal_mode` = `wal`.
- Linux CI on PR #7 (2026-09-04, run 33856323276, head 59f1c15): `pnpm check` and
  `pnpm tauri build --no-bundle` passed in 16m19s (rusqlite `bundled` compiles SQLite on the
  Ubuntu runner; the crash simulation, the session tests and the adapter tests hold on Linux).
  The macOS job is label-gated (`ci:macos`) and did not run.
- Not executed (human WKWebView pass, rows in the platform matrix): `kill -9 $(pgrep -f Gritty)`
  while typing → relaunch → prompt → Restore puts the text back dirty; the same with the file
  edited or deleted meanwhile (CONFLICTED / DELETED banners); quit and relaunch with several tabs,
  a Markdown tab in preview mode, a cursor deep in a file and changed preferences; a corrupted
  `gritty.sqlite` (write garbage into it) → clean start with the moved-aside notice; Cmd+R in
  `pnpm dev` with dirty tabs (§11.4); a window moved to a disconnected monitor comes back on
  screen; `pnpm tauri build --debug --bundles app` uses the real app-data directory.

### Architectural decisions

- ADR-014: `rusqlite` bundled behind a mutex in a core-agnostic crate; hand-rolled numbered
  migrations, one transaction each, runner-owned `schema_version`; keys are root + path, ids never
  persisted; snapshots from the core's buffer through a fan-out sink and bounded intents; row
  ownership rules (inherited rows are candidates until answered); restore protocol and the
  reconciliation matrix; UI-owned JSON blobs with a producer version outside the SQL migration
  convention; failure policy (move aside, unavailable, backoff, status event); `INTERRUPTED`
  reconciled at startup; window geometry by hand rather than `tauri-plugin-window-state`.
- `session.restore` never rejects for persistence reasons and is idempotent per process, so
  the §11.4 rehydration path is untouched and React StrictMode's double effect is harmless.
- Candidates are a separate command called after every `WorkspaceOpened`, not part of
  `restore`: probing every snapshot (read + hash) stays off the "UI first" path and rows of a
  workspace opened later in the same process are offered too.
- `Operation.durable` is core-side only (`serde(skip)`, `ts(skip)`): the field exists for the
  reconciliation without widening the wire.

### Deviations from the task text

- Allowed scope: T060 also touched `crates/workspace` (two `parse` helpers for values read
  back from storage) and `packages/editor` (plain view state, preference parsing, two host
  methods), plus the documents, editor, explorer and operations features in the WebView to lift
  the persisted state into `App`. All additive.
- Required behavior 3 is met from the core's buffer; the ≤ 300 ms (1 s large-file) UI debounce
  of ADR-012 is not covered (documented in ADR-014 and `persistence.md`).
- Required behavior 6: the durable field and the reconciliation exist; `INTERRUPTED` rows are not
  yet surfaced by `operations.list` (follow-up), and no operation is durable yet.
- Window geometry is claimed for macOS only (rule 11); Linux has a matrix row marked untested.

### Known limitations

- A large dirty buffer is rewritten every 2 s while typing continues (about 6 MB/s for 12 MiB).
- Snapshots under a root that moved are kept but never offered.
- `synchronous = NORMAL`: durable against process death, not against power loss between a
  commit and the OS flush.
- Cursor persistence after Cmd+Q relies on the writer's last flush (cursor writes trail by 1 s)
  and a best-effort `pagehide`.
- A database replaced at runtime after a failed `quick_check` loses the rows it held; a
  replacement that fails leaves an in-memory placeholder and the status says unavailable.

### Follow-up tasks

- Delta updates (`edits[]`, ADR-011) to close the UI debounce gap and cut snapshot cost.
- Surface `INTERRUPTED` records in `operations.list` when the first durable operation lands (T200+).
- "Recent workspaces" over the `workspace` table, including snapshots left under moved roots.
- Sidebar and panel layout in the `layout` blob once such layout exists.
- Temporary-file cleanup from ADR-011 (`.<name>.gritty-*.tmp`).
- Linux verification of window-state restore (Wayland positions, GTK maximize).

### Remaining risks

- The human pass has not exercised the prompt, banners and geometry in WKWebView.
- `quick_check` on a multi-hundred-MiB database runs once per start on the blocking pool; if
  snapshot rows grow that large it may deserve a size guard.
