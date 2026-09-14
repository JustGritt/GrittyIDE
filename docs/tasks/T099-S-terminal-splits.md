# Task T099-S — Split terminals: shells side by side in the dock

- **Phase:** 2
- **Status:** todo
- **Depends on:** T096, T098, T046, T120-T and T120-A (both landed on `main`)

## Objective

Let the dock show up to four shells **side by side**, in a *group*, instead of
exactly one. A split starts a new shell beside the focused one; each pane is
its own session with its own PTY, its own grid size and its own scrollback.

This is the first of two terminal-layout tickets. The second, T099-E (a
terminal as an editor tab), moves a session out of the dock entirely and builds
on the rendering change made here — see "The xterm outlives its container".

## Motivation

The dock already runs several shells, but shows one at a time: a dev server and
the shell you test it from, or a build and its log, cannot be watched together.
Switching pills with ⌥1…⌥9 is the workaround, and it loses sight of the one you
switched away from — exactly when it prints the error.

## What already exists (why this is renderer-only)

Established by reading the tree before writing this ticket:

- The core runs up to `MAX_SESSIONS = 16` concurrent sessions
  (`crates/gritty-core/src/terminal/mod.rs:42`), each with its own
  `terminal.write` / `terminal.resize` / `TerminalOutput`.
- `terminal-panel.tsx` already mounts **every** session's `TerminalView` and
  hides all but one with `display: none`. A split shows more than one.
- Each `TerminalView` observes its own container and reports its own grid
  (`terminal.onResize` → `terminal.resize`), so a narrower pane already tells
  its shell the narrower width.
- Output bypasses React through `output-bus.ts`, one listener per session, so
  two visible panes cost no extra renders.
- `components/splitter.tsx` (T046) is a generic, keyboard-operable gutter that
  drives a CSS custom property; it sizes panes in a row as readily as the dock.

So: **no protocol change, no Rust change, no ts-rs regeneration, no ADR.**
ADR-015, T090 and T096 each listed splits as out of their scope, not rejected.

## Decisions already made

Settled with the product owner before this ticket was written. Do not reopen
them inside the task; record disagreement as a follow-up.

1. **Groups.** A group is 1–4 sessions shown side by side. The dock shows one
   group at a time — the one holding the focused session. A session belongs to
   exactly one group.
2. **Side by side only.** No stacked (top/bottom) panes, no nesting. T046
   already excluded arbitrary split layouts, and hard rule 9 rules out a layout
   framework without a concrete need.
3. **At most four panes per group**, in one constant.
4. **Not persisted.** After a renderer reload every session comes back as its
   own group. The core does not know about groups, and teaching it would be a
   protocol change this feature does not need. No `persisted-layout.ts` change,
   no `LAYOUT_VERSION` bump, and pane widths are not persisted either — a width
   for a group that no longer exists means nothing.

## Dependencies

- T096 (the bar and the pill), T098 (colour edge, rooting a new shell), T047
  (names), T046 (`Splitter`).
- **T120-T must be committed to `main` first.** Its uncommitted change sits in
  `terminal-view.tsx`, the file this ticket rewrites most.
- **T120-A (make the editor and terminal obey their settings) must land
  first.** Its editor half is built but uncommitted, and its remaining terminal
  half rewrites how `use-terminals.ts` and `terminal-view.tsx` read the shell
  and font, and deletes `terminal-settings.tsx` from `terminal-panel.tsx` —
  the same three files this ticket restructures.
- **Do not run in parallel with T097** (shell integration and command blocks):
  it edits `terminal-view.tsx` and `terminal-panel.tsx` too. Sequence them.

## Required reading

- `docs/architecture/terminal.md`
- ADR-015 (terminal PTY), ADR-016 (tokens; every colour below comes from them)
- `docs/tasks/completed/T096-terminal-chrome.md` (the bar), `T046-resizable-panes.md`
- `apps/desktop/src/features/terminal/{terminal-panel,terminal-view,terminal-pill}.tsx`,
  `terminal-reducer.ts`, `output-bus.ts`, `terminal-chords.ts`
- `apps/desktop/src/features/editor/use-editor-binding.ts` — `attachContainer`:
  Monaco's DOM is a detached element appended into whichever container is on
  screen. That is the precedent for the xterm host below.
- AGENTS.md hard rules 8 and 10

## Allowed scope

- `apps/desktop/src/features/terminal/**`
- `apps/desktop/src/app.css` — the `terminal-panel`, `terminal-view` and
  `terminal-pill` rules only
