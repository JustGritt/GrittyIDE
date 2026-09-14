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
  listeners can also watch. **Built by T120-T — see the amendment below.** The
  forecast was right except for where the persisted field went.
- Tokens live in `apps/desktop` rather than a `packages/ui` (which the
  AGENTS.md repository map anticipates). Creating a package for one stylesheet
  with one consumer is the speculative layer hard rule 9 warns against; it moves
  when a second consumer appears.
- `.md-content pre` no longer hardcodes `#1e1e1e` / `#fffffe`. See ADR-013: the
  coupling to Monaco's background is real, but it now runs through
  `--surface-editor`, which is the value Monaco is given.

## Amendment (T120-T, 2026-09-09): a user-selectable theme

This ADR's "Revisit when" named *a user-selectable theme is specified*, and
ADR-031's settings surface specified one: `workbench.theme = system | light |
dark`. It also overrides T048's non-goal, which said *"a user-selectable theme;
the OS preference remains the only signal"*. Four things changed.

**1. The persisted field is a setting, not layout.** ADR-016 forecast a field in
`persisted-layout.ts`; it landed in `settings.json` instead. ADR-031 §7's rule
decides it: theme is user intent that should follow the person, not the state
of this window. That is the one part of the forecast this amendment reverses.

**2. Light is declared twice, and the media query stays.** Dark keeps the bare
`:root`. Light appears in a media query narrowed to
`:root:not([data-theme="dark"])`, and again outside it as
`:root[data-theme="light"]`. The narrowing is what lets an explicit Dark choice
beat an operating system asking for light; the second block is what lets an
explicit Light choice beat one asking for dark.

Keeping the media query at all is deliberate. It preserves correct **first
paint** for system users, who are the default and the majority: resolving the
theme in JavaScript alone would introduce a flash of the wrong theme where none
exists today. The cost is a third duplicated value in this ADR's list, and it is
held by a test — `features/theme/tokens-duplication.test.ts` parses the
stylesheet and fails if the two light blocks stop agreeing. The alternative,
~110 lines of `var(--light-*)` indirection to make duplication impossible, buys
less than it costs and obscures the palette it is protecting.

**3. There is now one answer to "what is the theme".**
`features/theme/theme-signal.ts` owns it: it resolves the choice against
`prefers-color-scheme`, stamps `data-theme` on the document root, sets
`color-scheme` so native scrollbars and form controls follow an explicit
choice, and notifies subscribers — but only when the *resolved* theme actually
moves, since `system → dark` on an already-dark OS shifts no token.

Both readers this ADR describes now subscribe to it instead of to `matchMedia`
directly, which is the substantive change: an explicit choice is precisely what
a media query cannot see, so either of them watching one would have left that
surface on the OS's appearance while the rest of the window followed the
picker. `watchColorScheme` gained a `ColorSchemeSource` parameter for this, and
`packages/editor` still defaults to the media query so it stays usable without
the desktop app's signal.

**4. Light is still derived, not designed.** This amendment ships the ramp that
already existed and makes it selectable. Artboard 2a is dark-only, so drawing a
real light theme remains a design task and is not this one.

## Revisit when

A second package needs the tokens, or someone wants a theme Gritty does not
ship — a custom palette, per-syntax token colours, or a theme read from a file.
That is where the "one stylesheet, two blocks" shape stops paying: it holds two
themes well and would not hold *n*.
