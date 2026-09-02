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

## Core protocol (T010 owns the final form)

The UI talks to the core only through typed commands and events:

```
workspace.open(path)  workspace.list(path)  workspace.read(path)  workspace.write(path, content)
search.query(text, options)     git.status()  git.diff(target)
terminal.spawn(profile)  terminal.write(id, data)
workstreams.create(goal)  workstreams.update(id, patch)
agents.start(workstreamId, config)  agents.cancel(runId)  agents.respond(runId, decision)
```

Conventions T010 must fix: command naming, event envelope, shared identifiers,
error envelope, asynchronous operation semantics, cancellation, serialization
and versioning. Types are exported from Rust with `ts-rs` into
`packages/protocol`, whose hand-written client is the only module importing
`@tauri-apps/api` (ADR-008).

## Asynchronous operation model

```
UI command → fast validation → operation created → operationId returned
→ supervised background execution → progress events → COMPLETED | FAILED | CANCELLED
```

Every cancellable operation carries: id, kind, actor, workspaceId, status,
createdAt, startedAt, finishedAt, cancellationRequested, correlationId. Defined
behavior is required for: cancel before start, cancel during execution, child
process termination, events after cancellation (dropped), duplicate completion
signals, UI reload, shutdown, unexpected worker death. A cancelled operation
never later appears completed. High-frequency sources (watchers, terminals,
indexing, agent logs) are batched/bounded before reaching the renderer.

## Document and buffer model (T030 owns Gate B)

A repository file and an open editable document are different things.

```
Document: id, workspaceId, path, encoding, languageId, savedRevision, savedHash,
          currentVersion, dirty, readOnly, binary, largeFileMode, externalModificationState
externalModificationState: UNCHANGED | MODIFIED_EXTERNALLY | CONFLICTED | DELETED_EXTERNALLY
```

Repository files are the durable artifacts; Gritty owns unsaved buffer state
while a document is open or recoverable; Monaco only renders and edits. Saves
are atomic (temp file → fsync → rename). Conflicts are represented, never
silently resolved.

## Actor model

`USER | SYSTEM | AGENT(runId) | EXTERNAL`, attached to operations, filesystem
changes, command execution, decisions, logs, ChangeSets and agent edits so the
UI can explain who did what. This is attribution, not event sourcing.

## Event vocabulary (illustrative; T010 fixes names)

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
