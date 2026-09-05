# Task T210 — Workstream UI: the island and the attention queue

- **Phase:** 3
- **Status:** review
- **Depends on:** T200 (workstream model), T048 (shell UI), T049 (startup screen)

## Objective

Make the T200 model usable: an always-visible surface that says what is
happening and what needs you, and a queue for triaging it. From design
artboards 5a/5b (the island), 4b (the attention queue) and 6a (where the island
goes — placement A, inline in the top bar, expanding downward).

A task must be manageable end to end by hand: create it, describe it, move it
through its states, block it on a question, answer that, review it, finish it.

## Dependencies

- T200 for the domain, its commands and its events.
- T048 for the token set and the shell's chrome; T049 for the launcher, which
  is what shows when no workspace is open.

## Required reading

- `docs/architecture/workstreams.md`, `docs/decisions/ADR-016-design-tokens.md`
- the design project `Gritty Agent Layout.dc.html`, artboards 4b, 5a, 5b, 6a
- `apps/desktop/src/features/terminal/{use-terminals.ts,terminal-reducer.ts}`
  (the hook + pure reducer shape every feature here follows)

## Allowed scope

- `apps/desktop/src/features/workstreams/**` (new)
- `apps/desktop/src/{App.tsx,app.css}`, `features/shell/app-bar.tsx`
- `crates/gritty-core/src/workstream/{wire,mod}.rs` — only to publish the
  allowed transitions (see the report)
- `docs/tasks/README.md`, `docs/architecture/workstreams.md`, this file

## Explicit non-goals

- No approvals, conflicts, failures, run logs, worktrees or diffs. Those come
  from agent runs (T300) and worktrees (T400); artboards 4a and 4c stay
  unimplemented until then.
- No cross-workspace aggregation (T410).
- No editing of a workstream's goal in the UI beyond what the island needs
  (the command exists; the form is follow-up work).
- No new Tauri capability, no direct filesystem or process access.

## Required behavior

1. A collapsed island lives in the top bar whenever a workspace is open: status
   dots, how many workstreams, and how many things need you.
2. ⌥A toggles it; Escape and a click outside close it; it never pushes the tab
   strip down and never occupies a layout track.
3. Expanded, it shows the single most urgent item in full (a question with its
   options, or work waiting for review), then the roster, then a field for
   adding a workstream.
4. The roster offers exactly the transitions the core allows for that state.
5. Answering a question resolves the decision through the core; the roster and
   the counts follow from the events, not from local guesses.
6. The attention queue replaces the editor body: filters (All, Questions,
   Reviews) with counts, items oldest first, and a detail rail for the selected
   one. Answering in either surface clears it in the other.
7. Every command failure is shown as the error envelope, not swallowed.

## Failure behavior

- A refused transition (`Conflict`) leaves the row where it was and shows the
  core's message. Nothing is applied optimistically.
- A question answered from two places: the second attempt is a `Conflict` and
  says so.
- No session database (`Io`): the island still renders, the error explains why
  nothing can be created.
- A workspace closing mid-flight clears the roster; a stray event for another
  workspace is ignored.

## Acceptance criteria

- A workstream can be taken from `DRAFT` to `DONE` entirely through the island.
- A question raised on running work shows in the island within one event, and
  answering it resumes the work without a reload.
- The queue and the island agree at all times.
- `pnpm check` green; the reducer and the attention rules are unit-tested.

## Tests required

- `attention.test.ts`: item building, ordering, filtering, roster order, labels.
- `workstreams-reducer.test.ts`: load, upsert, remove with cascade, workspace
  switching, event handling, duplicate response/event.
- Manual: the end-to-end pass in the running app (see the report).

## Commands to run

```
pnpm check
pnpm dev   # manual pass
```

## Completion report

### Summary

The island (5a/5b) in the top bar and the attention queue (4b) as a body view,
both over the T200 domain. A workstream can be created, moved, blocked on a
question, answered and finished without leaving the editor.

Placement follows artboard 6a's recommendation A: the island sits inline in the
bar where the window identity already is, and expands as a sheet that hangs
under it. It costs no layout track, so the tab strip never moves — which was
the trade-off B and D lose on.

The queue's five filters are three. Approvals, conflicts and failures come from
agent runs and worktrees; a filter that can never have anything in it teaches
the user to distrust the counts, so those arrive with the things they count.
The queue's footer says so in one line rather than pretending.

### Files changed

