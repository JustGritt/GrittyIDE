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
  before they reach the bus. **Registering the listener is itself asynchronous;
  see §Amendment 2026-09-06 for what that costs and what the client now
  guarantees.**
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

---

# Amendment, 2026-09-06 — listener registration is asynchronous

- **Status:** Accepted
- **Amends:** the *Events* bullet above, and the *Revisit when* clause, which
  fired.

## Context

`seq` gaps were observed in practice, which is the second of the two triggers
this ADR named. The observation was a shipped bug (PR #33): the shell sat on
the startup launcher *while a workspace was open*, and dropping that same
folder onto the window could not rescue it.

The cause was not the one this ADR anticipated. It was not that a domain needed
replay across a reload — it was that **`subscribe()` is synchronous to its
caller and asynchronous underneath.** `EventHub` registers its transport
listener with an IPC round trip, and `app.emit` to a WebView with no registered
listener succeeds and delivers nothing. `session.restore` opens the last
workspace and announces it inside that window; the announcement reached nobody.
The persisted tabs still appeared, because those come back in the command's own
*response* — which is exactly what made the resulting state so confusing.

## What was measured

1. Only one place in the repository sends a command: `invokeRaw` in
   `packages/protocol/src/client.ts`, reached by every domain client through
   `command()`, which already awaits a cached `core.health` probe.
2. `EventHub` was fully lazy — its constructor did not call `transport.listen`;
   registration began with the first `subscribe`, i.e. after the first commands
   had already been issued in practice.
3. A rejected `listen()` left every subscription already added permanently
   deaf: the promise was cleared, but only a *new* `subscribe` retried, and the
   hooks in this UI subscribe exactly once, at mount.
4. `seq` starts at 1, but the core is built during Tauri's `setup`, before the
   window is shown. A healthy cold start therefore has events numbered before
   any WebView exists — so "first envelope with `seq > 1`" is **not** evidence
   of a fault, and must not be reported as one.

## Decision

The client now guarantees **ordering**, not delivery-in-hindsight:

- `EventHub.start()` is called when the client is constructed, so registration
  overlaps the compatibility probe rather than waiting for a subscriber.
- `command()` awaits `EventHub.ready()` before sending. Since registration
  starts at construction and the probe is a full round trip, this has almost
  always resolved already.
- `ready()` is **bounded and fails open.** A dead event channel degrades to
  "commands work, events do not"; it never becomes a window that will not
  respond.
- A refused registration is retried with a bounded backoff.

**The guarantee, stated exactly:** an event emitted after a command was sent is
delivered to every subscription registered at the moment it is dispatched. It
is deliberately not "no event is ever lost". Two things it does not cover, both
of which are properties of the subscriber and not of the channel:

- **Events nothing asked for.** `AgentSessionsChanged` from the discovery
  poller, `PersistenceStatusChanged`, and T070's watcher events are emitted
  spontaneously. For those, only starting registration early helps.
- **Subscribing after commanding.** The hub delivers to the subscriptions live
  at dispatch time and buffers nothing, so a consumer that issues a command
  before its own `subscribe` still misses what it caused.

**No replay ring, still.** The bug was a registration-ordering fault, not a
replay fault, and this ADR's existing answer — rehydrate through list commands
— stands. A core-side buffer would also not have helped: `EventSink::emit` runs
under the bus lock and feeds the session tap that persistence depends on, and
"buffer until the first command" would flush into a WebView that is still not
listening, because the first command is the health probe.

**The rule this makes explicit, because both halves of PR #33 broke it:** *a
command that returns state is the authority for that state.* An event is a
notification, not the only delivery. Subscribing does not retroactively deliver.

## Consequences

- Every client registers one listener for its whole lifetime, including one
  that never subscribes. There is one client per WebView.
- Importing the client now has a side effect. In a non-Tauri environment
  (a jsdom test) registration rejects, is retried, and gives up with a
  diagnostic; nothing throws.
- The hub reports how many events preceded its listener. This is worded as an
  observation, not as loss, for the reason measured above — proving loss needs
  a baseline the client does not have.

## Revisit when

- A domain needs events the WebView must not miss across reloads **and** cannot
  re-ask for the state — then add a replay/list pair for that domain. T070 is
  the first candidate: its explorer half re-lists on mount and survives, but
  `documents.checkStatus` currently runs only on window focus, so an external
  edit while unfocused has no re-ask path behind it.
- Proving loss rather than observing precedence becomes worth a `lastEventSeq`
  on the health report.
