# Task T096 — Terminal chrome from design artboard 8a

- **Phase:** 0 / 2
- **Status:** done
- **Depends on:** T047, T048, T090, T095

## Objective

Replace the terminal dock's two stacked rows with the one 32px row from design
artboard 8a: an uppercase `TERMINAL` label that is also the disclosure, sessions
as pills carrying `dot · shell · directory · status`, `⌥1…9` to select one, a
`New terminal | zsh ⌄` split button, and the settings behind `⋯`. Terminating
demotes to the `✕` on the pill you are on.

## Motivation

The artboard's own critique of what ships today: it "spends 70px on two stacked
rows, names a tab `$SHELL`, and styles `Terminate` exactly like `New terminal`".
All three are true, and the third is the one that matters — the destructive
control and the constructive one are the same quiet button, distinguished only
by their text.

The first is a real cost too. The dock is a horizontal band at the bottom of a
window whose vertical budget is already contested (`EDITOR_MIN_HEIGHT` is
200px); 70px of chrome to show one line of a shell is chrome winning.

## Dependencies

T090 (sessions, `terminal.kill`), T095 (`terminal.profiles`, the settings
popover, sessions without a workspace), T047 (`terminal.rename`), T048/ADR-016
(the tokens every colour here comes from), T046 (the dock splitter).

## Required reading

- Design artboard 8a in the Claude Design project `Gritty Agent Layout.dc.html`
- `docs/architecture/terminal.md`, ADR-015, ADR-016
- `docs/tasks/T047-terminal-names.md`, `docs/tasks/T095-terminal-profiles.md`
- `apps/desktop/src/features/workstreams/island.tsx` — the ⌥ chord it already owns

## Allowed scope

