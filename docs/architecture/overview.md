# Architecture overview

Status: baseline written at T000. Each contract section names the task that
refines it. The planning documents in `docs/` remain the authority; this page is
the short version agents read first.

## Runtime boundaries

| Component | Responsibility | Must not |
|---|---|---|
| WebView UI (React, Monaco, Markdown) | render, edit, show workstreams/reviews | scan repositories, run agents, block on Git/LSP/PTY work, touch the filesystem |
| Gritty core (Rust, `crates/*`) | workspace lifecycle, commands/events, persistence, supervision | contain provider-specific agent logic in the domain model |
| Desktop shell (`apps/desktop/src-tauri`) | Tauri wiring, tracing, windows, command adapters | hold domain logic |
| Agent process | plan/execute a bounded task with granted tools | mutate UI state directly |
| LSP process | diagnostics/navigation/completion | become required for plain editing |
| PTY/shell | interactive terminals | run inside the renderer |
| Indexer/search worker | incremental search/index | block opening or editing files |
| SQLite | durable app/workstream/session metadata | become the source of truth for repository files |

Tauri already runs an async runtime (`tauri::async_runtime`, tokio-based). Core
async work builds on it; do not create a second runtime.

## Core protocol (fixed by T010, ADR-009)

The UI talks to the core only through typed commands and events, via the
hand-written client in `packages/protocol` whose types are generated from Rust
(`cargo test -p gritty-core` rewrites `packages/protocol/src/generated`; CI fails
when the output is stale). `packages/protocol/src/transport/tauri.ts` is the only
module that imports `@tauri-apps/api`.

| Convention | Shape |
|---|---|
| Command names | `<domain>.<verb>` in TypeScript ↔ `<domain>_<snake_verb>` in Rust; one `request` object per command (`core.health`, `operations.list`, `operations.cancel`, `operations.startDemo`, `workspace.open`, `workspace.close`, `workspace.current`, `workspace.list`, `documents.open`, `documents.close`, `documents.update`, `documents.save`, `documents.reload`, `documents.status`, `documents.list`, `shell.openExternal`) |
| Error envelope | `CoreError { code, message, details?, retryable, correlationId? }`, `code ∈ InvalidInput, NotFound, PermissionDenied, Conflict, Cancelled, Timeout, Io, Internal, ProtocolMismatch` |
| Event envelope | one channel `gritty://core/event`, payload `{ name, seq, workspaceId?, correlationId?, actor, payload }`; `seq` strictly increasing per process |
| Identifiers | UUID v7 strings (`WorkspaceId`, `OperationId`, `DocumentId`, `AgentRunId`, `CorrelationId`), branded in TypeScript |
| Timestamps | milliseconds since the Unix epoch, exported as `number` |
| Actor | `{ type: "USER" } \| { type: "SYSTEM" } \| { type: "AGENT", runId } \| { type: "EXTERNAL" }` |
| Versioning | `PROTOCOL_VERSION` (`MAJOR.MINOR.PATCH`, currently `1.2.0`) reported by `core.health`; the client refuses a different MAJOR with `ProtocolMismatch` |

Every command adapter runs its body behind a guard that creates a correlation id,
opens a tracing span and converts a panic into `Internal`; a promise never hangs.
Nothing is replayed after a renderer reload: the UI subscribes, then calls the
domain's list command (`operations.list`, `workspace.current`, `documents.list`)
to rehydrate.

## Asynchronous operation model

```
UI command → fast validation → operation created → operationId returned
→ supervised background execution → progress events → COMPLETED | FAILED | CANCELLED
```

`Operation { id, kind, actor, workspaceId?, status, createdAt, startedAt?, finishedAt?,
cancellationRequested, correlationId, progress?, error? }` with
`CREATED → RUNNING → COMPLETED | FAILED | CANCELLED` (and `CREATED → CANCELLED`).
The registry (`gritty_core::operation`) runs bodies on Tauri's tokio runtime with a
child `CancellationToken` per operation: cancel before start finishes immediately
and the body never runs; cancel during execution fires the token and aborts the
body task at its next await point; progress after cancellation is dropped; late or
duplicate completions are rejected by the state machine; once cancellation was
requested the only terminal state is `CANCELLED`; a body panic ends the operation
`FAILED` with `Internal`; shutdown cancels everything through a root token.
Progress events are throttled to one per 50 ms per operation, and high-frequency
sources (watchers, terminals, indexing, agent logs) must batch before reaching the
bus. `operations.startDemo` is the reference implementation.

## Workspace (fixed by T020, ADR-010)

