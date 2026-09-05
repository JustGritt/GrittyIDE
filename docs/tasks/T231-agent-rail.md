# Task T231 — The agent rail: a dedicated region for running agents

- **Phase:** 3
- **Status:** not yet started
- **Depends on:** T046 (resizable panes), T060 (layout persistence), T220, T230

## Objective

A third region in the shell, to the right of the editor, listing the agent
sessions running on the machine with a colour per status — and, when one is
selected, its timeline (T232) beneath the list. Beside your code, not instead
of it.

**It is deliberately not a tab.** Agent sessions and open files are different
things and mixing them in one strip makes both harder to read. Keeping them
apart also avoids the only expensive part of the alternative: `SessionTab.path`
is a required `WorkspacePath` backed by a non-nullable SQLite column
(`crates/persistence/src/store.rs`, `TabRow`), so a non-document tab would need
a wire change, a ts-rs regeneration and a migration. The rail's width lives in
the UI-owned layout blob instead and costs none of that.

## Dependencies

- **T230** for `agents.openTimeline` and `AgentTimelineAppended`.
- **T220** for `useAgents` and `AgentRoster`.
- **T221** for `BLOCKED`. Without it the rail renders four statuses honestly and
  simply never shows "Pending validation"; it must not fake it.
- **T046** for `Splitter`, `PANE_LIMITS` and the pane-size layout convention.
- **T060** for `session.update` and the UI-owned layout blob.

## Required reading

- `apps/desktop/src/App.tsx` — `shell__body`, both existing `Splitter` call
  sites, and `chords={!islandOpen}` (the manual chord gating)
- `apps/desktop/src/components/{splitter.tsx,splitter-geometry.ts}` —
  `orientation`, `direction`, `target`, `property`, `bounds`, `boundsWithin`
- `apps/desktop/src/features/session/persisted-layout.ts` — `Layout`,
  `PANE_LIMITS`, `DEFAULT_LAYOUT`, `parseLayout`, `serializeLayout`
- `apps/desktop/src/features/agents/{use-agents.ts,agents-reducer.ts,agents-panel.tsx}`
- `apps/desktop/src/features/workstreams/island.tsx` — the surface being reduced
- `apps/desktop/src/features/terminal/terminal-chords.ts` — the chord pattern
- `docs/decisions/ADR-016-design-tokens.md` — colours come from tokens
- `docs/architecture/terminal.md`, section "The dock" — the pill design and the
  rule that separates what a session *is for* from whether it is *running*

## Allowed scope

- `apps/desktop/src/features/agents/**`
- `apps/desktop/src/features/workstreams/island.tsx` (reduction only)
- `apps/desktop/src/features/session/persisted-layout.ts` + its test
- `apps/desktop/src/{App.tsx,app.css}`
- `docs/architecture/agent-sessions.md`, `docs/tasks/README.md`, this file

## Explicit non-goals

- **No agent entries in the document tab strip.** The separation is the point.
- **No Rust changes.** No new commands, no protocol bump, no schema change. If
  the rail needs something the core does not expose, that is a T230 gap: record
  it, do not add it here.
- **No timeline rendering.** T232 owns the feed and the files-touched list; this
  task ships the region, the list, the selection and the status colours, with a
  placeholder body.
- **No control.** No button acts on a session. The rail is a read.
- **No second roster.** The island must not keep its own copy — see §7.
- **No persistence of which session was selected.** A session id is ephemeral
  and rediscovered each launch; restoring a selection that no longer exists is
  worse than restoring nothing.
- **No inventing a status.** If the core cannot produce a value, the rail does
  not display it.

## Required behavior

### The region

1. A third child of `shell__body` in `App.tsx`, after the editor, with its own
   `Splitter`. Follow the sidebar call site exactly, with two differences:
   `direction={-1}` (the rail is anchored right, so dragging left grows it) and
   its own CSS custom property, e.g. `--agent-rail-width`.
2. `PANE_LIMITS` gains an `agentRail` entry and `Layout.panes` gains
   `agentRailWidth`, persisted through `session.update` like the other two.
   `parseLayout` must accept a blob written **before** the rail existed and fall
   back to the default — there is already a test for exactly this case with the
   terminal section, and it is the model to copy.
3. The rail collapses and expands, and a collapsed rail still shows the count of
   sessions that need you. The dock made this choice for the same reason: a
   collapsed surface that stops reporting is worse than one that takes space.
4. Collapsed state persists in the same blob.

### The list

5. One row per session, ordered by the core's ranking (T220 already sorts
   workspace-first, then blocked-on-you, then running, then recency). The
   renderer does not re-sort; if the order is wrong that is a core change.
6. Each row shows the provider, the session's title, its working directory and
   its status. A row for a session in another workspace is visually distinct —
   the roster is machine-wide and `inWorkspace` already says which.