| File | What |
|---|---|
| `features/workstreams/attention.ts` (+ test) | what "needs you" means: item building, ordering, filters, labels, wait formatting |
| `features/workstreams/workstreams-reducer.ts` (+ test) | pure state: load, upsert, remove with decision cascade, workspace switching |
| `features/workstreams/use-workstreams.ts` | the controller: nine commands, one error slot, no optimistic updates |
| `features/workstreams/island.tsx` | the pill, the sheet, the question card, the roster, the add field |
| `features/workstreams/attention-queue.tsx` | filters, list, detail rail, footer |
| `features/shell/app-bar.tsx` | an island slot; the bar owns the slot, not the state |
| `App.tsx` | one controller for both surfaces; the queue swaps the body |
| `app.css` | the island and queue styles, from the existing tokens only |
| `crates/gritty-core/src/workstream/{wire,mod}.rs` | `Workstream.allowedTransitions` |
| `docs/architecture/workstreams.md`, `docs/tasks/README.md` | the UI section, status |

### Tests actually executed

- `pnpm vitest run apps/desktop/src/features/workstreams` — 21 passed.
- `pnpm check` — Biome, tsc, 318 vitest tests, `cargo fmt --check`,
  `clippy -D warnings`, `cargo test --workspace` (161 core), generated-types
  freshness: green.

**Not executed: the manual pass in the running app.** This agent has no screen
access on this machine, so the end-to-end walk (create → ready → start → raise
→ answer → review → done, plus ⌥A, Escape, click-outside and the queue) has not
been performed. Every acceptance criterion that says "in the running app" is
therefore unverified. It is the one thing left before this can be called done.

### Architectural decisions

1. **`Workstream.allowedTransitions` is published on the wire** (protocol
   2.3.0 → 2.4.0, additive). The alternative was a copy of the state machine in
   TypeScript, which would drift from `state.rs` the first time a transition
   changed. The core already computes it from the table it checks against, so
   the UI can offer exactly the legal moves and nothing else. This is the only
   core change in T210.
2. **Nothing is optimistic.** Every mutation waits for the core and applies what
   comes back. A workstream the core refused must not appear to have moved, and
   the refusals here — an illegal transition, an already-answered question — are
   exactly what the user needs to see.
3. **One controller, two surfaces.** The island and the queue read the same
   `useWorkstreams`, so 5b's rule ("resolving one clears the other") is free
   rather than something to keep in sync.
4. **The sheet is an overlay, not a track.** It is absolutely positioned under
   the bar; the shell grid is untouched, so opening it cannot move the editor.
5. **Closing on Escape and outside-click, not on keypress.** The artboard says
   "collapses on any keystroke in the editor"; listening for every keystroke
   would mean the island watching the editor's keys. Typing in the editor
   requires focus there, and focus leaving the sheet closes it, so the two
   rules coincide without the coupling.
6. **⌥A and ⌥1…9 are ignored while a plain `<input>` has focus** — on macOS
   Option composes characters, and a title someone is typing must not open a
   sheet. Monaco's hidden textarea is deliberately *not* excluded: ⌥A from the
   editor is the point of the shortcut.

### Known limitations

- No manual verification yet (above).
- The roster's transition buttons appear on hover/focus. On a touch screen
  they would need a tap target; Gritty is desktop-only today.
- Editing a title or goal after creation is not in the UI, though
  `workstreams.update` exists. Same for `workstreams.remove` and
  `decisions.raise` with structured options — the core supports both, the
  island only raises free-form questions through the queue's absence of a form.
  **Raising a question has no UI at all yet**: decisions can be answered and
  dismissed, but the only way to create one is the command. That is the largest
  gap and the first follow-up.
- `waitedFor` is computed at render, so a wait only refreshes when something
  else re-renders. A ticking clock was deliberately left out: a queue that
  updates every second invites watching instead of working.
- No virtualization. A workspace with hundreds of workstreams would render them
  all in the sheet.

### Follow-up tasks

- **Raise a question from the UI** (a form in the island and in the queue,
  including structured options). Needed before the island is genuinely usable
  end to end.
- **Edit title and goal**, and remove a workstream, from the roster.
- **T410** turns the queue into the real cross-workspace attention surface and
  adds the filters that agent runs make meaningful.
- **T300** replaces "raised by you" with real agent runs; the island's dots and
  the queue's rows are already shaped for them.

### Remaining risks

- The island competes with the health popover for the right end of the bar. On
  a narrow window both are cramped; nothing collapses yet.
- ⌥A may collide with a future terminal shortcut (the artboard shows ⌥1…9 for
  attaching terminals, which is not implemented). Whoever adds that owns the
  conflict.
