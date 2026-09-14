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

### Summary

`workbench.theme = system | light | dark` is real. `tokens.css` declares the
light ramp twice so an explicit choice wins over the operating system in both
directions; `features/theme/theme-signal.ts` resolves the choice, stamps
`data-theme`, sets `color-scheme`, and is now the single thing Monaco and a live
xterm follow instead of watching `matchMedia` apiece. ADR-016 carries the
amendment.

The control that sets the value is T120-U's — it renders `workbench.theme` as an
ordinary enum row — so the two tickets did not share a file, as planned.

### Files changed

| File | What |
|---|---|
| `apps/desktop/src/styles/tokens.css` | dark on bare `:root`; light in `@media … { :root:not([data-theme="dark"]) }` and again in `:root[data-theme="light"]` |
| `features/theme/theme-signal.ts` (new) | the signal: resolution, the root stamp, `color-scheme`, subscribers |
| `features/theme/theme-settings.ts` (new) | `workbench.theme` → the signal, over `settings.get` + `SettingsChanged` |
| `features/theme/theme-signal.test.ts` (new) | 17 tests |
| `features/theme/tokens-duplication.test.ts` (new) | 7 tests: the two light blocks, and what Monaco is handed |
| `packages/editor/src/monaco/theme.ts` | `watchColorScheme` takes a `ColorSchemeSource`; `mediaColorScheme()` is the standalone default |
| `features/terminal/terminal-view.tsx` | the palette effect subscribes to the signal |
| `features/editor/use-editor-binding.ts` | **outside the listed scope**, 2 lines — see below |
| `apps/desktop/vitest.config.ts` | **outside the listed scope**, `css: { include: [/tokens\.css/] }` — see below |
| `docs/decisions/ADR-016-design-tokens.md` | the amendment |

### Tests actually executed

- `npx vitest run src/features/theme/` → **24 passed** (2 files).
- `pnpm check` → **exit 0**: Biome, tsc, 1233 vitest tests across 115 files,
  `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`,
  generated types fresh.
- **The duplication guard was mutation-tested**, because a guard that parses a
  file can pass by finding nothing. Changing `--surface-base` by one digit in
  one of the two light blocks fails it; so does changing `--fg` by one digit;
  so does turning `--fg-dim` into an `oklch()` (the Monaco half). All three pass
  again once reverted. An earlier version of this test *did* silently pass on an
  empty string — see the vitest note below.

### Architectural decisions

1. **The media query stays, narrowed.** Resolving the theme in JavaScript alone
   would introduce a flash of the wrong theme at startup where none exists
   today, for system users, who are the default and the majority. The cost is
   the light ramp declared twice; the guard is what makes that safe.
2. **Subscribers hear about a change in the *resolved* theme, not in either
   input.** `system → dark` while the OS is already dark moves no token, and
   both consumers respond by re-reading the whole palette and re-theming a live
   surface. The two light blocks being identical is what makes "resolved is the
   same" a sound proxy for "the palette is the same" — the same test guards
   both.
3. **The stamp happens before the notification.** A subscriber re-reads computed
   tokens, so the attribute has to be on the root by the time it looks;
   otherwise both readers re-apply the palette they were already showing. Pinned
   by a test.
4. **`color-scheme` is pinned only for an explicit choice.** On `system` the
   inline property is removed and the stylesheet's `dark light` applies again,
   which is what "let the platform decide" actually means for a native scrollbar.
5. **`packages/editor` still defaults to `matchMedia`.** `watchColorScheme` took
   a `ColorSchemeSource` parameter rather than an import of the app's signal:
   the editor package must not learn that settings exist. The default keeps it
   usable standalone and is explicitly *not* what the desktop app passes.

### Scope notes — two files outside the allowed list

Both are recorded here rather than quietly included.

1. **`features/editor/use-editor-binding.ts`, 2 lines** (an import, and passing
   the signal into `watchColorScheme`). Required behaviour 4 says both readers
   subscribe to the signal, and `watchColorScheme`'s only call site is here: a
   function's contract cannot be changed without touching its caller. The
   alternative considered and rejected was having `packages/editor` discover the
   theme through a `MutationObserver` on the root — in scope, but it would add a
   second thing that knows what the theme is, which is the exact opposite of
   this ticket's point.
2. **`apps/desktop/vitest.config.ts`, one option.** Required behaviour 2 asks
   for a test that *parses* `tokens.css`, and `apps/desktop` is a browser
   project: its tsconfig carries `vite/client`, not `@types/node`, so
   `node:fs` does not typecheck, and Vitest stubs CSS imports to an empty string
   so `?raw` returns nothing. The alternative was adding `@types/node` to
   `apps/desktop/tsconfig.json`, which would put Node's globals in reach of
   production WebView code — a real boundary, for a test-only need. This is
   test-only and scoped to one filename.

   A trap worth knowing: the pattern is matched against the module id, which
   carries the `?raw` query, so an anchored `/tokens\.css$/` matches nothing and
   the guard goes back to passing on an empty string. It is unanchored, with a
   comment saying why.

### Known limitations

- **Light is still derived, not designed.** Artboard 2a is dark-only; this ships
  the ramp that already existed and makes it selectable, as the ticket's
  non-goals require.
- The two light blocks are a real duplication. It is held by a test rather than
  made impossible by `var(--light-*)` indirection, which is the trade ADR-016's
  amendment records.
- `mediaColorScheme()` is now dead code inside the app — it survives as
  `packages/editor`'s default for a host without a theme signal, and there is no
  such host today.

### Follow-up tasks

1. **A design pass on the light theme.** Its values are inverted roles, not a
   drawn palette, and it is now something a user can choose deliberately rather
   than something they arrive at through their OS.
2. `packages/editor/src/monaco/index.ts` does not re-export `ColorSchemeSource`.
   The call site typechecks structurally; exporting the type would be tidier and
   is one line in a file outside this ticket's scope.
3. A theme Gritty does not ship (custom palette, per-syntax colours, a theme
   read from a file) is out of scope and is now ADR-016's "Revisit when".

### Remaining risks

- **The manual checks in this task file were NOT run**, and this is the main
  gap. They need a running window and eyes on it — flip the OS appearance with a
  terminal open and a file in Monaco; set Dark and flip the OS to light; restart
  on each of the three values and watch for a flash. I have no display access,
  and more to the point **there is no control to change the value from yet**:
  the picker is a row on T120-U's page, which is unmounted until T120-M. The
  right moment for this pass is immediately after T120-M lands, and it is the
  one thing standing between this ticket and "verified in the running app".
- What *is* verified is the resolution matrix (all three choices against both OS
  states, in both directions), the stamp, the notification rule, the ordering,
  and that the stylesheet's two light blocks agree. What is **not** verified is
  that the CSS cascade behaves as intended in a real browser engine. The
  specificity argument is straightforward — both light blocks are (0,2,0) and
  beat the bare `:root` at (0,1,0), and `:not([data-theme="dark"])` excludes the
  explicit-dark case — but an argument is not a screenshot.
- Nothing else in the tree keys off `prefers-color-scheme` any more: the only
  remaining uses are `tokens.css`'s intended media query and
  `packages/editor`'s standalone default. Checked by grep over sources.