7. **The rail and the island read the same controller.** `useAgents` is lifted
   so both consume one `AgentsController`, following T210's rule that two
   surfaces over one controller cannot disagree. The island's agents section is
   **reduced** to a count plus its single most urgent row; the rail becomes the
   primary surface. Two independent rosters would drift, and one of them would
   be wrong.

### Status colours

8. Five statuses, every colour from an ADR-016 design token — never a literal
   hex, so the light and dark ramps follow without a second palette:

   | Rail label | Wire value | Where it comes from |
   |---|---|---|
   | Working | `WORKING` | transcript — a tool call is outstanding |
   | Pending validation | `BLOCKED` | **hook evidence only** (T221) |
   | Done | `WAITING` | transcript — turn finished, your move |
   | Idle | `IDLE` | transcript quiet past the threshold |
   | Ended | `ENDED` | no live process |

9. **"Done" is the rail's label for `WAITING`.** The wire value keeps T220's
   name, because it means *the user's move* and renaming it would ripple through
   the core for a word. The mapping lives in one place in the renderer so the
   two cannot drift.
10. Status is conveyed by **more than colour** — a label, and a shape or icon —
    so the rail is readable without colour vision. `docs/architecture/terminal.md`
    already separates a session's colour from its liveness dot for a related
    reason; here status and liveness are one axis, and that is worth stating in
    the task rather than leaving implicit.
11. When the process table is unreadable, `processEvidence: false` already
    travels on the roster: the rail says so rather than showing everything as
    Ended and implying the machine is quiet.

### Selection and keyboard

12. Selecting a row calls `agents.openTimeline`; changing selection closes the
    previous one, so exactly one follower runs. Closing the rail closes the open
    timeline — an invisible follower is waste.
13. A toggle chord following `terminal-chords.ts`: match on `event.code`, not
    `event.key`, because Option composes characters on macOS, and
    `preventDefault` so the composed character never reaches a focused xterm.
14. **Gate it manually against the other listeners.** There is no central
    shortcut registry; `App.tsx` already passes `chords={!islandOpen}` to the
    terminal panel for this exact reason. Adding a fourth ad hoc listener
    without gating it is how two handlers fire on one keystroke.

### State

15. A new pure reducer `agent-rail-reducer.ts` with unit tests, following
    `agents-reducer.ts`: selection, collapsed state, and the timeline handle.
    Roster state is not duplicated into it — it comes from the shared
    controller.
16. Nothing is applied optimistically (T210's rule): every call waits for the
    core, and a refusal renders the error envelope rather than being swallowed.

## Failure behavior

| Situation | Behavior |
|---|---|
| No agents running | The rail shows an empty state, not an error. The normal case for most users |
| Process table unreadable | The rail says why (§11) instead of showing everything Ended |
| Selected session leaves the roster | The row goes to Ended and stays selected; its timeline stops growing. It is not removed under the pointer |
| `openTimeline` fails | Error envelope in the rail body; the list keeps working |
| T221 not installed | Four statuses; "Pending validation" simply never appears |
| Layout blob predates the rail | `parseLayout` falls back to the default width, as the terminal section already does |
| Stored width outside `PANE_LIMITS` | Clamped by the same bounds the gutter enforces |
| Workspace closed | The rail keeps listing — the roster is machine-wide and T090 set the precedent that a surface must not hide processes the user can still act on |
| Window too narrow for three regions | The rail collapses rather than squeezing the editor below its minimum |

## Acceptance criteria

- With Claude Code running in another terminal, the rail lists it, coloured by
  status, and the status changes as the session does.
- Dragging the gutter resizes the rail; the width survives a restart; a layout
  blob written before this task loads at the default without error.
- Collapsing the rail keeps the needs-you count visible.
- The island shows the same count as the rail at all times — they read one
  controller, so this cannot be arranged to disagree.
- The toggle chord works from inside a focused terminal and does not insert a
  character; it does not fire while the island is open.
- Selecting a session opens exactly one timeline; switching closes the previous.
- With T221 absent, no status ever reads "Pending validation".
- `pnpm check` green.

## Tests required

- **Unit (TS):** `agent-rail-reducer` — selection, switching, clearing when the
  selected session leaves the roster, collapse.
- **Unit (TS):** the status → label/token mapping is total over `AgentActivity`,
  so a value added to the wire cannot render blank.
- **Unit (TS):** `persisted-layout` — the rail width round-trips; a blob written
  before the rail existed parses to the default; an out-of-range width clamps.
- **Unit (TS):** the chord matcher, following `terminal-chords.test.ts`.
- **Manual:** run Claude Code in another terminal; watch the rail through
  working → done → ended. Resize, collapse, restart, confirm the width. Confirm
  the chord does not fight the terminal's `⌥1…⌥9` or the island's `⌥A`.

## Commands to run

```
pnpm check
```

## Completion report

_(filled in by the implementing session)_
