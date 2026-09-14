# Task T120-T — A theme the user picks: system, light or dark

- **Phase:** 2
- **Status:** review
- **Depends on:** T120-C (the contract). Independent of T120-U.

## Objective

Make `workbench.theme = system | light | dark` real: restructure `tokens.css`
so an explicit choice wins over the OS in both directions, stamp `data-theme`
on the document root, and give Monaco and xterm a signal they can follow that
is not a `matchMedia` event. **Amends ADR-016.**

## Motivation

ADR-016 costed this precisely and left it undone:

> An in-app theme toggle is not free: everything currently keys off
> `prefers-color-scheme`. Adding one means a root `data-theme` attribute, a
> persisted field in `persisted-layout.ts`, and a signal the two `matchMedia`
> listeners can also watch.

Its own "Revisit when" is *"a user-selectable theme is specified"*, and
ADR-031's settings surface specifies one. The persisted field lands in
`settings.json` rather than `persisted-layout.ts` — theme is user intent, not
window state (ADR-031 §7) — which is the one part of ADR-016's forecast this
ticket changes.

This also overrides T048's non-goal ("a user-selectable theme; the OS
preference remains the only signal"). That is deliberate and is recorded in
ADR-031's consequences.

## Dependencies

`settings.get` and `SettingsChanged` from T120-C. T120-U renders the control;
this ticket makes the value do something, and the two do not share a file.

## Required reading

- `docs/decisions/ADR-016-design-tokens.md` — in full; this ticket amends it
- `apps/desktop/src/styles/tokens.css` — dark on `:root`, light in a single
  `@media (prefers-color-scheme: light)` block at the end
- `packages/editor/src/monaco/theme.ts` — `colorsFor`, `watchColorScheme`, and
  the non-hex filter that exists because Monaco throws on what it cannot parse
- `apps/desktop/src/features/terminal/terminal-view.tsx` — `readTheme()` and
  the in-place re-apply
- `docs/tasks/completed/T048-ui-revamp.md` — the non-goal being overridden

## Allowed scope

- `apps/desktop/src/styles/tokens.css`
- `apps/desktop/src/features/theme/` (new: the signal and its test)
- `packages/editor/src/monaco/theme.ts`
- `apps/desktop/src/features/terminal/terminal-view.tsx`
- `docs/decisions/ADR-016-design-tokens.md` — the amendment

## Explicit non-goals

- **Designing a light palette.** ADR-016 says the light values are derived, not
  designed, because artboard 2a is dark-only. This ticket ships what exists and
  says so; drawing a real light theme is a design task, not this one.
- Custom or user-authored themes, a theme marketplace, per-syntax token colours.
- Any settings UI (T120-U).

## Required behavior

1. **`tokens.css` restructure.** Dark stays on bare `:root`. The existing light
   block's selector becomes `:root:not([data-theme="dark"])` inside the media
   query, and a second block `:root[data-theme="light"]` carries the same values
   outside it.

   Keeping the media query is deliberate: it preserves correct **first paint**
   for system users, who are the default and the majority. Resolving the theme
   in JavaScript alone would introduce a flash of the wrong theme where none
   exists today.
2. **A duplication guard.** The restructure means the light values appear
   twice. Add a unit test that parses `tokens.css` and asserts the two light
   blocks declare identical values — cheaper and more honest than ~110 lines of
   `var(--light-*)` indirection, and it holds ADR-016's real invariant.
3. **`theme-signal.ts`.** One small subscribable owning the resolved theme:
   stamps `data-theme` on the document root, listens to `matchMedia` itself,
   applies `SettingsChanged`, sets `color-scheme` so native scrollbars and form
   controls follow, and notifies subscribers on either input.
4. **Both readers subscribe to it** instead of to `matchMedia` directly, so
   there is exactly one place that knows what the current theme is.
   `watchColorScheme` and the terminal's theme effect keep their re-apply
   behaviour; only their trigger changes.
5. `system` resolves through `prefers-color-scheme` and follows the OS live.

## Failure behavior

- Settings unavailable, or the theme value missing or unrecognised → `system`,
  which is today's behaviour. The theme never fails closed to an unreadable
  surface.
- A token missing from `tokens.css` → the existing fallback literals in
  `theme.ts` and `readTheme()` still apply; this ticket does not remove them.
- `matchMedia` unavailable → dark, and the picker still works.

## Acceptance criteria

- Light, Dark and System each apply to the window chrome, Monaco **and a live
  xterm** without a reload.
- System follows an OS appearance change live.
- An explicit Dark choice survives the OS being set to light, and vice versa.
- No flash of the wrong theme at startup for a system-preference user.
- No colour literal is added anywhere outside `tokens.css`.
- ADR-016 carries the amendment.
- `pnpm check` green.

## Tests required

- The `tokens.css` duplication guard described above.
- `theme-signal.test.ts` — resolution for all three values, that an explicit
  choice beats the media query in both directions, and that subscribers are
  notified on both inputs.
- A test that Monaco's colour map still contains only values Monaco can parse
  after the restructure — the non-hex filter exists because it throws otherwise.

## Manual checks

- Flip the OS appearance with the theme on System, with a terminal open and a
  file in Monaco; all three surfaces follow.
- Set Dark, flip the OS to light, confirm nothing changes.
- Restart on each of the three values and watch for a flash at startup.

## Commands to run

```
pnpm check
pnpm dev      # the manual checks above; a live xterm is the one that matters
```

## Completion report

*(filled on completion)*
