# ADR-011: Document/buffer contract (Gate B)

- **Status:** Accepted
- **Date:** 2026-09-03

## Context

T030 introduces the domain that every editing feature builds on: Monaco
(T040), Markdown (T050), crash recovery (T060), the watcher (T070) and agent
edits later. The architecture addendum (§4) fixes the intent: a repository
file and an open document are different things, the core owns unsaved
buffer state, Monaco is never the source of truth, saves are atomic, and
dirty-buffer versus external-file conflicts are represented, never resolved
silently. After Phase 0 the document identity and the shape of the contract
are frozen (addendum §17), so the concrete choices are recorded here.

## Decision

- **Identity.** A document is `(workspaceId, WorkspacePath)`, identified by a
  `DocumentId` (UUID v7) for its lifetime in the core. Opening a path that is
  already open returns the same document with its *current buffer*, not the
  disk content; this is also how the UI rehydrates after a renderer reload
  (`documents.list`, then `documents.open` per document).
- **Buffer ownership.** The core holds the text of every open text document.
  The UI sends the full text with `documents.update { documentId, baseVersion,
  text }`, debounced (300 ms in the desktop shell). `currentVersion` starts at
  1 and increments on every accepted update and on reload; an update whose
  `baseVersion` is not the current version is rejected with `Conflict`. The
  upgrade path to deltas keeps `baseVersion` and adds an `edits` array next
  to `text`.
- **Dirty is derived**: `sha256(buffer as it would be written) != savedHash`,
  never a stored flag. `savedRevision` counts synchronizations with the disk
  (open, save, reload). Hashes are SHA-256 (`sha256:<hex>`) because repository
  content is untrusted input and `sha2` was already in the dependency tree.
- **Detection.** A file is text only when it contains no NUL byte and is
  valid UTF-8 as a whole; otherwise it is `binary` and has no buffer, so it
  can never be edited or rewritten. A UTF-8 BOM is stripped from the text and
  written back on save (`encoding: "utf-8-bom"`). Files at or above 10 MiB
  open in `largeFileMode` and are read-only until the UI reopens them with
  `largeFileEditing`; the mode is decided when the buffer is synchronized
  from disk (open, reload), not after each save. Files above 64 MiB are
  refused (`InvalidInput`, `reason: "too large"`). Only UTF-8 is supported;
  UTF-16 files appear as binary.
- **External modification** is checked on demand, never assumed: on
  `documents.status`, before every save, and on reload. The check hashes the
  file (no reliance on mtime granularity). An internal disk state
  (unchanged / modified / deleted) combined with `dirty` yields the wire
  state `UNCHANGED | MODIFIED_EXTERNALLY | CONFLICTED | DELETED_EXTERNALLY`;
  transitions emit `DocumentExternalChangeDetected { document, previousState }`
  with actor `EXTERNAL`. Watcher-driven checks arrive with T070 through the
  same path.
- **Saves are atomic**: temporary file `.<name>.gritty-<pid>-<n>.tmp` in the
  destination's directory → `fsync` → destination permissions copied →
  `rename` → directory `fsync` (Unix). The write goes to the resolved
  (canonical) path, so a symlink inside the root keeps pointing at its
  target. A save whose disk hash differs from `savedHash`, or whose file is
  gone, fails with `Conflict` and moves the state; `save { force: true }`
  overwrites or recreates. A failed save leaves buffer, dirty flag and
  destination untouched and removes the temporary file; a process death
  between write and rename leaves the destination untouched and the
  temporary file behind (recoverable content, cleanup is a follow-up).
- **Nothing is dropped silently.** `documents.close` refuses a dirty document
  without `force`; `workspace.close` and a replacing `workspace.open` consult
  a close guard installed by the document manager and refuse with `Conflict`
  (listing `dirtyDocuments`) unless `force: true`. When a workspace closes
  for any reason, its documents close with `DocumentClosed { …, reason:
  WORKSPACE_CLOSED, dirty }` so the UI can keep the text in view; a root that
  disappears therefore loses no text the UI already had, and T060 snapshots
  will close the remaining gap.
- **Read-only** means edits are refused (`PermissionDenied`): the file has
  no write permission bit, or it is large and editing was not enabled.
- **Language ids** come from a small file-name / extension map in
  `crates/workspace/src/detect.rs` (VS Code / Monaco vocabulary, `plaintext`
  by default); see `docs/architecture/documents.md`.
- **Not operations.** Document I/O runs on the blocking pool, bounded by the
  64 MiB limit; a save must run to completion (the rename is the atomic step)
  and a bounded read takes at most a few hundred milliseconds, so none of it
  is modelled as a cancellable operation. Per document, disk work is
  serialized by an async lock; in-memory state is never held across I/O.
- **Crate layout.** `gritty-workspace` owns detection, hashing, bounded reads,
  the disk probe and the atomic write, plus the `weird-files` fixture
  stager; `gritty_core::document` owns identity, versions, states, events,
  the close guard and the mapping onto `CoreError`.
- **Protocol version** moves to `1.1.0` (additive: seven `documents.*`
  commands, five `Document*` events, `force` on the two workspace requests).

## Consequences

- Monaco (T040) binds a model to `DocumentContent.text`, forwards edits with
  the version it derived them from, and takes `dirty` from the core.
- Every writer of repository files inside Gritty (agents included) must go
  through the same atomic write and the same conflict rules.
- The text of a large document crosses the IPC boundary as one string; a
  paged or streamed representation is needed before the limit can grow.
- Line endings are opaque: the buffer is stored and written verbatim. A UI
  that normalizes EOLs (Monaco does by default) would make a document dirty
  on the first edit; T040 must configure the model accordingly.
- Two documents can be opened on one physical file through a symlink and a
  direct path; they are independent buffers and will conflict at save time.

## Revisit when

Deltas replace full-text updates, encodings beyond UTF-8 are needed, the
64 MiB bound or the 10 MiB threshold prove wrong, or the watcher (T070) shows
that on-demand checks miss cases users hit.
