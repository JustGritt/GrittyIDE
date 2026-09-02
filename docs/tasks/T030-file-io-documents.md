# Task T030 — File I/O + document/buffer contract (Gate B)

- **Phase:** 0 / 1A
- **Status:** todo
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

(fill in)