- `apps/desktop/src/App.tsx` — only if `TerminalPanel`'s props must change
  (prefer that they do not)
- `docs/architecture/terminal.md` — a "Splits (T099-S)" section
- this task file

Not in scope: `components/splitter*` (use it as it is; if it cannot size a pane
in a row, stop and record why), `features/session/**`, `packages/protocol`,
`crates/**`.

## Explicit non-goals

- Stacked panes, nested splits, more than four panes.
- Persisting groups or pane widths across a reload.
- Joining two **existing** sessions into a split, or dragging pills between
  groups. A split always starts a new shell (follow-up).
- The same session in two panes. One PTY has one size, and `output-bus.ts`
  deliberately allows one view per session.
- Broadcasting input to every pane in a group.
- The session rail (artboard 8b) and a shortcut registry (T335's).
- A terminal in the editor area — that is T099-E.
- Any protocol, core or Rust change.

## Required behavior

### The model

1. `terminal-reducer.ts` holds groups: an ordered list of non-empty groups,
   each an ordered list of at most four `TerminalId`s. Invariants, true after
   every action: every known session is in exactly one group; no group is
   empty; no group exceeds four; the pill order is the groups flattened, so the
   members of a group are always adjacent.
2. `activeId` keeps its meaning of **the focused session**. The visible group
   is derived: the group containing `activeId`. Visible and focused are two
   different questions and nothing may conflate them again.
3. `opened` from "New terminal" or the dock's auto-start creates a **new
   group**, as today.
4. `opened` from a split carries the anchor session: the new session is placed
   immediately right of the anchor in the anchor's group and becomes focused.
   If the anchor is gone, or its group is already full, the new session becomes
   its **own group** right after the anchor's — the reducer never produces a
   fifth pane and never drops a session.
5. `unsplit` moves a session into its own group, placed right after the group
   it left.
6. `closed` removes a session from its group. If it was focused and its group
   still has members, focus moves to its left neighbour (right neighbour if it
   was leftmost). If its group is now empty the group is dropped and focus
   falls back as today.
7. `rehydrate` puts every session in its own group, in the core's order.
8. `focusPane` with `previous` / `next` moves focus within the visible group
   and **stops at the ends** (no wrap).

### The xterm outlives its container

9. **A layout change never recreates an xterm.** Splitting, unsplitting,
   closing a neighbour, switching groups and resizing a gutter must not dispose
   and reconstruct any session's `Terminal`. The core keeps no scrollback to
   replay, so a recreated xterm is an empty one — that would be silent data
   loss.
10. The chosen mechanism: **lift each session's xterm host element out of
    React's tree**, the way `use-editor-binding.ts` handles Monaco. A small
    per-session registry creates the `Terminal` and its host `<div>` once, on
    the session's first appearance, and disposes both only when the session is
    closed (`bus.forget` is the same moment). A pane is a slot that appends the
    host into itself and fits it after it lands.
    - Rejected: rendering every view as flat siblings positioned by CSS grid.
      It satisfies this ticket alone, but T099-E moves a terminal into the
      editor pane — a different subtree CSS cannot reach — and two mechanisms
      for one invariant is one too many.
    - After a re-append, fit and refresh the terminal; a moved DOM renderer can
      otherwise paint stale rows.
11. `TerminalView`'s `active` prop is replaced by the slot's position: whether
    a host is on screen (`visible`) and whether it should take keyboard focus
    (`focused`). Focus is taken when `focused` becomes true, as `active` does
    today, and never merely because a pane became visible.

### The dock

12. The visible group's panes are laid out in a row filling the surface.
    Between adjacent panes sits a `Splitter` (`orientation="vertical"`). New
    panes are equal width; dragging sizes the pane to the left of the gutter
    and the rightmost pane takes the rest. Double-click resets the group to
    equal widths. Arrow keys work because the `Splitter` already does.
13. A minimum pane width, in one constant, keeps roughly 20 columns at the
    default font. When the dock is narrower than the panes' minimums the panes
    shrink evenly; the surface never scrolls horizontally.
14. Widths live in panel state for the life of the renderer, per group, and
    reset to equal when a pane joins or leaves the group.
15. While a group has two or more panes, the focused pane is marked with a 1px
    `--accent` edge. A single pane looks exactly as it does today.
16. Pills stay **one per session** — the name, colour edge, status dot,
    rename, terminate and close all keep working per session. Members of a
    group are drawn joined (no gap, one shared outline); groups are separated by
    the ordinary gap. The focused session's pill is the active one; the other
    pills of the visible group carry a quieter "shown" state. Tokens only,
    readable in Light and Dark.
17. Clicking a pill shows its group and focuses that pane. ⌥1…⌥9 do the same,
    numbering sessions in pill order.

### Starting and ending a split

18. A **Split** button sits beside "New terminal". It starts the same shell,
    in the same directory, that "New terminal" would (T095 profile, T098 cwd),
    beside the focused session. It is disabled with a tooltip saying why when
    there is no session or the visible group is full.
19. The pill context menu gains **Split** (beside that session) and **Move out
    of split** (enabled only in a group of two or more). "Split" on a collapsed
    dock opens it, as a ⌥-chord does.
20. Chords, claimed **only while focus is inside a terminal pane** (so Monaco,
    inputs and the rename field never see them), refused in
    `attachCustomKeyEventHandler` so they never reach the shell, and matched on
    `code` as `terminal-chords.ts` requires:
    - Split: **⌘\\** on macOS; **Ctrl+Shift+5** elsewhere.
    - Focus previous / next pane: **⌥⌘←** / **⌥⌘→** on macOS;
      **Ctrl+Alt+←** / **Ctrl+Alt+→** elsewhere.
    - **Plain Ctrl+\\ is never claimed on any platform**: it is SIGQUIT in a
      shell.
    - Nothing else in the app binds these today (checked); verify again when
      the task starts.
21. An exited session stays in its pane with its exit line, input disabled,
    until it is closed — the same rule the dock has today.

## Failure behavior

- **A split's spawn fails** (the 16-session `Conflict`, a shell that no longer
  exists): the error envelope shows as for "New terminal"; the group is
  unchanged; no empty or placeholder pane is left behind.
