# ADR-016: CSS custom properties are the single source of truth for the palette

- **Status:** Accepted
- **Date:** 2026-09-04

## Context

The Phase 0 shell had six design tokens, all colour, and three unrelated
palettes that agreed only by coincidence:

- `app.css` defined `--bg`, `--fg`, `--muted`, `--ok`, `--error`, `--border`,
  and synthesized every hover and surface with `color-mix()` at four ad-hoc
  percentages. Spacing, radii, type sizes and both font stacks were literals
  repeated across 1430 lines.
- Monaco used the stock `vs` / `vs-dark` themes, following
  `prefers-color-scheme` through its own `matchMedia` listener.
- xterm read three colours (`--bg`, `--fg`) once, at construction, and fell
  through to its own defaults for the sixteen ANSI colours. A live terminal
  never re-themed.

Nothing was wrong while all three keyed off the same OS query and the palette
was close to Monaco's own. Adopting a designed palette (artboard 2a) breaks
that coincidence: the editor surface, the terminal background and the window
chrome are now specific values that have to agree.

## Decision

`apps/desktop/src/styles/tokens.css` holds the palette, the type scale and the
geometry scale as CSS custom properties, and is the only place any of them are
written. `app.css` imports it and consumes `var(--…)`.

The two non-CSS surfaces read the computed values back off the document root
rather than keeping a copy:

- `packages/editor/src/monaco/theme.ts` defines `gritty-dark` / `gritty-light`
  with `monaco.editor.defineTheme`, inheriting `vs-dark` / `vs` token rules and
  overriding only the chrome colours from the tokens. It redefines the theme on
  every colour-scheme change, so the values are re-read rather than cached.
- `apps/desktop/src/features/terminal/terminal-view.tsx` builds the full xterm
  `ITheme` — sixteen ANSI colours, selection and cursor accent — from
  `--ansi-*` tokens, and re-applies it in place when the scheme changes.

Two values are deliberately duplicated, each with a comment saying so:

- `EDITOR_FONT_STACK` in `packages/editor/src/editor-options.ts` must match
  `--font-mono`. Monaco measures its font on a canvas, where `var()` does not
  resolve.
- The fallback literals in `theme.ts` and `readTheme()`, which only apply if a
  token is missing.

Monaco's colour keys accept hex only, so `theme.ts` filters out any token that
resolves to `oklch()`, `rgba()` or a `color-mix()` — Monaco throws on a colour
it cannot parse, and an absent key falls back to the base theme.

Light values are derived, not designed: artboard 2a is dark-only. They keep
every role and invert the surface ramp.

## Consequences

- A palette change is one file, and the editor and terminal follow.
- An in-app theme toggle is not free: everything currently keys off
  `prefers-color-scheme`. Adding one means a root `data-theme` attribute, a
  persisted field in `persisted-layout.ts`, and a signal the two `matchMedia`
  listeners can also watch.
- Tokens live in `apps/desktop` rather than a `packages/ui` (which the
  AGENTS.md repository map anticipates). Creating a package for one stylesheet
  with one consumer is the speculative layer hard rule 9 warns against; it moves
  when a second consumer appears.
- `.md-content pre` no longer hardcodes `#1e1e1e` / `#fffffe`. See ADR-013: the
  coupling to Monaco's background is real, but it now runs through
  `--surface-editor`, which is the value Monaco is given.

## Revisit when

A second package needs the tokens, or a user-selectable theme is specified.
