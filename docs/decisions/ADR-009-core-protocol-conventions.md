# ADR-009: Core protocol conventions (ids, envelopes, events, versioning)

- **Status:** Accepted
- **Date:** 2026-09-03

## Context

T010 (Gate A) fixes the contract every later task consumes: how commands are
named, what an error looks like, how events reach the WebView, how things are
identified and how the two sides detect incompatibility. After Phase 0 these
shapes are frozen (`AGENTS.md`), so the choices are recorded here. Rust owns
the schema; `ts-rs` generates the TypeScript side (ADR-008).

## Decision

- **Commands** are `<domain>.<verb>` in TypeScript and `<domain>_<snake_verb>`
  in Rust (Tauri command identifiers cannot contain dots). A command with
  arguments takes exactly one `request` object whose type is generated from
  Rust; the adapter deserializes it itself so a malformed request is
  `InvalidInput`, not Tauri's opaque string rejection.
- **Error envelope**: every command returns `Result<T, CoreError>` with
  `{ code, message, details?, retryable, correlationId? }` and a closed
  `CoreErrorCode` (`InvalidInput | NotFound | PermissionDenied | Conflict |
  Cancelled | Timeout | Io | Internal | ProtocolMismatch`). Adapters run every
  command body on the async runtime behind a guard that attaches a correlation
  id and turns a panic into `Internal`; a promise never hangs.
- **Identifiers** are newtype strings generated as **UUID v7** (RFC 9562,
  hyphenated lowercase): time-ordered, standard, and `uuid` was already in the
  dependency tree (ULID would have added a crate). TypeScript sees branded
  `string` aliases (`OperationId`, `WorkspaceId`, `AgentRunId`,
  `CorrelationId`) so ids cannot be mixed up at compile time.
- **Timestamps** are milliseconds since the Unix epoch (`Timestamp`, exported
  as `number`). 64-bit integers are exported as `number` and must stay below
  2^53; nothing in the protocol needs more.
- **Actor** is a tagged union `{ type: "USER" | "SYSTEM" | "EXTERNAL" } |
  { type: "AGENT", runId }`.
- **Events** travel on **one** Tauri channel, `gritty://core/event`, as a flat
  envelope `{ name, seq, workspaceId?, correlationId?, actor, payload }`.
  `seq` is assigned and delivered under one lock, so it is strictly increasing
  in delivery order per process. The client demultiplexes by `name` and reports
  sequence discontinuities. Nothing is replayed after a renderer reload; the UI
  rehydrates through list commands (`operations.list`). Progress events are
  throttled per operation (one per 50 ms); high-frequency sources must batch
  before they reach the bus.
- **Operations** follow `CREATED → RUNNING → COMPLETED | FAILED | CANCELLED`
  (plus `CREATED → CANCELLED`). Cancellation is cooperative through a child
  `CancellationToken`; the body task is also aborted at its next await point.
  Once cancellation was requested the only terminal state is `CANCELLED`, and
  the state machine rejects any later or duplicate completion signal. Finished
  operations are retained in memory (last 200) for rehydration; persistence is
  T060's decision.
- **Versioning**: `PROTOCOL_VERSION` (`MAJOR.MINOR.PATCH`; `1.1.0` since T030
  added the `documents.*` family, `1.2.0` since T050 added `shell.openExternal`,
  `1.3.0` since T060 added `session.*` and `PersistenceStatusChanged`)
  lives in Rust and is
  reported by `core.health`; the same test that exports the types writes it to
  `packages/protocol/src/generated/version.ts`. The client probes `core.health`
  before its first command and refuses to run (every command rejects with
  `ProtocolMismatch`) when the MAJOR differs. Additive changes bump MINOR.
- **Generation**: `cargo test -p gritty-core` rewrites
  `packages/protocol/src/generated/`; `pnpm check:generated` fails when the
  committed output is stale. The generated directory is excluded from Biome.
- **Boundary**: `packages/protocol/src/transport/tauri.ts` is the only module
  importing `@tauri-apps/api`, enforced by Biome `noRestrictedImports` and by
  the dependency graph (`apps/desktop` no longer depends on `@tauri-apps/api`).

## Consequences

- New domains add a request/response type in Rust, a row in `COMMANDS`, and a
  client method; the TypeScript types appear on the next `cargo test`.
- Breaking a shape means bumping MAJOR and updating this ADR.
- The single event channel keeps ordering trivial but means every event is
  serialized for the WebView even when nothing listens to its family; if that
  becomes measurable, split by family behind the same client API.

## Revisit when

A domain needs events the WebView must not miss across reloads (then add a
replay/list pair for that domain), or `seq` gaps are observed in practice.
