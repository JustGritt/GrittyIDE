# Task RUNS-EVENT-FILTER — `use-runs.ts` subscribes to every event of every kind

- **Phase:** maintenance (found by the ADR-032 gate)
- **Status:** review
- **Depends on:** —

## Objective

`use-runs.ts:68-71` in the desktop app's runs feature does:

```ts
core.subscribeAll((event) => dispatch({ type: 'event', event }));
```

Every domain event in the app — watcher, git, terminal, documents — becomes a
dispatch and a render, including the many `runsReducer` ignores. Filter at the
subscription so the runs feature only wakes for run events.

This is **not** a framework problem. It was found while gating ADR-032 and would
be identical under any framework (ADR-032 decision 2); it is a subscription that
is too wide, and it belongs to this feature.

## Dependencies

None.

## Required reading

- the runs feature's `use-runs.ts` and its reducer
- the core protocol's event envelope, and whatever discriminates a run event —
  the filter must key off the contract, not off a string match on a name
- any sibling feature that already subscribes narrowly, so the idiom is copied
  rather than invented

## Allowed scope

The desktop app's runs feature directory. If the protocol offers no way to
subscribe to a single domain, record that as a follow-up rather than adding one
here — that is a contract change and belongs upstream.

## Explicit non-goals

- Changing the event envelope, or adding a filtered-subscription API to the
  protocol.
- Reworking `runsReducer`, or memoising components. The subscription is the bug.

## Required behavior

- The runs feature dispatches only for events its reducer acts on.
- No run event is dropped: everything the feature used to react to, it still
  reacts to. This narrows what wakes it, not what it knows.
- Unsubscribing on unmount is unchanged.

## Failure behavior

An unrecognised or future event kind must be ignored silently rather than
throwing — a filter that has to know every event kind becomes a second place to
update whenever the contract grows. Prefer an allow-list keyed on the run domain
with an inert default.

## Acceptance criteria

- A test asserts that a non-run event causes no dispatch, and that a run event
  still does.
- `pnpm check` green.
- No behavioral change visible in the run console.

## Tests required

A unit test around the subscription: feed it one run event and one unrelated
event, and assert the dispatch count. Manual: open a run console, edit files in
the explorer, and confirm the console still updates while unrelated activity no
longer re-renders it.

## Commands to run

```
pnpm exec vitest run apps/desktop/src/features/runs
pnpm check
```

## Completion report

**Summary.** `use-runs.ts` took `core.subscribeAll` and dispatched every domain
event of every kind into `runsReducer`, which reads exactly two names
(`AgentRunChanged`, `AgentRunEventAppended`) and returns its input state for
everything else. It now takes two typed `core.subscribe` calls instead — the
house pattern, already used and commented in `use-agents.ts:58`. No protocol
change was needed: `CoreClient.subscribe<N>` already existed.

**Files changed.**
- `apps/desktop/src/features/runs/use-runs.ts` — one `subscribeAll` becomes two
  typed subscriptions and a two-call teardown, with a comment recording why.
- `apps/desktop/src/features/runs/use-runs-subscription.dom.test.tsx` (new) —
  4 tests over a real `CoreClient` on a fake transport, not a mocked `core`,
  so the hub's own name filtering is exercised.

**Tests actually executed.**
- `npx vitest run apps/desktop/src/features/runs` → 5 files, 81 tests, passed.
- Full JS suite `npx vitest run` → 119 files, 1263 tests, passed.
- `npx tsc --noEmit -p apps/desktop/tsconfig.json` → exit 0.
- `npx biome check src/features/runs/` → clean.
- **The new test was verified failing against the pre-fix code**: the
  subscription was stashed and the suite re-run, giving 2 of 4 failures
  (`expected [] to deeply equal [ 'AgentRunChanged', … ]` and
  `expected 4 to be 3`). It is a regression test, not a test written to pass.

`pnpm check` was **not** run in full. The change is TypeScript-only and cannot
affect the Rust half; the working tree carried three other sessions'
uncommitted work at the time, so a workspace-wide `cargo clippy`/`cargo test`
would have been reporting their state, not this ticket's. Rust was untouched
(`git status` shows no `.rs` or `Cargo` changes).

**Measured effect, honestly.** 8 unrelated events cost **1** extra render
before the fix and **0** after. The `useReducer` identity bail-out was already
absorbing most of the cost, and React coalesces the dispatches within one
flush — so this removes a re-entry *per flush*, not one per event. Worth doing,
and worth not overstating: the ticket's value is that it takes an avoidable
confound out of the way before ADR-032's typing measurement is taken, which is
what ADR-032 criterion 2 now requires.

**Follow-ups (not done here, out of scope).** Seven other hooks still call
`subscribeAll`: `use-terminals`, `use-explorer`, `use-operations`,
`use-documents`, `use-workstreams`, `use-session`, `use-git`. Several read
several event names and switch on them, so narrowing is not automatic and each
needs its own judgement — `use-explorer.ts:140` documents its choice
deliberately. Not swept here; recorded so the pattern is visible.

**Risks.** Narrowing a subscription is how a feature silently stops receiving
something. That is why tests 3 and 4 exercise both event names end to end
rather than only asserting the narrow subscription. A third run event added to
`runsReducer` later must also be added here — the reducer and the subscription
are now two places that have to agree, where `subscribeAll` was one.
