# Task T046 — Resizable panes

- **Phase:** 1A
- **Status:** review
- **Depends on:** T040 (editor pane), T060 (persisted layout), T090/T095 (dock)

## Objective

Let the user size the shell's panes by dragging the gutters between them, with
a keyboard equivalent, and remember the sizes across restarts. Today the
sidebar is a fixed `minmax(18rem, 24rem)` grid track and the terminal dock is a
hardcoded 280px.

## Dependencies

The persisted layout blob (T060) already carries UI-owned state and is the
right home for pane sizes.

## Required reading

- `docs/architecture/overview.md` (shell layout)
- `apps/desktop/src/features/session/persisted-layout.ts` (blob conventions)
- Plan: priority order — responsiveness before ergonomics; rule 8 on
  high-frequency work in the renderer

## Allowed scope

- `apps/desktop/src/components/splitter*` (new)
- `apps/desktop/src/App.tsx`, `app.css`
- `apps/desktop/src/features/session/persisted-layout.ts` (two sizes, a version)

## Explicit non-goals

- Arbitrary split layouts, nested splits, or draggable panes (a workstream
  concern, T210 at the earliest).
- Collapsing a pane by dragging it to zero; the dock already has a toggle.
- Persisting sizes per workspace; the layout blob is global.
- Resizing the editor's own internal split (markdown preview owns that).

## Required behavior

1. A gutter between the sidebar and the editor resizes the sidebar's width; a
   gutter on the terminal dock's top edge resizes the dock's height.
2. Dragging is bounded: neither pane can be dragged past its minimum, and
   neither may squeeze the other below its own minimum.
3. Each gutter is a real `separator` widget: focusable, with
   `aria-orientation`, `aria-valuenow/min/max`, and arrow keys that move it by
   a step (`Home`/`End` jump to the bounds). Double-click restores the default.
4. Sizes persist with the rest of the layout and are restored at startup; a
   stored size outside the current bounds is clamped rather than discarded.
5. The terminal dock's gutter only exists while the dock is open.
6. Dragging does not re-render the application per pointer event.

## Failure behavior

- A window too small for both minimums clamps rather than producing a negative
  or overlapping track.
- A blob written before this task, or carrying junk, falls back to the current
  defaults — the same rule every other field in the blob follows.

## Acceptance criteria

- Both gutters drag smoothly, stop at their bounds, and survive a restart.
- Tab reaches each gutter; arrows move it; the announced values are right.
- Resizing the dock reflows the terminal (its grid follows) without clearing it.
- No visible jank while dragging with a document and a terminal open.

## Tests required

- Vitest: the geometry (clamping, delta direction, keyboard steps, bounds from
  a container) as pure functions; layout blob round-trip and sanitizing.
- Manual: drag both gutters, keyboard-resize both, double-click to reset,
  restart and confirm the sizes came back.

## Commands to run

`pnpm check`, `pnpm dev`.

## Completion report

### Summary

Two draggable gutters: one between the sidebar and the editor, one on the
terminal dock's top edge. Both are real `separator` widgets — focusable, arrow
keys, `Home`/`End`, double-click to reset — and both sizes persist with the
layout blob. No protocol change.

The design decision that shaped the code: **a drag never re-renders the
shell.** The gutter writes a CSS custom property straight onto the shell
element and tells React once, on release. Re-rendering the tree on every
pointer event would re-render Monaco and every xterm with it, which is exactly
the renderer-blocking work the layout must not add.

### Files changed

- `apps/desktop/src/components/splitter-geometry.ts` (new): `clampSize`,
  `sizeFromDelta`, `sizeFromKey`, `boundsWithin` — the whole arithmetic, with
  no DOM, plus 13 tests.
- `apps/desktop/src/components/splitter.tsx` (new): the widget. Pointer
  capture, keyboard, double-click reset, announced bounds.
- `apps/desktop/src/features/session/persisted-layout.ts`: a `panes` section
  and `PANE_LIMITS`, `LAYOUT_VERSION` 3 → 4, clamping on read; two tests.
