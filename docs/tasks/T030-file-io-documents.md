# Task T030 — File I/O + document/buffer contract (Gate B)

- **Phase:** 0 / 1A
- **Status:** review
- **Depends on:** T020

## Objective

Introduce the document/buffer domain: safe read with UTF-8/binary detection,
explicit dirty state owned by the core, atomic saves, external modification
states, and the inputs crash recovery (T060) needs. Monaco (T040) must consume
this contract instead of becoming the source of truth.

## Dependencies

T020 (workspace root, path validation, fixtures).

## Required reading

- Addendum §4 (document model, source of truth, external modification conflict,
  safe writes), §2.1 Gate B, §10.3 weird-files, §16 rules 3 and 7
- Plan §8.1 (large-file guardrails), §11 (atomic writes), §12
- ADR-003, ADR-004

## Allowed scope

- `crates/workspace` (file read/write/stat, atomic save, hashing)
- `crates/gritty-core` (document registry, states, events)
- `packages/protocol` (`documents.open/close/update/save/reload/status`)
- `apps/desktop/src-tauri` adapters
- `apps/desktop/src/**` minimal plain-textarea editing surface (until T040) or a test harness
- new `fixtures/weird-files/**`
- dependency additions with justification (e.g. a fast hash crate, `tempfile` for atomic writes)

## Explicit non-goals

- Monaco integration (T040), Markdown (T050), persistence of buffers to SQLite (T060),
  filesystem watching (T070; detection here is on-demand), encodings beyond UTF-8
  (documented as unsupported for now), merge UI (later).

## Required behavior

1. `documents.open(path)` returns `Document { id, workspaceId, path, encoding, languageId,
   savedRevision, savedHash, currentVersion, dirty, readOnly, binary, largeFileMode,
   externalModificationState }` plus the text content for text files.
2. Detection: NUL bytes or invalid UTF-8 → `binary: true`, no content returned, editor
   shows a placeholder; UTF-8 BOM preserved on save; files above a threshold (default
   10 MB) open with `largeFileMode: true` (read-only until explicitly enabled).
3. Buffer ownership: the UI sends edits as `documents.update(id, baseVersion, text)`
   (debounced full text is acceptable for now; document the upgrade path to deltas);
   the core stores the current text, increments `currentVersion`, sets `dirty`
   (`currentHash != savedHash`) and emits `DocumentChanged`.
4. Save: `documents.save(id)` writes the core's buffer atomically (temp file in the same
   directory → fsync → rename → preserve permissions), updates `savedHash/Revision`,
   emits `DocumentSaved`. If the on-disk hash differs from `savedHash` at save time,
   save fails with `Conflict` and the state becomes `CONFLICTED`; `save({ force: true })`
   overwrites only when the UI asked explicitly.
5. External modification: on `documents.status(id)`, on window focus, and before save,
   compare disk mtime/hash to `savedHash`: `UNCHANGED | MODIFIED_EXTERNALLY | CONFLICTED
   (dirty + modified) | DELETED_EXTERNALLY`; emit `DocumentExternalChangeDetected`. The
   UI offers reload / keep buffer (minimal), never silently picks one.
6. `documents.reload(id)` replaces the buffer from disk only when the UI asks.
7. Language id from extension/filename map (small, documented).
8. `fixtures/weird-files`: Unicode names, spaces, very long lines, a large text file,
   binary file, invalid UTF-8, hidden file, symlink, broken symlink, deep path,
   permission-denied file (generated at test time where needed).

## Failure behavior

- Read of unreadable/permission-denied file → `PermissionDenied`, no document created.
- Write failures (read-only file, disk full, directory vanished) → error envelope, buffer
  and dirty state preserved, no temp file left behind.
- File deleted externally while dirty → `DELETED_EXTERNALLY`; save recreates the file only
  with `force`.
- App crash between temp write and rename → destination untouched (verified by test with
  an injected failure).

## Acceptance criteria

- Edit → save → reload round trip on `small-repo`; external edit with `sed` while dirty is
  detected and reported as `CONFLICTED`; no data loss in any path.
- `weird-files` fixture opens/lists correctly; binary and invalid UTF-8 never corrupt.
- No temp files remain after successful or failed saves.

## Tests required

- Rust unit tests: detection (binary/BOM/large), hashing, state transitions.
- Rust integration tests: atomic save with injected failure, external modification matrix.
- Vitest: document store reducer / dirty indicator logic.
- Manual: force-quit during typing does not corrupt the file (recovery itself is T060).

