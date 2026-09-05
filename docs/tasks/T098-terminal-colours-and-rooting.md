# Task T098 — Organising terminal sessions: colours, and starting where you work

- **Phase:** 0 / 2
- **Status:** review
- **Depends on:** T047, T090, T095, T096

## Objective

Two changes, both about a dock that has stopped being one shell:

1. **A user colour per session.** `terminal.recolor` gives a session one of six
   hues, drawn as a bar down its pill's left edge, so groups of shells can be
   told apart at a glance.
2. **A new terminal starts where you are working** — the folder of the document
   being edited, not the workspace root.

## Motivation

T096 made a session legible: a pill says which shell, where it is and how it
ended. What it cannot say is *what the session is for*. A name (T047) answers
that, but naming six shells is six deliberate acts; grouping them by colour is
one click each and survives not reading the text at all.

The second is a smaller and more constant tax. `terminal.spawn` roots at the
workspace root, so someone editing `crates/terminal/src/system.rs` who opens a
shell types the path that is already on their screen, every time.

## Dependencies

T047 (`terminal.rename`, whose reasoning for core state this copies), T090
(`SpawnTerminalRequest.cwd`, which has always accepted a workspace-relative
path nothing sent), T095 (rooting rules), T096 (the pill and its context menu).

## Required reading

- `docs/architecture/terminal.md` — "Naming a session (T047)" is the argument
  this reuses
- ADR-010 (the renderer never names a directory), ADR-016 (the palette)

## Allowed scope

- `crates/gritty-core/src/terminal/{wire.rs,mod.rs}`,
  `crates/gritty-core/src/protocol/{mod.rs,export.rs}`
- `packages/protocol/src/{client.ts,commands.ts,types.ts}` + generated
- `apps/desktop/src-tauri/src/{commands.rs,lib.rs}`, `tests/commands.rs`
- `apps/desktop/src/features/terminal/**`, `components/menu.tsx`, `App.tsx`,
  `app.css`

## Explicit non-goals

- **Free-form colours.** The palette is a closed set; see the decision below.
- **Persisting a colour past the session.** Terminal ids are per-session and
  die with the process, so there is nothing durable to key a colour to. A colour
  that outlived its shell would need a concept of a *named* session, which is
  T095's deferred "named profiles".
- **Colouring anything but the pill** — not the terminal surface, not the
  scrollback, not the status bar.
- **A setting for the rooting rule.** One behaviour; if it turns out to be
  wrong, it is a preference then, not speculatively now.

## Required behavior

1. `terminal.recolor({ terminalId, color? })` sets or clears a session's colour
   and returns the updated snapshot, like `rename` and `resize`. It emits no
   event: nothing else reacts to a colour.
2. The colour is one of `RED GREEN YELLOW BLUE MAGENTA CYAN`, or absent. An
   unknown value is `InvalidInput` — the palette is closed on the wire, not just
   in the picker.
3. A colour is allowed on an ended session, like a name: an ended shell still
   sits in the group it belonged to.
4. The colour is drawn as a 3px bar down the pill's left edge. **It never
   replaces the status dot** — a group you coloured yellow must not stop telling
   you that one of them died.
5. The pill's context menu picks the colour: "No colour" and the six hues, as a
   radio group with the current one checked.
6. `New terminal`, and the dock's auto-start, root the shell at the folder of
   the active document, relative to the workspace root; the workspace root when
   nothing is open, or when the active tab belongs to another workspace. The
   button's tooltip says which.
7. The renderer still never names a directory: it sends a workspace-relative
   `cwd` and the core resolves it against the root (ADR-010).

## Failure behavior

- A `cwd` whose directory has been deleted or renamed since the tab was opened
  falls back to the workspace root rather than failing a spawn the user cannot
  act on — the same call T095 makes for a stale shell profile. Only the
  fallback's failure is reported.
- `cwd` is never sent without a `workspaceId`; the core would refuse it, and
  correctly.
- Colouring an unknown session is `NotFound`, like renaming one.
- Clearing goes back to *no* colour, never to a default one.
- The dock's auto-start no longer races the workspace answer: `ready` now
  includes it, so the first shell cannot land in the home directory while a
  workspace is open. A core that cannot answer settles the flag anyway rather
  than leaving the dock waiting forever.

