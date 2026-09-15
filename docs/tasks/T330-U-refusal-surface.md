# Task T330-U — The refusal surface

- **Phase:** 4 (orchestration)
- **Status:** todo
- **Depends on:** T330-R (the fence, the matcher, and the hook's refusal), T326 (observed ownership violations), T335 (the attention queue's permission cards)

## Objective

T330-R makes a refusal happen and puts it on the wire: a `REFUSED` conflict,
a per-provider enforcement switch, and a per-session `ENFORCED` / `OBSERVED`
fact. This task makes all three visible and actionable, so that when an agent
is refused, the user can see:
- **what happened**: a write that did *not* land;
- **who initiated it**: which session, on which workstream's behalf;
- **whose ground it was**, and **what they can do**: release, or override on
  the record (ADR-019 §8).

It also moves `docs/architecture/ownership.md`'s "observed, not enforced" line.
T330-U is the last piece of T330, so enforcement ships here (ADR-018A §8).

Split out of T330 (write interception) on 2026-09-14.

## Dependencies

- T330-R on `main`, with its protocol minor: `ConflictState::REFUSED`, the
  enforcement fact on sessions, enforcing in integration status, and
  `agents.setEnforcement`.
- T326's conflict rows in the attention queue, the island and the panel.
  T335's queue.
- The settings page (T120-U) and `agent-integration-section.tsx`, with its
  install / uninstall diff flow.

## Required reading

- ADR-018A §5, §6, §8 and "What is enforced, and what is not"
- `docs/tasks/completed/T326-ownership-violations.md` §6, §7 and §12 (queue
  wording; why a write that happened and one that did not must not read alike)
- ADR-019 §8 (release and override)
- `apps/desktop/src/features/workstreams/{ownership.ts,attention-queue.tsx,island.tsx}`
- `apps/desktop/src/features/settings/agent-integration-section.tsx`,
  `apps/desktop/src/features/agents/agent-integration.ts`
- `apps/desktop/src/features/agents/agents-section.tsx`, `agent-console.tsx`
  (where a per-session fact is drawn)
- `docs/architecture/ownership.md`

## Allowed scope

- `apps/desktop/src/features/workstreams/**`
- `apps/desktop/src/features/settings/agent-integration-section.tsx` and its tests
- `apps/desktop/src/features/agents/**` (the enforcement badge, and the
  integration controller calls)
- `apps/desktop/src/app.css` (appended blocks only; tokens per ADR-016)
- `docs/architecture/ownership.md`, this file

## Explicit non-goals

- No core, hook or wire change. A missing field means a T330-R follow-up,
  not a workaround here.
- **No "allow once"**, and no answer from the terminal. The only ways through
  are release and `OVERRIDE` (ADR-018A §6).
- Never the words "protected", "locked" or "secured" for a file or a lease
  (ADR-018A §8). Enforcement is per session and per tool, and the shell is
  still a hole.
- No enforcement for owned runs in the UI beyond what the wire says. If T330-R
  left them observed, they read as observed.

## Required behavior

### Consent: settings

1. In the agent integration section, per provider, beside install and
   uninstall, add an **Enforce ownership** switch.
   - It is disabled unless the entry is installed.
   - Turning it on shows the same kind of diff install shows (the flag and the
     timeout being added) and applies only on confirmation.
   - Turning it off applies immediately and says that observation stays
     installed.
2. The explanatory text states the limits in one short paragraph, adapted from
   ADR-018A's table:
   - refuses `Write`, `Edit`, `MultiEdit` and `NotebookEdit` by linked
     sessions on another workstream's ground, while Gritty is running;
   - **does not** stop shell commands, MCP tools or unlinked sessions;
   - plus the `bypassPermissions` caveat, if T330-MEASURE recorded one.

### Per session: enforced or observed

3. Each agent row in the sidebar and each session console shows the session's
   enforcement fact as a quiet badge: `enforced` or `observed`. The tooltip
   says why a session is observed (not linked; enforcement off for its
   provider; entry not installed).
4. Where a lease is drawn (the island row, the ownership panel), linked
   sessions are listed with the same badge. The lease itself never says
   "enforced".

### The refusal: attention queue, island, panel

5. A `REFUSED` conflict is its own card and row kind. It is never worded like
   `OBSERVED`:
   - `OBSERVED` keeps "Wrote" (`attention-queue.tsx`);
   - `REFUSED` reads "**Refused**: `<session>` for `<workstream>` tried to
     write `<target>` on `<holder>`'s ground";
   - the time is shown, and retries of the same target collapse into one card
     with a count.
6. Actions:
   - `Release`, when the user may release the holder's lease;
   - `Override`, which is attributed and on the record (ADR-019 §8, the
     existing `resolveConflict(id, 'OVERRIDE')` path);
   - `Dismiss`.

   There is no action that lets the write through by itself; the agent's next
   attempt succeeds once the fence is rewritten.
7. Counts: `REFUSED` counts toward "needs you" wherever `OBSERVED` does today.

### Docs

8. `docs/architecture/ownership.md`: the "observed rather than enforced" line
   becomes the ADR-018A §8 statement. A lease is enforced against a session
   only when the five conditions hold; everything else is observed; and there
   is a table of what is and is not refused.

## Failure behavior

- `agents.setEnforcement` failing (for example, a config file with an
  unsupported shape) shows the error envelope. The switch returns to its
  previous state; nothing optimistic.
- A `REFUSED` conflict whose lease has since been released: the card says the
  ground is free, and offers only `Dismiss`.
- An older core without the new fields: the badge is absent rather than
  guessed, and the switch is hidden.

## Acceptance criteria

- With enforcement on and a linked session refused (produced through T330-R's
  test path or a live probe), the card:
  - appears within one tick;
  - reads as refused, not written;
  - collapses retries;
  - offers Release and Override;
  - disappears after an Override, and the agent's next attempt is not
    refused.
- Switching enforcement off in settings changes every affected session's badge
  to `observed`, and no new refusal cards appear.
- No string anywhere in the UI calls a file protected or locked (a test greps
  the rendered output).
- Exercised in `pnpm dev`, and the report says what was seen.
- `docs/architecture/ownership.md` updated.

## Tests required

- `ownership.ts` model tests: `REFUSED` vs `OBSERVED` wording and actions;
  collapsing retries; a released lease.
- `attention-queue` DOM/render tests: the refused card and its three actions.
- `agent-integration-section` tests: the switch disabled when not installed,
  diff shown before enabling, error restores the previous state.
- Badge tests on a sidebar row and a console, including the tooltip reason.
- Mutation check, stated in the report: make `REFUSED` render through the
  `OBSERVED` branch and a test fails.

## Commands to run

```
pnpm vitest run apps/desktop/src/features/workstreams apps/desktop/src/features/settings apps/desktop/src/features/agents
pnpm check
pnpm dev    # manual: enable enforcement, link a session, provoke and override a refusal
```

## Completion report

_Not started._
