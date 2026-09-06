# Task EVENT-LISTEN-GAP — An event emitted before the WebView is listening reaches nobody

- **Phase:** 1
- **Status:** todo
- **Depends on:** T010 (Gate A: the event envelope, `seq`, the transport
  contract)

## Objective

`core.subscribe(…)` returns synchronously and registers asynchronously.
Between those two moments the renderer is not listening, and an event the core
emits in that window is dropped by the platform — with no error, no retry and
no trace on either side. Close that window, or state precisely why it cannot
be closed and make the loss visible instead.

## Motivation

This ticket exists because the gap already cost a day. PR #33:
`session.restore` opens the last workspace and announces it with
`WorkspaceOpened`; the shell missed the event and sat on the launcher **over an
open workspace**. The tabs were there — they come back in the restore's own
response — so the window showed a live `AGENTS.md` under "No workspace open".
There was no way back in either: re-dropping the same folder is the one
`workspace.open` that emits nothing (`crates/gritty-core/src/workspace/
mod.rs:271`), so drag and drop appeared to be broken.

PR #33 fixed those two call sites by reading the answer the command had already
returned. That is a mitigation, not a fix. **Ten subscribers register this
way** and every one of them is exposed:

```
features/session/use-session.ts        features/documents/use-documents.ts
features/explorer/use-explorer.ts      features/terminal/use-terminals.ts
features/workstreams/use-workstreams.ts features/operations/use-operations.ts
features/agents/use-agents.ts          features/agents/use-agent-rail.ts
features/drop/use-file-drop.ts (x2)
```

The urgency is **T070**. A file watcher is *entirely* event-driven: there is no
command whose response can be read instead. Built on this transport it would be
occasionally, unreproducibly wrong — the worst failure this project has, given
`correctness → recoverability → inspectability`.

## Dependencies

- **T010** for the envelope, the single event channel and `seq`.

## Required reading

- `packages/protocol/src/events.ts:55-84` — `#add` calls `#ensureListening`,
  which stores the *promise* and only later the `Unlisten`. Nothing waits on
  it. Also `:86-108`, `#dispatch` and the sequence-discontinuity diagnostic.
- `packages/protocol/src/transport.ts` — the two primitives; `listen` returns
  `Promise<Unlisten>`, which is where the window comes from.
- `packages/protocol/src/transport/tauri.ts` — the only file allowed to import
  `@tauri-apps/api` (ADR-003); the registration is a real IPC round trip.
- `crates/gritty-core/src/event/mod.rs:359-370` — `seq` is assigned under a
  mutex and **starts at 1**, which is what makes loss provable.
- `apps/desktop/src-tauri/src/core.rs:69` — `TauriEventSink`; `app.emit` to a
  WebView with no listener registered succeeds and delivers nothing.
- PR #33's two call sites, as the pattern to generalise or replace:
  `apps/desktop/src/features/session/restore-flow.ts`,
  `apps/desktop/src/features/drop/use-file-drop.ts`.
- `AGENTS.md`, freeze points: the command/event/error envelopes and the
  operation model are frozen after Phase 0. Expect to write an ADR.

## Allowed scope

- `packages/protocol/src/{events.ts,client.ts,transport.ts,transport/tauri.ts}`
  and their tests.
- `apps/desktop/src/core/client.ts`, and the shell's startup ordering in
  `apps/desktop/src/App.tsx` if the chosen design needs it.
- A new ADR in `docs/decisions/`.
- `crates/gritty-core/src/event/**` only if the chosen design has a core side.

## Explicit non-goals

- **No event sourcing, no durable event log, no replay from disk** (hard rule
  9). A bounded in-memory hand-over is not an event store; anything larger
  needs its own ADR and a concrete requirement.
- Not a general-purpose reliability layer for transports that do not exist yet.
- Do not change the envelope shape or `seq` semantics unless the ADR says so
  outright — both are frozen.
- Not fixing the ten subscribers one at a time. Making that unnecessary is the
  point of the ticket.

## Required behavior

1. An event the core emits **after `subscribe()` has returned** reaches that
   subscriber — or the contract states plainly that it does not, and says what
   the caller must do instead. Either is acceptable; silence is not.
2. **Loss is inspectable.** `seq` starts at 1, so a first envelope with
   `seq > 1` is proof that events were missed. `#dispatch` cannot currently see
   this: the discontinuity check is skipped while `#lastSeq === undefined`,
   which is exactly the startup case.
3. A **failed registration does not create permanently deaf subscribers.**
   `#ensureListening`'s `catch` clears `#listening`, so a *later* `subscribe`
   retries — but every subscriber already added stays silent forever, and the
   UI shows no sign of it.
4. The rule PR #33 applied is written down where the next author will find it:
   **a command that returns state is the authority for that state; an event is
   a notification, not the only delivery.** Whether that lives in the ADR, the
   client's docstring or `docs/architecture/` is the implementer's call.

## Failure behavior

- `listen()` rejects — bounded retry, and the UI must be able to say it is not
  receiving events rather than looking merely idle.
- The WebView reloads (dev HMR, ⌘R) — a new hub with a new gap. A reload must
  reconcile by *asking*, not by waiting.
- The core emits before any WebView exists (setup-time work) — a supported,
  documented case, not an accident.
- Two windows, one core — a second WebView must not lose events because the
  first one is already listening.

## Acceptance criteria

- A test **fails before the change and passes after**: a fake transport whose
  `listen()` resolves on command, an event emitted while it is pending, and a
  subscriber that must not miss it.
- A first envelope with `seq > 1` is reported through the diagnostic channel.
- No subscriber under `apps/desktop/src/features/**` carries its own workaround
  for this.
- The decision is recorded — a new ADR, or an amendment to an existing one.
- `pnpm check` green; no new dependency.

## Tests required

- `packages/protocol` unit tests against a controllable fake transport
  (`packages/protocol/src/testing` already exists for this).
- The retry path after a failed `listen()`.
- The `seq > 1` detection.
- Manual: cold start into a persisted workspace five times in a row; the
  launcher must never appear. That is the exact symptom PR #33 chased, and it
  is timing-dependent, so once is not evidence.

## Commands to run

```
pnpm check
cargo test -p gritty-core     # only if the core side changes
```

## Design options (a decision for the implementing session, not a decision made
here)

- **A — awaitable readiness.** `EventHub.ready()` / `core.ready()`, awaited
  before the first command. Smallest change; relies on every future caller
  remembering, which is how this bug happened once already.
- **B — the hub buffers.** Does not work alone: the drop happens in Rust,
  before anything reaches JS.
- **C — a subscribe handshake.** The renderer announces itself and receives
  everything since `seq N` from a small bounded core-side ring. Robust, and the
  only option that survives a WebView reload. Costs a command, a retention
  policy and an ADR; watch rule 9.
- **D — the core withholds emissions until the renderer speaks.** Implicit,
  fragile, and wrong the moment there are two windows.

C is the most likely answer and the most expensive; the ADR should say why the
cheaper ones were refused.

## Completion report

_(unstarted)_
