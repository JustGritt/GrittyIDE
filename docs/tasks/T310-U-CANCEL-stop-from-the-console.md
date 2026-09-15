# Task T310-U-CANCEL — Stop a run from the console it is showing in

- **Phase:** 4 (orchestration)
- **Status:** todo
- **Depends on:** T310-U (the prompt box, and an owned run in the console)

## Objective

The run console is where a user watches a run misbehave, and it has no way to
stop it. The capability exists — `runs.cancel` is wired end to end and the
sidebar row has a Cancel button — but T310-U's report names the gap: "the
surface you are looking at while a run misbehaves is not the one with the stop
button" (`docs/tasks/completed/T310-claude-code-adapter.md`, Known limitations).

This task puts a Stop control in the run console, asks once before it acts,
and makes the sidebar's Cancel follow the same rules — today it is offered on
runs that have already ended and never disables while a stop is under way.

**Renderer only.** No command, event or type changes.

## Dependencies

All present on `main`:

- `runs.cancel` — `packages/protocol/src/commands.ts`, `client.ts`; the desktop
  adapter `runs_cancel` (idempotent; an unknown run is `NotFound`).
- The core's stop: `AgentRunManager::cancel` → `request_stop` sets
  `cancellationRequested`, cancels the token and withdraws an outstanding ask;
  the Claude driver denies that ask and terminates the child.
- `RunsController.cancel(runId)` in `apps/desktop/src/features/runs/use-runs.ts`.
- `RunConsole` already receives the `controller`.

## Required reading

- `apps/desktop/src/features/runs/run-console.tsx` and `run-console-model.ts`
  (`promptGate` already words "This run is stopping")
- `apps/desktop/src/features/agents/agents-section.tsx` (the sidebar Cancel) and
  `roster-model.ts` (`RosterRow`, `runBadge`'s `cancelling`)
- `apps/desktop/src/features/runs/runs-panel.tsx` — the DEV panel's Cancel,
  which already gates on `!terminal` and disables while cancelling: the rule to
  copy
- `docs/architecture/agent-runs.md` — cancellation
- ADR-016 (tokens) for any new class

## Allowed scope

- `apps/desktop/src/features/runs/run-console.tsx`, `run-console-model.ts` and
  their tests (`run-console-render.test.ts`, `run-console.dom.test.tsx`,
  `run-console-model.test.ts`)
- `apps/desktop/src/features/agents/agents-section.tsx`, `roster-model.ts` and
  their tests
- `apps/desktop/src/app.css` — only an appended block for the confirm strip
- this file

## Explicit non-goals

- **No modal dialog, and no dependency on a shared confirm component.** Alexis
  chose an inline two-step confirm on 2026-09-14. A shared
  `components/confirm-dialog` may land with T102-U (staging and the commit
  box); do not adopt it here.
- No keyboard chord for Stop. There is none for cancel anywhere today;
  choosing one (`⌘.` is the usual candidate) belongs with the chord table —
  filed as a follow-up.
- No stop for an **observed** session. Gritty cannot stop what it did not
  start, and the row model exists to keep that boundary.
- No change to what cancelling does in the core, the 250 ms join grace, or the
  denial the driver sends.

## Required behavior

### One rule for when Stop is offered

1. Add a derived `stoppable` predicate for an owned run, in one place both
   surfaces use (`run-console-model.ts`, exported): **not terminal and not
   `cancellationRequested`**. The sidebar's `RosterRow` gains
   `stoppable: boolean` (always `false` for an observed row) computed from it.
2. A terminal run shows no Stop in either surface. A run with
   `cancellationRequested` shows the control **disabled**, reading
   `Stopping…`.

### The console

3. A `Stop` button in the console header (`agent-console__bar`), after the
   spacer and before the provider label, drawn only for an owned run that is
   `stoppable` or stopping.
4. Clicking it does **not** cancel. It replaces the button, in place, with a
   confirm strip: `Stop this run?` · `Stop` · `Keep running`.
   - When the run is `BLOCKED` with a `pending` ask, the question says so:
     `Stop this run? Its request to use <tool> will be refused.` — cancelling
     denies the outstanding ask, and the user is about to lose a card they may
     have meant to answer.
   - Focus moves to `Keep running`, the safe default. `Escape` is `Keep
     running`. The strip never auto-confirms and has no timeout.
5. `Stop` calls `controller.cancel(run.id)` once and returns the header to the
   disabled `Stopping…` state until the run's `AgentRunChanged` arrives. A
   second click cannot send a second cancel.
6. If the run ends or starts stopping by other means while the strip is open
   (the sidebar, the run finishing on its own), the strip closes without
   acting.
7. An error from `runs.cancel` shows through the existing
   `controller.error` envelope; the header returns to `Stop`.

### The sidebar

8. The row's Cancel button renders only when `row.stoppable` or the run is
   stopping, and uses the same two-step: `Cancel` → `Stop? · Stop · Keep`
   in the row's trailing slot, with the same focus, `Escape` and
   state-change rules. Disabled `Stopping…` while `cancellationRequested`.
9. The confirm state is per surface and per run: confirming in the console
   does not open anything in the sidebar, and vice versa.

## Failure behavior

- `runs.cancel` on a run that already ended is idempotent in the core; the
  renderer must not treat the returned snapshot as an error.
- A cancel that races the run finishing: whichever `AgentRunChanged` arrives
  decides what is drawn; the strip closes.
- The console unmounting mid-confirm (tab closed) sends nothing.

## Acceptance criteria

- On a `RUNNING` run, the console shows `Stop`; clicking shows the strip;
  `Keep running` restores it with nothing sent; `Stop` sends exactly one
  `runs.cancel` and the header reads `Stopping…` until the run is `CANCELLED`.
- On a `BLOCKED` run with an ask, the strip names the tool that will be refused.
- A `FINISHED`, `FAILED`, `INTERRUPTED` or `CANCELLED` run shows no Stop in the
  console and no Cancel in its sidebar row.
- The sidebar Cancel obeys the same rules and never cancels on the first click.
- Exercised in the running app (`pnpm dev`) against a real owned run, and the
  report says what was seen.

## Tests required

- `run-console-model.test.ts`: `stoppable` for every `AgentRunState`, with and
  without `cancellationRequested`.
- `run-console.dom.test.tsx`: first click opens the strip and sends nothing;
  `Keep running` and `Escape` close it; `Stop` calls `cancel` once; the blocked
  wording names the tool; a run turning terminal closes an open strip; focus
  lands on `Keep running`.
- `run-console-render.test.ts`: its controller stub currently
  `refuse('cancel')`s to prove the console never cancels. Change it
  deliberately: rendering alone still never cancels, and the DOM test owns the
  click path.
- `roster-model` and `agents-section` tests: `stoppable` on owned rows by
  state, always false on observed rows; the two-step in the row.
- Mutation check, stated in the report: remove the confirm step (call cancel
  on the first click) and a test fails.

## Commands to run

```
pnpm vitest run apps/desktop/src/features/runs apps/desktop/src/features/agents
pnpm check
pnpm dev    # manual: start a run, stop it from the console and from the sidebar
```

## Completion report

_Not started._

Follow-up to file on completion: a chord for stopping the focused run, decided
with the chord table.