- **The anchor is closed while its split is in flight**: the new session
  arrives as its own group (behaviour 4). It is never lost and never attached to
  an unrelated group.
- **Two splits in flight on a group of three**: the first fills the group, the
  second becomes its own group. Visible, and never a fifth pane.
- **A shell in a split exits or is killed**: its pane stays with the exit line;
  its neighbours are unaffected.
- **Renderer reload**: every session comes back running, one group each.
  Nothing is killed, nothing is hidden. This is the accepted limitation.
- **Gutter drags**: xterm raises `onResize` only when the *grid* changes, so
  `terminal.resize` traffic is bounded by column changes, not pointer events.
  Confirm this in the manual pass (count calls during a drag); if a drag
  produces more than one resize per pane per frame, coalesce it — rule 8.
- **A session closed from anywhere** disposes its xterm and host exactly once;
  a late `snapshot` for it must not resurrect a pane (the reducer already
  refuses this; keep the test).

## Acceptance criteria

- Split twice: three shells side by side, each with its own prompt; typing goes
  only to the focused one.
- After dragging a gutter, `stty size` reports different column counts in
  adjacent panes, and each matches what is drawn.
- `vim` in one pane redraws correctly while its gutter is dragged.
- `yes` running in one pane does not make typing in its neighbour lag.
- Unsplit, split again, close a neighbour, switch to another group and back:
  scrollback in every pane is intact.
- ⌘\\ in a full group does nothing and the Split button says why.
- Ctrl+\\ in a pane still sends SIGQUIT to the shell.
- ⌘R reloads the renderer: every shell survives as its own pill.
- Light and Dark: the joined pills, the "shown" state and the focus edge are
  all distinguishable.

## Tests required

- **Reducer (Vitest):** split beside an anchor; the cap (fifth → own group);
  anchor closed before the split resolves; close the focused pane (left, then
  right neighbour); close the last member drops the group; unsplit placement;
  rehydrate is one group each; `focusPane` stops at both ends; a `snapshot` for
  a closed session does not resurrect it. Plus one sequence test that applies a
  long fixed-seed random run of actions and asserts the invariants of
  behaviour 1 after each step.
- **Chords (Vitest):** split and focus chords per platform, refused with extra
  modifiers, and plain Ctrl+\\ refused on every platform.
- **DOM (Vitest, `*.dom.test.tsx`, `@xterm/xterm` mocked):** the `Terminal`
  constructor runs once per session across split → unsplit → close neighbour →
  switch group; two visible panes report `resize` with their own ids.
- **Manual (macOS, `pnpm dev`):** every acceptance criterion above. The
  non-macOS chords are specified but **not claimed as verified** until Linux CI
  returns (CI-LINUX-DARK, Linux CI dark until the minute allowance resets).

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