One open workspace at a time: `Workspace { id, root, name, openedAt }`. It is
opened by `workspace.open` (the native directory picker runs on the Rust side
when no path is given; `null` means the picker was dismissed), closed by
`workspace.close`, replaced when another root is opened (`WorkspaceClosed` with
reason `REPLACED`), and closed with reason `ROOT_MISSING` when its root
disappears; the command that notices fails with `NotFound`. `workspace.current`
rehydrates the UI after a reload. Events: `WorkspaceOpened { workspace }` and
`WorkspaceClosed { workspaceId, root, reason }`.

Paths on the wire are `WorkspacePath` strings: relative to the root,
`/`-separated, no `.`/`..`, validated while the request is deserialized; the
joined path is canonicalized and must stay inside the root
(`docs/architecture/security.md`). `workspace.list { workspaceId, path, cursor?,
pageSize? }` returns exactly one level: `DirEntry { name, kind: file | dir |
symlink | other, size?, modifiedAt?, hidden, ignoredHint, symlink? }`, directories
first then by name, at most 5000 entries per page (`total`, `truncated`, `cursor`).
The listing runs on the blocking pool with a 100 ms budget; past it the command
returns `{ status: "pending", operation }` (kind `workspace.list`, cancellable) and
the page arrives in `OperationCompleted.result`; otherwise `{ status: "ready", page }`.
`crates/workspace` implements the root, the path policy, the listing and the
fixture generator without depending on `gritty-core`; `gritty_core::workspace`
owns identity, events, the budget hand-over and the error mapping. Deterministic
fixtures live in `fixtures/` (`small-repo` committed, `large-repo` generated).

## Document and buffer model (fixed by T030, ADR-011)

A repository file and an open editable document are different things.

```
Document: id, workspaceId, path, encoding, languageId, savedRevision, savedHash,
          currentVersion, dirty, readOnly, binary, largeFileMode,
          externalModificationState, size
externalModificationState: UNCHANGED | MODIFIED_EXTERNALLY | CONFLICTED | DELETED_EXTERNALLY
```

Repository files are the durable artifacts; the core owns the buffer of every
open document (`gritty_core::document::DocumentManager`); Monaco (T040,
`packages/editor`, ADR-012) only renders and edits. One document per `(workspace,
path)`; `documents.open` on an open path returns the current buffer.
`documents.update { documentId, baseVersion, text }` replaces the buffer and
increments `currentVersion` (a stale `baseVersion` is `Conflict`); `dirty` is
derived from `sha256(buffer) != savedHash`. `documents.save` writes atomically
(temp file → fsync → permissions → rename, `crates/workspace/src/file.rs`) and
fails with `Conflict` when the disk no longer matches `savedHash` or the file
is gone, unless `force`. External modification is checked on demand
(`documents.status`, before save, on reload) by hashing the file; transitions
emit `DocumentExternalChangeDetected`. NUL bytes or invalid UTF-8 make a file
`binary` (no buffer); files ≥ 10 MiB open read-only in `largeFileMode`; files
above 64 MiB are refused. A dirty document, and a workspace with dirty
documents, close only with `force`; documents of a closing workspace emit
`DocumentClosed { reason: WORKSPACE_CLOSED, dirty }`. Details, state matrix
and language map: `docs/architecture/documents.md`.

## Actor model

`USER | SYSTEM | AGENT(runId) | EXTERNAL`, attached to operations, filesystem
changes, command execution, decisions, logs, ChangeSets and agent edits so the
UI can explain who did what. This is attribution, not event sourcing.

## Event vocabulary (Operation* fixed by T010, Workspace{Opened,Closed} by T020, Document* by T030; other families named by their tasks)

Workspace{Opened,Closed,Changed}; Document{Opened,Changed,Saved,ExternalChangeDetected,Closed};
FileSystemBatchChanged; Operation{Started,Progress,Completed,Failed,Cancelled};
GitStatusChanged; ChangeSetUpdated; Workstream{Created,Changed};
AgentRun{Started,Changed,Finished}; Decision{Requested,Resolved}.

Gritty is not event-sourced: events notify; SQLite/domain state and repository
files stay authoritative.

## Git and ChangeSets (T100+)

`Repository`, `Revision`, `ChangeSet`. A ChangeSet is a coherent set of changes
tied to work (workstream, agent run, actor, base revision, files, diff summary,
test status, review status), not the raw output of `git diff`.

## Capabilities (T300+)

Agent permissions are explicit, scoped capabilities
(`filesystem.read`, `filesystem.write` with roots, `process.spawn`,
`network.access`, `git.read`, `git.write`, `workspace.outsideRoot`,
`secrets.access`), translated by adapters into provider mechanisms. Defaults are
narrow; grants are inspectable.
