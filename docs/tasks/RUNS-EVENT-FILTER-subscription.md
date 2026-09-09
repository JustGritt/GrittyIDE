# Task RUNS-EVENT-FILTER — `use-runs.ts` subscribes to every event of every kind

- **Phase:** maintenance (found by the ADR-032 gate)
- **Status:** todo
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

_Not started._
