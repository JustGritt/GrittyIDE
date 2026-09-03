# Task T010 — Core protocol + operation/event/error conventions (Gate A)

- **Phase:** 0
- **Status:** review
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

### Summary

Gate A is in place on branch `t010-core-protocol`. `crates/gritty-core` now owns the
wire contract (UUID v7 newtype ids, epoch-millisecond timestamps, the closed
`CoreError` envelope, the tagged `Actor` union, `PROTOCOL_VERSION`), the event
envelope and bus (`EventMeta` + adjacently tagged `CoreEvent`, one `seq` stream,
one Tauri channel `gritty://core/event`), and the operation model (strict state
machine, registry on Tauri's tokio handle with cooperative cancellation, throttled
progress, bounded retention, shutdown, demo operation). `packages/protocol` holds
the generated TypeScript types plus a hand-written client (command table, error
mapping, event hub, protocol MAJOR guard); its Tauri transport is the only module
importing `@tauri-apps/api`, enforced by Biome and by the dependency graph. The
desktop shell exposes `core_health`, `operations_list`, `operations_cancel` and
`operations_start_demo` behind a guard that attaches a correlation id and turns
panics into `Internal`; the UI shows health through the client and, in
development builds, an operations panel to start/cancel the demo and inspect
error envelopes. ADR-009 records the conventions.

### Files changed

- Root: `Cargo.toml` (tokio, tokio-util, uuid, thiserror, ts-rs), `Cargo.lock`,
  `package.json` (`check:generated`, `check`), `pnpm-lock.yaml`, `biome.json`
  (restricted imports, generated dir excluded), `.github/workflows/ci.yml`,
  `AGENTS.md`, `README.md`, `.gitignore`
- `crates/gritty-core/`: `Cargo.toml`, `src/lib.rs`, `src/health.rs`,
  `src/protocol/{mod,ids,time,error,actor,export}.rs`, `src/event/mod.rs`,
  `src/operation/{mod,state,registry,progress,demo}.rs`
- `packages/protocol/` (new): `package.json`, `tsconfig*.json`, `vitest.config.ts`,
  `src/{index,types,commands,errors,events,transport,version,client}.ts`,
  `src/transport/tauri.ts`, `src/testing/fake-transport.ts`, `src/*.test.ts`,
  `src/generated/**` (ts-rs output + `version.ts`)
- `apps/desktop/`: `package.json` (depends on `@gritty/protocol`, not
  `@tauri-apps/api`), `src-tauri/{Cargo.toml,src/lib.rs,src/core.rs,src/commands.rs,tests/commands.rs}`,
  `src/App.tsx`, `src/app.css`, `src/core/{client,health,health.test}.ts`
  (`bridge.ts` deleted), `src/components/error-envelope.tsx`,
  `src/features/operations/{operations-reducer,operations-reducer.test,use-operations}.ts`,
  `src/features/operations/operations-panel.tsx`
- Docs: `docs/decisions/ADR-009-core-protocol-conventions.md` (+ index),
  `docs/decisions/ADR-003-ui-core-boundary.md`, `docs/architecture/overview.md`,
  `docs/plans/phase-0-spike.md` (B3), `docs/tasks/README.md`, this file

### Tests actually executed (2026-09-03, macOS 26.6 arm64, Node 24, Rust 1.98.0)