- `apps/desktop/src/App.tsx`: a ref on the shell, the two CSS variables, both
  gutters and their bounds.
- `apps/desktop/src/app.css`: the gutter styles, the sidebar grid track, the
  dock height.
- `biome.json`: `useSemanticElements` off for `splitter.tsx` (see below).

### Tests actually executed

| Command | Result |
|---|---|
| `pnpm exec vitest run` | 263 pass across 42 files (15 new) |
| `pnpm check` | fully green — every gate, including the generated-types guard |
| `cargo test --workspace` | 279 pass (unchanged; no Rust touched) |

Not executed: `pnpm dev`. Every acceptance criterion here is about dragging
something with a pointer, which the agent cannot do.

### Architectural decisions

- **The drag writes CSS, not state.** The gutter owns a `--sidebar-width` /
  `--terminal-height` custom property on the shell element during a drag and
  commits to React once on release. React also sets the same properties from
  the committed value, so the two agree; a stray re-render mid-drag snaps to
  the committed size for one frame at most.
- **Deltas, not edges.** The size follows `start + (pointer - origin)` rather
  than measuring against the pane's own edge, because that edge moves as the
  pane resizes and the gutter drifts away from the pointer over a long drag.
- **Bounds are evaluated per interaction, against the live layout**, so a
  resized window changes what a drag allows without any stored state.
- **The neighbour's minimum is enforced twice, deliberately.** `boundsWithin`
  bounds the drag; `min()` in the grid track and the dock height bound
  everything else. The CSS one is what holds when a stored size arrives from a
  larger monitor, or when the window shrinks with nothing being dragged — no
  JavaScript involved, so there is no resize handler to miss.
- **Clamp, do not discard, a stored size.** A blob from another monitor should
  open a usable sidebar, not silently reset one the user chose.
- **`useSemanticElements` is off for `splitter.tsx`**, via the same `overrides`
  mechanism the protocol transport already uses. The rule suggests `<hr>`,
  which is the non-focusable thematic break; the ARIA window-splitter pattern
  is a focusable `separator` carrying `aria-valuenow`, which `<hr>` cannot be.
  An inline `biome-ignore` does not attach to a JSX attribute diagnostic, so
  the file-scoped override is the honest form.
- **The dock's gutter only exists while the dock is open.** An invisible tab
  stop resizing a collapsed panel is worse than no gutter.

### Known limitations

- **Manual checks are owed.** Dragging, keyboard resizing, double-click reset
  and the restart round trip are all visual.
- **The gutters are invisible until hovered or focused.** Deliberate (it is how
  VS Code behaves) but it means the affordance is the cursor, which a
  touch-only user does not get.
- **Resizing the dock reflows the terminal on every grid step.** xterm's
  `onResize` fires per *cell* change, not per pixel, so a full drag is on the
  order of ten `terminal.resize` calls rather than sixty — bounded, but not
  batched.
- **The stacked layout below 60rem has no vertical gutter**; the sidebar height
  there is fixed by the existing media query.
- **Sizes are global, not per workspace.** The layout blob has always been.
- **The editor's minimums are two constants in `App.tsx` and one `22.5rem` in
  the stylesheet.** They must agree; nothing enforces that they do.

### Follow-up tasks

- A settings surface, or a menu item, to reset the layout — double-click on a
  gutter is undiscoverable on its own.
- Persist and restore the dock's open/closed state; it is React state today and
  resets on every launch.
- Keep the editor's minimum in one place once there is somewhere sensible to
  share a constant between the stylesheet and the shell.

### Remaining risks

- The horizontal gutter is positioned to straddle the dock's top border with a
  negative margin. It looks right in the markup but has never been seen; if it
  sits a few pixels off, the fix is in `.splitter--horizontal`.
- `100vh` in the dock's `min()` is the window, not the shell, so the `22rem`
  allowance absorbs the header. If the header ever grows, that number is wrong
  and the dock can crowd the editor again.
