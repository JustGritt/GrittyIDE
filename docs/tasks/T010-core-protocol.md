# Task T010 — Core protocol + operation/event/error conventions (Gate A)

- **Phase:** 0
- **Status:** in-progress
- **Depends on:** T000

## Objective

Establish the typed contract between the WebView and the core that every later
feature consumes instead of reinterpreting: command conventions, event
envelope, shared identifiers, error envelope, asynchronous operation semantics,
cancellation semantics, serialization/versioning; plus the tooling that keeps
Rust and TypeScript in sync (`ts-rs` export) and the hand-written typed client
(`packages/protocol`).

## Dependencies

T000 (workspace, `pnpm check`, the temporary `health` command in
`apps/desktop/src-tauri/src/commands.rs` and bridge in `apps/desktop/src/core/bridge.ts`).

## Required reading

- `docs/Gritty IDE Architecture Refinements.md` §2.1 Gate A, §3 (operation model,
  lifecycle semantics, backpressure), §5 (actors), §6 (event vocabulary), §16 rules
- `docs/Gritty IDE Project Plan.md` §6.2 (protocol surface), §12 (security)
- ADR-003, ADR-008; `docs/architecture/overview.md`
- Tauri docs: commands, events, `tauri::async_runtime`; `ts-rs` docs

## Allowed scope

- `crates/gritty-core/src/**` (new modules: `protocol` or `ids`, `error`, `actor`, `operation`, `event`)
- `apps/desktop/src-tauri/src/**` (command adapters, event emission, error mapping)
- new `packages/protocol/**` (typed client, generated types, tests)
- `apps/desktop/src/**` (replace `core/bridge.ts` with the protocol client)
- `pnpm-workspace.yaml`, root `Cargo.toml` (add `ts-rs`, `uuid`/`ulid` with justification)
- `docs/architecture/overview.md` (protocol section), `docs/decisions/` if a decision changes
- `.github/workflows/ci.yml` (generated-types freshness check)

## Explicit non-goals

- No workspace/file/terminal commands (T020/T030/T090).
- No durable persistence of operations (T060 decides what is persisted).
- No `tauri-specta`. No global state library in the UI.
- No real long-running work beyond a demo operation used for tests.

## Required behavior

1. **Naming.** Commands are `<domain>.<verb>` in TypeScript (`core.health`,
   `operations.cancel`) mapped to snake_case Rust command functions; document the
   mapping. Events use the addendum §6 families (`OperationStarted`, …) with one
   documented naming convention.
2. **Error envelope.** Every command returns `Result<T, CoreError>` where
   `CoreError { code: CoreErrorCode, message, details?: JSON, retryable: bool, correlationId? }`.
   `CoreErrorCode` is a closed enum (e.g. `InvalidInput`, `NotFound`, `PermissionDenied`,
   `Conflict`, `Cancelled`, `Timeout`, `Io`, `Internal`). Command panics must surface as
   `Internal`, never as a hung promise.
3. **Event envelope.** `{ name, seq, workspaceId?, correlationId?, actor, payload }`
   with monotonically increasing `seq` per process; typed emit helper in Rust; typed
   `subscribe(name, handler): Unsubscribe` in the client.
4. **Identifiers.** Newtype string ids (`WorkspaceId`, `OperationId`, …) with one
   generation scheme (UUID v7 or ULID; decide, justify, document).
5. **Actor.** `USER | SYSTEM | AGENT(runId) | EXTERNAL` serialized as a tagged union.
6. **Operation model.** `Operation { id, kind, actor, workspaceId?, status, createdAt,
   startedAt?, finishedAt?, cancellationRequested, correlationId }`; statuses
   `CREATED → RUNNING → COMPLETED | FAILED | CANCELLED`; `operations.cancel(id)` and
   `operations.list()`; a registry in the core running on `tauri::async_runtime`;
   progress via `OperationProgress` events with bounded frequency.
7. **Cancellation.** Cooperative tokens; cancel before start resolves immediately;
   cancel during execution stops within a bounded time for the demo op; events that
   arrive after cancellation are dropped; duplicate completion signals are ignored;
   cancelling an unknown id returns `NotFound`; cancelling twice is idempotent.
8. **Versioning.** `PROTOCOL_VERSION` constant exposed by `core.health`; the client
   refuses to run against a mismatching major version with a clear error.
9. **Type export.** Protocol types derive `TS` and export to
   `packages/protocol/src/generated/` via `cargo test -p gritty-core`; CI fails if the
   generated files are stale (`git diff --exit-code`).
10. **Client.** `packages/protocol` exposes `invoke`-wrapping typed functions and
    event subscription; it is the only module importing `@tauri-apps/api`. Add a
    Biome `noRestrictedImports` rule (or equivalent check) so other packages cannot
    import it. Delete `apps/desktop/src/core/bridge.ts`.
11. **Demo operation.** `operations.startDemo({ steps, stepMs })` emitting progress,
    used by tests and by a small dev-only UI control to exercise cancel.

## Failure behavior

- Invalid arguments → `InvalidInput` with field details, no partial side effects.
- Serialization failure of a payload → `Internal` with a logged correlation id.
- Renderer reload while operations run: the registry survives; `operations.list()`
  rehydrates the UI; no duplicate events are replayed.
- App shutdown: all running operations are cancelled; the demo op observes the token.

## Acceptance criteria

- `pnpm dev` shows health via the typed client; forcing an error (dev toggle or
  renamed command) renders the envelope fields, never a hang.
- Demo operation: start → progress visible → cancel → status `CANCELLED` within
  the bound; no `COMPLETED` afterwards (automated Rust test + manual UI check).
- Generated TypeScript types equal the Rust definitions (CI freshness check green).
- No `@tauri-apps/api` import outside `packages/protocol` (lint enforced).

## Tests required

- Rust unit tests: operation state machine transitions (including invalid ones),
  cancellation before/during/after, duplicate completion, envelope serialization.
- Vitest: client error mapping, event subscription/unsubscription, version guard.
- Manual: the acceptance flow above, recorded in the completion report.

## Commands to run

`pnpm check`, `pnpm dev`, `cargo test -p gritty-core` (regenerates types), `git diff --exit-code packages/protocol/src/generated`.

## Completion report

(fill in)
