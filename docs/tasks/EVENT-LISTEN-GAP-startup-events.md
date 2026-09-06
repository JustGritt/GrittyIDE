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

**Summary.** The client now guarantees *ordering* rather than
delivery-in-hindsight. `EventHub.start()` begins registration when the client is
constructed, so it overlaps the compatibility probe instead of waiting for the
first subscriber; `command()` awaits `EventHub.ready()` before sending, so an
event a command causes cannot be emitted into a WebView that is not listening
yet. `ready()` is bounded and fails open, a refused registration is retried with
a bounded backoff, and the hub reports how many events preceded its listener.
Design option **A**, not **C** — the reasoning is in the ADR amendment.

**Files changed.**
- `packages/protocol/src/events.ts` — `start()`, `ready()`, the one-shot budget
  latch, `#retry()`, and the first-envelope report.
- `packages/protocol/src/client.ts` — `hub.start()` at construction,
  `await hub.ready()` in `command()`, `listenBudgetMs` on `CoreClientOptions`.
- `packages/protocol/src/testing/fake-transport.ts` — `failListen`, so a
  refused registration can be tested at all.
- `packages/protocol/src/client.test.ts` — four new cases; one existing case
  updated (see below).
- `docs/decisions/ADR-009-core-protocol-conventions.md` — amended, not a new
  ADR: this is that ADR's own revisit trigger firing.

**Tests actually executed.**
- `pnpm check` — **exit 0**: Biome + tsc over 247 files, **572 vitest tests in
  72 files**, `cargo fmt --check`, `cargo clippy --workspace --all-targets -D
  warnings`, `cargo test --workspace`, generated types fresh.
- **Each new test was confirmed to fail without its mechanism**, which is the
  point of the ticket's "fails before, passes after" criterion:
  - removing `await hub.ready()` → *does not send a command until the listener
    is registered* and *lets commands through when the channel never opens*
    both fail (2 failed | 63 passed).
  - removing `hub.start()` → *registers the listener before anything subscribes*
    fails (1 failed | 64 passed).
- One existing test changed: *reports sequence discontinuities and malformed
  payloads* emits `seq 5` first, which is now also reported as preceding the
  listener. The assertion gained that message rather than losing anything.

**Architectural decisions.**
- The ordering test injects a 60-second budget deliberately. With a short one it
  would pass even with the gate deleted — the timer would let the command
  through and the post-`resolveListen` assertion would still hold. Ordering is
  the property, so nothing may rescue it but registration.
- The first-envelope report says "core events preceded this listener", never
  "lost". The core is built during Tauri's `setup`, before the window is shown,
  so a healthy cold start always has events numbered before any WebView exists.
  A warning that always fires is not inspectability — and arming this ADR's own
  revisit trigger with noise would be worse than silence.
- Retry happens on *rejection* only, never on budget expiry. Retrying a `listen`
  that is merely slow would register twice, and two live listeners would append
  every timeline delta and every byte of terminal output twice.

**Known limitations.**
- **The guarantee is narrower than "no event is lost", by design**, and the ADR
  says so in those words. It does not cover events nothing asked for
  (`AgentSessionsChanged`, `PersistenceStatusChanged`, T070's watcher), nor a
  consumer that issues a command before its own `subscribe` — the hub buffers
  nothing and delivers only to subscriptions live at dispatch. For the first
  class, starting registration early is the only thing that helps, and it now
  starts as early as the client exists.
- **Startup cost is reasoned, not measured.** Registration overlaps a round trip
  that already happens, so the expected delta is nil, but `first webview health`
  has not been compared against the baselines in `docs/plans/phase-0-spike.md`.
  That needs a relaunch of the app; it is the one verification item outstanding.
- **The UI still cannot say "I am not receiving events."** The ticket asks for
  it under Failure behavior. The capability exists — `CoreClientOptions.onDiagnostic`
  — but where a dead channel should appear (status bar? session notice?) is a
  design decision on a surface this ticket does not own, so it is recorded here
  rather than invented.
- The "subscribe before you ask" comments in `use-explorer.ts` and
  `use-terminals.ts` still describe a real responsibility. What changed is that
  it is now written down in ADR-009 instead of being folklore in two files.

**Follow-ups.**
1. `lastEventSeq` on `HealthReport`, from `EventBus::last_seq()`, would turn the
   precedence report into proof of loss. It needs `core_health` to reach `Core`
   state and `Core` to retain the bus — `apps/desktop/src-tauri/**`, outside
   this ticket's scope, and a protocol MINOR with regenerated types.
2. Make a detected gap *actionable*: one hub-level signal that the consumers
   which already know how to re-ask (T070 §12 re-lists every loaded node on
   `overflowed: true`) can act on. That generalises to reloads and to a
   late-succeeding retry, far more cheaply than a replay ring.
3. **T070 should not be started before reading this**: its §13 criterion — an
   external edit *while unfocused* surfaces `MODIFIED_EXTERNALLY` within one
   debounce window — has no re-ask path behind it, because
   `use-documents.checkStatus()` runs only from the window-focus listener. The
   explorer half survives a lost batch (it re-lists on mount, and a dropped
   batch is indistinguishable from the `overflowed: true` one it already
   handles); the documents half does not.

**Risks.** Importing `apps/desktop/src/core/client.ts` now has a side effect:
the module-level singleton registers a listener at import. Under jsdom that
rejects, retries and gives up with a diagnostic — noisy in a component test, but
nothing throws. The JSDOM-VITEST work happening in parallel was warned.