## Acceptance criteria

- Right-click a pill → Blue puts a blue bar on it; the dot still says whether
  the shell is alive.
- The colour survives a renderer reload (`terminal.list` returns it).
- "No colour" removes the bar.
- With `crates/terminal/src/system.rs` open, "New terminal" lands in
  `crates/terminal/src` and the pill says so.
- With `README.md` open, it lands at the workspace root, exactly as before.
- With no folder open, it lands in `~`, exactly as before.
- Both ramps: no colour is unreadable in Light.

## Tests required

- Rust: colour set, cleared, allowed on an ended session, `NotFound`, and
  present in the `list` snapshot.
- Tauri: `terminal_recolor` round-trip, the colour surviving a relist, clearing,
  and an unknown hue refused as `InvalidInput`.
- Vitest: `cwdForDocument` for a nested file, a top-level file (`''`, the root)
  and nothing open; the palette's token/label/class agreement.

## Commands to run

`pnpm check`, `pnpm dev`.

## Completion report

### Summary

`terminal.recolor` plus a colour bar on the pill, and new terminals that start
in the folder of the document being edited. Protocol **2.7.0** (additive).

### Files changed

- `crates/gritty-core/src/terminal/wire.rs`: `TerminalColor`, `Terminal.color`,
  `RecolorTerminalRequest`.
- `crates/gritty-core/src/terminal/mod.rs`: `recolor`, four tests.
- `crates/gritty-core/src/protocol/{mod.rs,export.rs}`: 2.6.0 → 2.7.0, exports.
- `apps/desktop/src-tauri/src/{commands.rs,lib.rs}`, `tests/commands.rs`: the
  adapter, its registration and one IPC test.
- `packages/protocol/src/{client.ts,commands.ts,types.ts}`: `terminal.recolor`.
- `apps/desktop/src/components/menu.tsx`: `MenuItem` gains `swatch`, `checked`
  (which makes the item a `menuitemradio`) and `separatorBefore`.
- `apps/desktop/src/features/terminal/terminal-colors.ts` (new),
  `terminal-cwd.ts` (new), and tests for both.
- `terminal-pill.tsx`: the edge and the palette menu.
- `terminal-panel.tsx`, `App.tsx`: the cwd, and `onRecolor`.
- `use-terminals.ts`: `recolor`, a cwd-aware `spawn` with the root fallback, and
  `located` in the readiness gate.
- `app.css`: `.terminal-pill__edge--*`, `.menu__swatch`, `.menu__separator`, the
  checked marker.

### Architectural decisions

- **The colour is core state, for T047's exact reason.** `terminal.list` is what
  rehydrates the panel after a renderer reload, so a colour kept in the renderer
  would vanish while the session it grouped kept running. Writing a doc that
  said names survive a reload but colours do not would be incoherent.
- **A closed palette, not a hex string.** The palette is CSS custom properties
  with a light ramp as well as a dark one (ADR-016); a colour picked against one
  would be unreadable in the other, and the core has no business holding a value
  it cannot reason about. The six ANSI hues are the ones the terminal already
  paints with, so a pill and the output under it agree.
- **The colour is an edge, not the dot.** Two independent facts — what this
  shell is for, and whether it is alive — need two channels. Overloading the dot
  would trade an inspectability signal for a decorative one.
- **The rooting fallback is silent, and that is deliberate.** Hard rule 7 is
  about dirty-buffer conflicts, where silence loses work. Here the cwd is a
  *convenience default* the user did not ask for; failing the spawn because a
  folder moved would be a worse answer than starting at the root, and T095 set
  the precedent with stale shell profiles.

### Known limitations

- A colour dies with the session, because a terminal id does. Colouring a shell,
  quitting and expecting the colour back needs named sessions (a T095 non-goal).
- Six hues, no custom set, no per-workspace defaults.
- The rooting follows the *active tab*, so switching tabs changes where the next
  terminal starts. That is the intent, but it means the answer is only visible
  in the "New terminal" tooltip until the shell exists.
- Not yet exercised by a pointer; see the manual pass.

### Follow-up tasks

- Colour the terminal surface's edge as well, so the active session's identity
  is visible while typing into it.
- A colour for a *workstream*, so a shell an agent is driving inherits it
  (T300).