## Commands to run

`pnpm check`, `pnpm dev`.

## Completion report

### Summary

The document/buffer domain (Gate B) is implemented end to end. `gritty-workspace`
gained the file primitives: bounded whole-file reads with binary / UTF-8 / BOM
detection, SHA-256 content hashes, a disk probe, `resolve_for_write`, an atomic
write (temporary file in the destination directory → fsync → permissions →
rename → directory fsync) with an injectable fault, and the language id map.
`gritty_core::document::DocumentManager` owns every open buffer: one document
per `(workspace, path)`, `currentVersion` incremented per accepted full-text
update (stale `baseVersion` → `Conflict`), `dirty` derived from hashes, the
four external modification states derived from an on-demand disk check plus
dirtiness, atomic saves that refuse to overwrite a changed or deleted file
without `force`, reload only on request, and five `Document*` events. It
installs itself as the workspace close guard so `workspace.close` and a
replacing `workspace.open` refuse (`Conflict`, listing the dirty documents)
unless forced, and documents of a closing workspace emit `DocumentClosed {
dirty }`. Seven `documents.*` commands are exposed through the Tauri adapters
and the typed client (protocol `1.1.0`). The desktop shell has a plain-textarea
editing surface (tabs, debounced versioned updates, save / overwrite / reload /
keep, binary and large-file placeholders, orphaned buffers) opened from the
explorer's file names. `fixtures/weird-files` is committed with a stager that
adds the generated 12 MiB text and mode-000 members. Decisions are recorded in
ADR-011 and `docs/architecture/documents.md`.

Everything was verified on macOS 26 (arm64); the previous task's verification
was on Windows 11, so this branch is also the first time the T020 code and
tests ran on macOS (all green).

### Files changed

- Root: `Cargo.toml` (`sha2` pin), `Cargo.lock`, `.gitattributes`
  (`fixtures/weird-files/** -text`), `.gitignore` (`generated/`), `biome.json`
  (skip the fixture: its broken symlink is test data).
- `crates/workspace`: new `src/detect.rs`, `src/file.rs`,
  `examples/generate_weird_files.rs`, `tests/documents.rs`; `src/error.rs`
  (`NotAFile`, `TooLarge`), `src/root.rs` (`resolve_for_write`),
  `src/fixtures.rs` (weird-files stager), `src/lib.rs`, `Cargo.toml`.
- `crates/gritty-core`: new `src/document/mod.rs`; `src/event/mod.rs` (five
  `Document*` events, `document_id()`), `src/workspace/mod.rs`
  (`UnsavedPolicy`, `WorkspaceCloseGuard`, `force` on open/close requests,
  `pub(crate) require`, error mapping), `src/protocol/{ids,mod,export}.rs`
  (`DocumentId`, `PROTOCOL_VERSION = 1.1.0`, exports), `src/lib.rs`.
- `apps/desktop/src-tauri`: `src/commands.rs` (seven adapters, `force`
  plumbing), `src/core.rs`, `src/lib.rs`, `tests/commands.rs` (three document
  scenarios).
- `packages/protocol`: `src/{client,commands,types}.ts`, new
  `src/documents-client.test.ts`, 19 new files under `src/generated/` plus
  `CoreEvent.ts`, `OpenWorkspaceRequest.ts`, `CloseWorkspaceRequest.ts`,
  `version.ts` (written by `cargo test -p gritty-core`).
- `apps/desktop/src`: new
  `features/documents/{documents-reducer.ts,documents-reducer.test.ts,use-documents.ts,document-panel.tsx}`;
  `features/explorer/{explorer-panel.tsx,use-explorer.ts}` (file click,
  discard-and-retry after a `Conflict`), `App.tsx`, `app.css`.
- Fixtures: new `fixtures/weird-files/**` (26 committed entries incl. three
  symlinks), `fixtures/README.md`.
- Docs: new `docs/decisions/ADR-011-document-buffer-contract.md`,
  `docs/architecture/documents.md`; `docs/architecture/{overview,security}.md`,
  `docs/decisions/{README,ADR-009-core-protocol-conventions}.md`,
  `docs/testing/platform-matrix.md`, `docs/plans/phase-0-spike.md` (A5),
  `docs/tasks/README.md`, this file; T020 flipped to done.

### Tests executed (macOS 26 arm64, Rust 1.98.0, pnpm 11.25, Node 24)