- `apps/desktop/src/features/terminal/**`
- `apps/desktop/src/App.tsx` (the dock's composition only)
- `apps/desktop/src/app.css` (the terminal dock section, and the popover modifier)
- `apps/desktop/src/components/menu.tsx` (promoted from
  `features/editor/tab-menu.tsx`), `apps/desktop/src/features/editor/tab-strip.tsx`
  (its import)
- `docs/architecture/terminal.md`, `docs/tasks/README.md`, this file, and the
  two documents T097 needs

## Explicit non-goals

- **Command blocks.** 8a's body — output grouped per command with a duration and
  an exit code — needs OSC 133 shell integration. Specified as T097.
- **Agent-owned pills.** 8a draws blue and amber pills for shells an agent is
  driving, with an attention badge. There is no data behind either: T220 is
  read-only discovery and T410 owns the queue. No tone variant is invented for
  them.
- **The session rail (8b), splits, search, scrollback persistence.**
- Any protocol, core or Rust change.

## Required behavior

1. One 32px row (`--section-h`), always visible, outside the collapsible body.
   A collapsed dock still shows its pills; the count badge is deleted rather
   than kept, because the thing it counted is now on screen.
2. The `TERMINAL` label is the disclosure, carrying the `aria-expanded` the
   removed toggle had, and `aria-controls` the body it hides.
3. A pill is `dot · label · directory · status`. The label is `terminalLabel`
   (T047). The directory is the workspace's own name at its root, the
   workspace-relative path deeper in, `~` for a home session, and nothing at all
   for the root of a workspace that is no longer open — its name is not on the
   wire, and the tooltip still explains.
4. The dot says running (`--ok`), ended cleanly (`--fg-gutter`) or ended with
   something to explain (`--error`).
5. `✕` is contextual: on a running session it terminates, and the pill stays so
   the exit status can be read; on an ended one it removes the pill. Both are
   idempotent in the core, so neither is ever an error. Right-click offers
   Rename, Terminate and Close, so removing a running session stays one step.
6. `⌥1…⌥9` select the *n*th session from anywhere, including from inside a
   focused terminal. A chord that lands on a collapsed dock opens it. The hint
   is printed on the bar once there is more than one session.
7. "New terminal" and the shell it starts are one split button; the caret half
   appears only when more than one profile is startable. The choice persists
   exactly as T095 made it.
8. `⋯` is the T095 settings popover, opening upward — the dock is the bottom of
   the window.
9. F2 and double-click still rename, one pill at a time, and the row does not
   jump while a rename is open.

## Failure behavior

- A remembered profile that is gone still falls back to the default
  (`selectedProfile`); the caret label follows the fallback, not the memory.
- A name the core would refuse is dropped on commit, as before: the bar is not
  a place for a validation message.
- `⌥7` with three sessions open consumes the keystroke and does nothing, rather
  than letting the character Option composed reach whichever shell has focus.
- With the island open, `⌥1…9` answer its decision and never reach the dock.
- A session whose workspace has been replaced keeps its pill and its ✕; the core
  did not kill it, so the UI must not hide it (T095).

## Acceptance criteria

- The dock's chrome is one 32px row, collapsed or not.
- Three shells read `zsh GrittyIDE`, `zsh crates/terminal`, `zsh ~`.
- `⌥2` from inside a focused terminal switches and prints nothing.
- `✕` on a running pill leaves `exit N` behind; `✕` again removes it.
- The caret lists only `available` profiles and the choice survives a restart.
- `⋯` opens over the editor, never off the bottom of the window.
- Nothing in `crates/`, `packages/protocol` or `src-tauri` changed.

## Tests required

- Vitest, `terminal-naming.test.ts`: every `pillLocation` branch including the
  foreign-workspace `null`, the three `pillTone` cases including a signal, and
  `pillStatus`'s `exit 101` / `exited` / `killed SIGKILL`.
- Vitest, `terminal-chords.test.ts`: `Digit1`…`Digit9`, `Digit0` refused, every
  modifier combination refused, non-digit codes refused, and the macOS point —
  matching happens on `code`, so the composed character is irrelevant.
- The existing terminal suites must pass unchanged; if one needs editing, the
  boundary was crossed.
- Manual: `docs/testing/phase-0-manual-pass.md` session 7.

## Commands to run

`pnpm check`, `pnpm dev`.

## Completion report

### Summary

Design artboard 8a's header, in full. The dock's chrome is one 32px row: the
`TERMINAL` label is the disclosure that used to be a row of its own, sessions
are pills carrying where they are and how they ended, `⌥1…9` select one, "New
terminal" and the shell picker are one split button, and the settings are behind
`⋯`. Terminate is now the `✕` on the pill you are on.

No protocol change, no Rust change, no new dependency. Every colour is an
existing ADR-016 token, so the light ramp followed for free.

8a's body — output grouped per command — is **not** here: it needs OSC 133 shell
integration, which T090, T047, ADR-015 and `docs/architecture/terminal.md` each
defer to a task of its own. It is now specified as T097 with a draft ADR-017.

### Files changed

- `apps/desktop/src/components/menu.tsx`: moved from
  `features/editor/tab-menu.tsx` and renamed `Menu` / `MenuAnchor` / `MenuItem`
  (CSS `.menu`, `.menu__item`). Its second consumer arrived, which is the
  trigger ADR-016 names for promoting a shared thing.
- `apps/desktop/src/features/editor/tab-strip.tsx`: the import.
- `apps/desktop/src/features/terminal/terminal-naming.ts`: gains `pillTone`,
  `pillLocation`, `pillStatus`, `describeCwd`, `PillWorkspace`. They live beside
  `terminalLabel` because they answer the same question — what a session says
  about itself — and because this app's vitest project matches `.test.ts` only,
  so logic worth a test cannot live in the component.
- `apps/desktop/src/features/terminal/terminal-chords.ts` (new):
  `terminalChordIndex`, `TERMINAL_CHORD_HINT`.
- `apps/desktop/src/features/terminal/terminal-pill.tsx` (new): the pill, its
  rename field and its context menu.
- `apps/desktop/src/features/terminal/terminal-panel.tsx`: rewritten header, the
  chord effect, the split button, and the body it now owns.
- `apps/desktop/src/features/terminal/terminal-view.tsx`: one
  `attachCustomKeyEventHandler` that refuses the chord.
- `apps/desktop/src/features/terminal/terminal-settings.tsx`: the summary is
  `⋯`, and the popover opens upward.
- `apps/desktop/src/App.tsx`: the dock is the panel; the toggle and the count
  badge are gone.
- `apps/desktop/src/app.css`: the terminal dock section rewritten;
  `.prefs--up`; `.tab-menu*` renamed `.menu*`.
- Tests: `terminal-naming.test.ts` (+22 cases), `terminal-chords.test.ts` (new).
- Docs: `docs/architecture/terminal.md`, `docs/tasks/README.md`,
  `docs/testing/phase-0-manual-pass.md`, `docs/tasks/T097-…`,
  `docs/decisions/ADR-017-…`.

### Architectural decisions

- **The panel owns the dock.** The bar could not be the always-visible row while
  it lived inside the element the disclosure hides. Moving the collapse into the
  panel is what makes one row possible, and it has a second effect worth having:
  a collapsed dock still says which shells are running.
- **`✕` is contextual rather than one meaning.** Terminating and closing are two
  different things — one keeps the exit status readable, the other admits you
  are finished with it — and collapsing them into "kill and forget" would have
  made an ended session's status unreachable for the sessions the user dismisses
  themselves. The context menu keeps the one-step removal.
- **`⌥1…9` is refused twice.** `attachCustomKeyEventHandler` stops xterm sending
  the chord to the shell; the panel's `preventDefault` stops the browser
  inserting the character macOS composed (`⌥1` is `¡`). Neither alone silences
  it, which is why both are there.
- **The chord is gated on the island, not sniffed from the DOM.** `island.tsx`
  binds the same chord to a decision's options, and `isTypingInField` there
  deliberately lets xterm's helper textarea through. `chords={!islandOpen}` is a
  fact the shell already has.

### Known limitations

- 8a's blue and amber agent pills and the attention badge are not built; there
  is no data behind them yet.
- The `✕` reserves its width at zero opacity so the row does not reflow under
  the pointer, which is what the tab strip already does. An inactive pill is
  therefore slightly wider than the artboard draws it.
- No component test covers the pill or the bar: this app's vitest project runs
  in `node` and collects `.test.ts` only. Everything worth asserting was moved
  into `.ts` modules instead; a jsdom project would be its own change.
- Not yet exercised by a pointer — the manual pass is owed (see
  `HANDOFF.md`).

### Follow-up tasks

- **T097** — shell integration and command blocks, 8a's body.
- A jsdom vitest project so `.tsx` can be tested at all.
- Reordering pills by drag, which the editor's tab strip now has and the dock
  does not.
- 8b's session rail, once more than about six shells is a real case.