| Command | Result |
|---|---|
| `pnpm check` | exit 0 in 17 s: Biome 43 files, 0 errors, 0 warnings; tsc root + desktop ×2 + protocol ×2 clean; vitest 6 files / 30 tests (protocol 23: error mapping, version guard, command table, event hub; desktop 7: health summary, operations reducer); `cargo fmt --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` 43 (core) + 4 (desktop unit) + 6 (desktop mock runtime) passed; `check:generated` clean |
| `cargo test -p gritty-core` (repeated) | regenerates `packages/protocol/src/generated` deterministically; `git status` stays clean |
| Negative lint check | `import { invoke } from '@tauri-apps/api/core'` added to `App.tsx` → Biome `lint/style/noRestrictedImports` error with the ADR-003 message; reverted |
| `pnpm --filter @gritty/desktop build` | Vite bundles `@gritty/protocol` from source: 34 modules, 198 kB JS |
| `GRITTY_LOG=debug pnpm dev` | app started 3× (initial + two rebuilds on source change); each start logged `core.health` inside `command{command="core.health" correlation_id=…}` 0.5–2 s after launch, i.e. the typed client works under the dev CSP; WebKit 21624.4.5.11.5, protocol 1.0.0 |
| Core registry tests | cancel before start (body never runs), cancel during execution (`CANCELLED` within 500 ms, no `COMPLETED` afterwards), unknown id → `NotFound`, idempotent/after-completion cancel, duplicate and stale completions rejected, completion after cancellation request reported as `CANCELLED`, progress after cancel dropped, throttle (1000 reports → 2 events, snapshot current), body panic → `FAILED/Internal`, body error → `FAILED`, shutdown cancels running ops, list ordering + retention, `seq` strictly increasing across 8 threads, envelope JSON shape |
| Mock-runtime adapter tests (`apps/desktop/src-tauri/tests/commands.rs`) | health via IPC; `NotFound` envelope with correlation id; `InvalidInput` for `steps: 0` (field `steps`) and for a malformed request (field `request`) with no operation created and no event; unknown command rejected (string, never a hang); demo start → `RUNNING` with progress → cancel → `CANCELLED` in under 1 s, second cancel idempotent, no `OperationCompleted`, exactly one `OperationCancelled`, contiguous `seq`, envelope carries the operation's correlation id and actor; `failAtStep` → `FAILED` with the error envelope |
| Panic guard | `guard` unit tests: panic → `Internal` with correlation id; core errors receive the correlation id |
| CI (PR #2, run 33730165169) | Linux `pnpm check` + `tauri build --no-bundle` green in 6 m 5 s (includes the generated-types freshness step); macOS job skipped by design (no `ci:macos` label; macOS verified locally) |

Not executed by the agent: clicking through the development panel (start, cancel,
forced errors, renderer reload) and quitting the app during a demo operation. The
CLI session has neither screen-recording nor assistive access, so the WebView
could not be observed or driven; the same flow is automated on Tauri's mock
runtime (row above). The dev process was stopped with SIGTERM, which does not
emit `RunEvent::Exit`, so the shutdown hook was not observed live (its registry
half is unit-tested).

### Manual checks for the reviewer (`pnpm dev`)

1. Health facts show `Protocol 1.0.0` and the runtime/WebView versions.
2. Operations panel: Start demo (20 steps × 250 ms) → progress bar advances →
   Cancel → status `CANCELLED (cancelling)` then `CANCELLED`; the row never
   changes again.
3. `failAtStep` = 3 → `FAILED` with `Internal: demo operation failed at step 3`.
4. "Cancel unknown id" → envelope with `code NotFound`, message, `retryable false`,
   a `correlationId`.
5. Renderer reload: start a demo (60 × 1000 ms), save `App.tsx` with a trivial
   change to force a full reload → the running operation reappears from
   `operations.list()` and progress keeps updating.
6. Cmd+Q during a demo → terminal shows `shutting down: cancelling running
   operations` and `operation finished status=Cancelled`.

### Architectural decisions

- Recorded in ADR-009: UUID v7 ids (branded in TypeScript); ms timestamps; 64-bit
  integers exported as `number`; `Option` fields omitted and exported optional;
  one `request` object per command, deserialized by the adapter so malformed input
  is `InvalidInput`; single event channel with a flat envelope; `EventEnvelope`
  composed in TypeScript as `EventMeta & CoreEvent` (avoids ts-rs enum flattening);
  once cancellation is requested the only terminal state is `CANCELLED`; finished
  operations retained in memory (200); progress throttled to one event per 50 ms;
  client refuses a different protocol MAJOR and keeps refusing.
- `Operation` carries `progress?` and `error?` beyond the addendum list so
  `operations.list()` can rehydrate the UI after a renderer reload.
- The core receives a `tokio::runtime::Handle` from the shell instead of spawning
  on an ambient runtime: no second runtime, and tests run under `#[tokio::test]`.
- Command bodies run as their own tasks behind `guard`; operation bodies run as
  their own tasks under the runner: a panic in either is contained.
- One explicit `export_bindings` test lists the protocol roots (no `#[ts(export)]`
  per type, no `.cargo/config.toml`); it also writes `version.ts`.
- `#[serde(transparent)]` dropped from newtypes: serde already serializes a
  single-field tuple struct as its inner value, and ts-rs 12 cannot parse the
  attribute (it printed warnings).
- Shell wiring is generic over the Tauri runtime (`configure`, `attach_core`) so
  the mock runtime drives the real adapters; the mock never runs `setup`, so tests
  call `attach_core` themselves.

### Dependencies added

- `ts-rs 12.0.1` (`serde-json-impl`): Rust → TypeScript export (ADR-008). The only
  new crate tree (`ts-rs-macros`, `termcolor`).
- `tokio 1.53.1` (`rt`, `sync`, `time`, `macros`; dev `rt-multi-thread`, `test-util`):
  async primitives on Tauri's runtime; already in the tree.
- `tokio-util 0.7.19`: `CancellationToken`; already in the tree.
- `uuid 1.26.0` (`v7`): identifiers; already in the tree.
- `thiserror 2.0.20`: error derives; already in the tree.
- `serde_json`: promoted to a normal dependency of the core (`details`, results)
  and added to the desktop crate (request parsing).
- `tauri` feature `test` as a desktop dev-dependency: mock runtime for adapter tests.
- JavaScript: nothing new; `@tauri-apps/api` moved from `apps/desktop` to
  `packages/protocol`.

### Known limitations

- Events emitted between a renderer reload and re-subscription are lost by
  design; the UI rehydrates through `operations.list()`.
- Throttled progress drops intermediate reports; the snapshot stays current and
  the terminal event carries the final state.
- Finished operations live in memory only (last 200); persistence is T060's call.
- `client.health()` memoises the probe result; a fresh report needs a new client
  (no consumer needs one yet).
- A body without await points cannot be stopped; the runner reports `CANCELLED`
  after a 250 ms grace and logs a warning.
- The operations panel exists only in development builds and has no component
  test (jsdom arrives with T040).
- `RunEvent::Exit` is only emitted on a normal quit; a SIGKILL skips the shutdown
  hook (operations are in-memory, so nothing is lost).

### Follow-up tasks

- T020: add `workspace.*` requests to `COMMANDS`, `Workspace*` variants to
  `CoreEvent`, and a `WorkspaceOpened/Closed` rehydration pair.
- Client-supplied correlation ids (propagating a UI intent through several
  commands) when a use case appears.
- `EventHub` could release its listener when the last subscriber leaves; today it
  stays until `dispose()`.
- Automating the manual panel flow needs screen-recording plus assistive access for
  the agent session on macOS, or a WebDriver setup on Linux.
- Phase 0 B3 should be re-run with a real long operation once T080 (search) exists.

### Remaining risks

- ts-rs is pinned exactly; output drift on upgrade is caught by `check:generated`.
- Tauri documents its `test` module as unstable; the mock-runtime test may need
  adjustment on Tauri upgrades.
- Every event is serialized for the WebView even without a listener for its
  family; acceptable at current volumes (ADR-009 names the revisit trigger).