- `pnpm check`: Biome (59 files), tsc for the three projects, vitest,
  `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`,
  `cargo test --workspace` and `check:generated` all passed (the last one
  after the generated bindings were committed).
- `cargo test -p gritty-workspace`: 22 unit tests (detection incl. NUL /
  invalid UTF-8 / UTF-16 / BOM, encode/decode round trip, language ids and
  table uniqueness, hash format and `of_text`, temp-file naming, path matrix,
  root, cursor) and 14 integration tests in `tests/documents.rs` against the
  staged fixture (every category listed and read with the expected encoding,
  binary never yields text, 12 MiB file flagged large and the read bound
  enforced, mode-000 file → `PermissionDenied`, directories / root / missing
  paths rejected, symlinks followed only inside the root and an escaping link
  refused before any read, atomic write replaces content with no temp file,
  permissions preserved incl. a 0444 file, injected failure before rename
  keeps the destination and cleans up, simulated crash before rename keeps
  the destination and leaves exactly the flushed temp file, write into a
  vanished directory fails without leftovers, `resolve_for_write` creates
  only inside existing directories and writes through a symlinked directory,
  probe distinguishes missing / present / changed at equal size) plus the 11
  existing listing tests.
- `cargo test -p gritty-core`: 64 passed, 11 new in `document::tests`: open
  returns text and announces (idempotent reopen returns the live buffer),
  update versions and derives dirty (stale base → `Conflict`, disk untouched,
  size bound), save preserves the BOM and emits `DocumentSaved`, the external
  modification matrix (UNCHANGED → MODIFIED_EXTERNALLY at equal size →
  CONFLICTED on edit → save refused → DELETED_EXTERNALLY → forced recreate →
  forced overwrite), reload replaces only when asked and rejects the in-flight
  stale edit, binary and large guards (read-only until `largeFileEditing`,
  mode decided at sync time), dirty documents never dropped silently
  (document close, workspace close and replacement all `Conflict` without
  force; discard flags `dirty: true`), root missing / replacement close the
  documents, injected write failure preserves buffer, dirty flag and disk with
  no temp file, open failures create no document (incl. mode 000), wire
  types.
- `cargo test -p gritty-desktop`: 5 unit + 13 mock-runtime adapter tests (3
  new: open → update → save → list → close round trip through the ACL with
  events; `sed`-style external edit while dirty → `CONFLICTED`, save and
  workspace close refused, forced save, status of all, reload, workspace close
  closing the document; request validation before any disk access).
- vitest: 55 tests in 10 files (6 new documents reducer tests: tabs and
  reopening, draft/unsaved semantics across in-flight updates, binary and
  read-only refuse drafts, external state and keep-buffer, closing / orphans,
  failures; 3 new protocol tests for the `documents.*` namespace and `force`).
- `pnpm dev` on macOS: the app compiled with the new commands, started
  (`starting Gritty … protocol_version="1.1.0"`) and the WebView (WKWebView
  21624.4.5.11.5) completed the `core.health` probe; the process tree was then
  stopped. The explorer → textarea → save → external edit → banner flow was
  not driven by hand (the agent has no WebView access on this machine) and is
  pending in `docs/testing/platform-matrix.md`.
- Manual "force-quit during typing does not corrupt the file": not executed.
  By construction `documents.update` never touches the disk and the write path
  is covered by the injected failure/crash tests; the reviewer's manual pass
  is still owed.

### Architectural decisions

- ADR-011 records the contract: identity per `(workspace, path)`, idempotent
  open returning the live buffer (also the rehydration path), full-text
  versioned updates with the delta upgrade path, hash-derived dirty state,
  SHA-256 hashes, detection rules and limits (binary on NUL or invalid UTF-8,
  BOM round trip, 10 MiB large-file mode decided at sync time, 64 MiB refusal),
  on-demand hashing for external changes (no mtime reliance), atomic write
  semantics, `force` everywhere something would be lost, and the workspace
  close guard.
- Document I/O is not an operation: bounded reads and non-interruptible atomic
  saves run on the blocking pool behind a per-document async lock; the
  reasoning is in the module documentation and ADR-011.
- `DocumentExternalChangeDetected` is attributed to `EXTERNAL`, emitted on
  transitions only, and carries `previousState`.
- `DocumentClosed` carries `dirty` so the UI can keep the text of a buffer the
  core had to drop (workspace gone) in view; T060 makes this durable.

### Deviations from the task text

