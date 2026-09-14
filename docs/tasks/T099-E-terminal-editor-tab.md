# Task T099-E — A terminal as an editor tab

- **Phase:** 2
- **Status:** todo
- **Depends on:** T099-S, T120-M (landed on `main`)

## Objective

Let a terminal session move **out of the dock and into the editor area**, as a
tab beside the open files, where it fills the whole editor pane — and back
again. Same shell, same process, same scrollback: moving a terminal is a change
of *where it is drawn*, never a new session.

## Motivation

The dock is a strip under the editor, sized for glancing. Some terminal work is
the main event rather than an aside: a long test run, a TUI (`htop`, `lazygit`,
`vim`), an agent CLI streaming output. Giving it the editor's area — the widest
and tallest surface in the window — is the point. Growing the dock to full
height does not do that; it takes the editor away and leaves a terminal in a
panel still shaped like a panel.

## What already exists

- **Non-document tabs are an established pattern.** ADR-026 made the agent
  console a tab (T233-C) without a wire change or migration, and T120-M added
  the settings page as the second kind — extending the pattern, generalising
  `showConsole` into `showSurface` so the next kind would not miss a site. A
  terminal is the **third kind**, and ADR-026's "Revisit when" names exactly
  this moment.
- **Nothing about a terminal tab reaches `SessionTab`.** `buildSessionTabs`
  walks `documents.state.order` only, and `persisted-layout.test.ts` ("an agent
  console is never persisted (ADR-026)") guards it. No protocol change, no
  ts-rs regeneration, no migration.
- **The xterm already outlives its container** once T099-S lands: each
  session's `Terminal` and host element live in a registry outside React and
  are appended into whichever slot shows them. That is what makes a move
  lossless, and it is why this ticket depends on T099-S rather than re-deriving
  the mechanism.
- **Every session already survives being hidden**: the reducer keeps sessions
  across workspace changes, so a shell the user can no longer see is never one
  they cannot find and kill.

## Decisions already made

1. **One session per terminal tab.** A split group does not move as a unit; a
   pane moved out of a group leaves the group, under T099-S's rules.
2. **Not persisted** (ADR-026's rule, for its reason). After a renderer reload
   every session is back in the dock, one group each — the same outcome
   T099-S accepts for groups.
3. **Never a preview tab.** Moving a terminal is a deliberate act; nothing a
   single click replaces.
4. **The tab's ✕ sends the session back to the dock. It never kills the
   process.** Every other ✕ in the strip closes a *view* and loses no work;
   a ✕ that silently terminated a running build would make this the one tab
   where closing destroys something (priority order: recoverability before
   ergonomics). Terminating and forgetting stay explicit, on the tab's context
   menu and on the pill.
5. **The pill stays in the dock** while its session is in the editor, marked
   as being there. The dock remains the one place that lists every running
   shell; clicking that pill activates its tab.
6. **Extend the pattern, do not build a registry.** A third group in the strip
   and a third term in `showSurface`, as T120-M did for the second. Hard rule 9
   applies; but record every site that needed a parallel edit in the ADR-026
   amendment, so whoever adds the fourth kind (T101-D's diff tab is queued) has
   the evidence for or against a registry.

## Dependencies

- **T099-S**: groups, the lifted xterm host, `visible` / `focused`.
- **T120-M must be committed to `main` first.** Its uncommitted change owns
  `editor-pane.tsx`, `tab-strip.tsx` and `App.tsx` — every mount point here.
- **Do not run in parallel with T101-D** (the diff as an editor tab), which
  mounts into the same three files and adds a non-document tab kind too.
  Whichever lands second updates the ADR-026 note's count.

## Required reading

- ADR-026 (agent console tab) in full, especially "Revisit when"
- `docs/tasks/T120-M-settings-mount.md` — the second kind, and §2 on
  `showSurface`
- `apps/desktop/src/features/editor/{editor-pane,tab-strip,status-bar}.tsx`
- `apps/desktop/src/features/settings/settings-tab.ts` and
  `apps/desktop/src/features/agents/agent-tabs-reducer.ts` — the two tab
  reducers this one sits beside
- `apps/desktop/src/features/session/persisted-layout.test.ts` — the ADR-026
  guard
- `docs/tasks/T099-S-terminal-splits.md` and its completion report

## Allowed scope

- `apps/desktop/src/features/terminal/**`
- `apps/desktop/src/features/editor/{editor-pane,tab-strip,status-bar}.tsx`
  and their tests
- `apps/desktop/src/features/session/persisted-layout.test.ts` — extending the
  guard only
- `apps/desktop/src/App.tsx` — wiring
- `apps/desktop/src/app.css` — tab and terminal rules
- `docs/decisions/ADR-026-agent-console-tab.md` — an amendment for the third
  kind
- `docs/architecture/terminal.md`
- this task file

## Explicit non-goals

- A terminal **beside** a file in the editor area. That needs editor groups,
  which Gritty does not have (follow-up).
- Split panes inside a terminal tab, or moving a group as a unit.
- Persisting which sessions were in the editor.
- Dragging a pill into the tab strip, or a tab into the dock (follow-up).
- A chord to move a terminal (the shortcut registry T335 asked for is owed
  first).
- Opening a *new* shell straight into a tab (follow-up; moving covers it in two
  clicks).
- A terminal in its own window.
- A generic non-document tab registry (decision 6).
- Any protocol, core or Rust change.

## Required behavior

### Placement

1. A session is placed either in the dock (in a T099-S group) or in the editor
   (in a terminal tab), never both and never neither. The reducer holds that
   invariant and is tested for it.
2. **Move to editor** — on the pill's context menu. The session leaves its dock
   group (T099-S's rules for focus and empty groups apply), a terminal tab
   opens after any existing terminal tabs, and it becomes the active tab. The
   dock's open/collapsed state is not changed.
3. **Move to dock** — on the tab's context menu, and what the tab's ✕ does.
   The session becomes its own group, placed after the dock's visible group,
   and focused; the dock opens (you asked to see it there). The editor pane
   hands back to the last active document, exactly as closing the settings tab
   you are looking at does.
4. The tab's context menu also has **Terminate** (disabled once exited) and
   **Close** (terminate and forget, as on the pill).
5. The xterm instance is **the same object** throughout. Moving in either
   direction never disposes or reconstructs it (T099-S behaviour 9): scrollback,
   cursor position and a running full-screen program survive. On arrival the
   terminal is fitted to its new container and the PTY is told the new grid.

### The tab

6. Terminal tabs are a group in the strip **after the agent consoles and
   before the settings tab**, never interleaved with anything, not
   drag-reorderable.
7. The tab reads like the pill: the session's name (T047) or the pill's
   shell-and-directory label, the T098 colour edge, and the status mark where a
   document's unsaved dot stands, carrying status by shape as well as colour.
8. The strip renders when a terminal tab is the only tab open (the early
   return in `tab-strip.tsx` counts terminal tabs).
9. Activating a terminal tab focuses its xterm. Activating a document, a
   console or the settings tab hands the pane away; the terminal tab stays
   open and inactive (the settings tab's `documents` rule).

### The pane

10. `showSurface` gains the terminal. Monaco is hidden, and the markdown
    toolbar, the "Editor settings" button and the document banners are not
    drawn — through `showSurface`, with no parallel condition added beside it.
11. The status bar shows the session's label, as it does "Settings", and no
    cursor position.
12. **⌘S with a terminal tab active saves nothing.** The editor pane's window
    listener saves `state.activeId` whenever focus is outside Monaco; with a
    terminal on screen that document is not visible, and saving it would be an
    action on something the user cannot see. The listener yields whenever
    `showSurface` is true — which deliberately changes the settings tab the
    same way; record it in the report.
13. Quick open (⌘P) keeps yielding to xterm's helper textarea, so `Ctrl+P`
    stays readline's inside a terminal tab.

### The dock's view of it

14. The pill of a session in the editor carries an "in editor" mark and its
    tooltip says so. Clicking it, or its ⌥ chord, activates the tab and does not
    open the dock.
15. T099-S's Split and "Move out of split" are disabled on such a pill, with a
    tooltip saying why.

## Failure behavior

- **The session exits while in a tab**: the tab stays, shows the exit line and
  the exited mark; ✕ still returns it to the dock, where Close forgets it.
- **The session is closed from the pill while its tab is open**: the tab
  disappears; if it was active the pane hands back to the documents.
- **The workspace is closed or switched**: terminal tabs stay. A running shell
  the user can no longer see is the failure the reducer's `workspace` case
  already refuses; a tab is how it stays seen.
- **Renderer reload**: sessions return to the dock, running. Nothing is killed.
- **The last document is closed while a terminal tab is showing**: the tab and
  the terminal are unaffected.
- **A move racing a close** (menu open while the session is closed elsewhere):
  the move is a no-op on an unknown session, never a tab for a ghost.
- **Nothing reaches `SessionTab`**: extend the ADR-026 guard in
  `persisted-layout.test.ts` to cover a terminal tab.

## Acceptance criteria

- `htop` running in the dock → Move to editor: it redraws at the editor's size
  and keeps running; → ✕: back in the dock at the dock's size, still running.
- Scrollback from before the first move is still there after two round trips.
- ⌥N on a session in the editor activates its tab; the dock stays as it was.
- With a terminal tab active, ⌘S writes nothing to disk (check the dirty dot
  on a modified document is still there).
- A terminal tab and a settings tab open together: switching between them and a
  document shows the right surface each time, and neither closes the other.
- ⌘R: every shell survives, all in the dock.
- Light and Dark: the tab's colour edge, status mark and the pill's "in editor"
  mark are all legible.

## Tests required

- **Reducer (Vitest):** placement invariant over a fixed-seed random action
  sequence; move to editor out of a group of three (focus and group rules);
  move to dock placement and focus; close from the pill while in a tab;
  rehydrate places everything in the dock; a move on an unknown session is a
  no-op.
- **Terminal tab state (Vitest):** open, activate, another surface taking the
  pane, ✕ returning to the dock, the active tab closing elsewhere.
- **Tab strip render (Vitest):** group order documents → consoles → terminals →
  settings; the strip renders with only a terminal tab; the ✕'s accessible name
  says it moves the terminal to the dock rather than closing it.
- **Editor pane:** ⌘S saves nothing while a terminal tab is active.
- **Persistence guard:** a terminal tab never produces a `SessionTab`.
- **DOM (`*.dom.test.tsx`, `@xterm/xterm` mocked):** the `Terminal` constructor
  runs once across dock → editor → dock, and `resize` is sent with the
  editor-sized grid on arrival.
- **Manual (macOS, `pnpm dev`):** every acceptance criterion above.

## Commands to run

`pnpm check`, `pnpm dev`.

## Completion report

### Summary

### Files changed

### Tests actually executed, with results

### Architectural decisions

### Known limitations

### Follow-up tasks

### Remaining risks