- `documents.list` was added to the six named commands: the ADR-009 convention
  (no replay, rehydrate through list commands) needs it after a renderer
  reload.
- `documents.status` takes an optional `documentId` and returns `Document[]`
  so one call covers the "on window focus" case.
- `force?: boolean` was added to `OpenWorkspaceRequest` and
  `CloseWorkspaceRequest` (T020 shapes, additive, MINOR bump) so that rule 7
  holds for workspace close/replace as well; `WorkspaceManager::open/close`
  take an `UnsavedPolicy`.
- No `tempfile` crate: the atomic write is 60 lines of `std` with an explicit
  temp-file name pattern. The hash is `sha2` (already in the tree through
  `tauri-codegen`) rather than a new fast-hash crate, because repository
  content is untrusted input.
- `biome.json` (outside the listed scope) gained `!fixtures/weird-files`; the
  fixture's dangling symlink otherwise produces a Biome warning on every run.
- Detection reads and validates the whole file rather than sniffing a prefix,
  so a file that turns binary after 8 KiB can never be rewritten as text.

### Dependencies

- `sha2` 0.10.9 as a runtime dependency of `gritty-workspace` (content hashes).
  Already present in `Cargo.lock` via `tauri-codegen`; no new crate.

### Known limitations

- Line endings are opaque: the buffer is written verbatim, and a UI that
  normalizes EOLs (Monaco's default) would dirty a document on the first
  edit. T040 must configure the model to keep EOLs.
- The text of a document crosses the IPC boundary as a single string on open,
  reload and every debounced update; fine below the 64 MiB bound, but a paged
  or delta representation is needed before that bound can grow.
- Up to one debounce interval (300 ms) of typing lives only in the WebView
  before it reaches the core; a force-quit inside that window loses it. T060
  snapshots do not close this gap either; a shorter debounce or per-keystroke
  deltas would.
- `documents.status` hashes each file in full, sequentially; with many large
  open documents a focus check costs O(total size).
- `readOnly` is derived from permission bits only (`Permissions::readonly`),
  so a file owned by another user with owner-write set is reported writable
  and fails at save time; a read-only-by-permission file cannot be edited at
  all in Gritty yet (no "overwrite anyway").
- A crash between the temp write and the rename leaves
  `.<name>.gritty-<pid>-<n>.tmp` behind; nothing cleans it up yet.
- Two documents can be opened on the same physical file (symlink and direct
  path); they are independent and conflict at save time.
- A workspace whose root disappears closes its documents; dirty text survives
  only in the UI's orphan list until dismissed.
- Only UTF-8 is decoded; UTF-16 and legacy encodings appear as binary.
- The desktop editing surface is a textarea: no undo history beyond the
  browser's, no syntax highlighting, no keyboard shortcut for save.

### Follow-up tasks

- T040: bind Monaco models to `DocumentContent`, forward edits with
  `baseVersion`, take `dirty` from the core, keep EOLs, honour `readOnly` /
  `largeFileMode`, map unknown language ids to `plaintext`, Cmd+S.
- T060: dirty-buffer snapshots so a workspace closing for `ROOT_MISSING`, or a
  crash, never loses text; consider the debounce window.
- T070: feed watcher events into the same disk-check path
  (`DocumentManager::observe`).
- Clean up stale `.*.gritty-*.tmp` files (on open of the same path, or a
  workspace-level sweep) after a confirmed crash.
- Delta updates (`edits` next to `text`) once Monaco is in place.
- `documents.create` / save-as and overwrite of read-only-by-permission files.
- Parallel `documents.status` for many documents; skip the hash when size and
  mtime match if measurements show the focus check is too slow.
- Manual macOS UI pass (explorer → textarea → save → external edit → banner →
  reload/keep/overwrite; force-quit while typing) and the platform matrix rows;
  Linux and Windows runs of the new tests (Linux CI runs on push).
- Explorer: keyboard navigation and a visible "open" affordance instead of the
  bare file-name button.

### Remaining risks

- Verified on macOS only in this session; Linux runs through CI on push,
  Windows has not run the new tests (the stager recreates symlinks and skips
  the mode-000 file there by design).
- The mode-000 assertions are skipped when tests run as root.
- `fs::rename` replacement semantics on network filesystems and on Windows
  (`MoveFileEx` replace) are assumed from the platform APIs, not measured.
- The workspace close guard runs under the workspace lock and takes the
  document table lock; the lock order (workspace → table → record) is
  documented in the module and must be preserved by future code.
