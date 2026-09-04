# Task T048 — Shell UI revamp (design artboard 2a)

- **Phase:** 0
- **Status:** review
- **Depends on:** T020, T030, T040, T046, T050, T060, T090

## Objective

Replace the Phase 0 shell's debug-harness appearance with the visual language of
design artboard **2a "Cockpit, refined"** (Claude Design project `4a56ac43`),
and give the app one real design-token layer that CSS, Monaco and xterm all
read from.

## Dependencies

Every region being restyled already exists: explorer (T020), documents and the
status bar (T030), Monaco and the tab strip (T040), splitters (T046), markdown
preview (T050), persisted layout (T060), terminal dock (T090).

## Required reading

- `docs/decisions/ADR-016-design-tokens.md` (written by this task)
- `docs/decisions/ADR-003-ui-core-boundary.md`, `ADR-012-monaco-editor-binding.md`,
  `ADR-013-markdown-preview.md`
- Design project `4a56ac43`: `Gritty Agent Layout.dc.html` (artboard `2a`) and
  `Gritty Current Shell.dc.html` (the before)

## Allowed scope

`apps/desktop/src/**`, `packages/editor/src/editor-options.ts`,
`packages/editor/src/monaco/{theme,editor-host}.ts`, `docs/`.

## Explicit non-goals

- The agent rail, agent chips, attention cards and file-lock list that artboard
  2a also shows. Those are T200–T410 and are **not** built here.
- A user-selectable theme. The OS preference remains the only signal.
- A styling framework, a component library, or a `packages/ui`.
- Any new protocol command or event.

## Required behavior

1. Design tokens in `apps/desktop/src/styles/tokens.css`, imported by `app.css`:
   a seven-step surface ramp, five text weights, accent/ok/warn/error roles, a
   type scale named by role, a geometry scale, and the sixteen ANSI colours.
2. Edge-to-edge chrome: no page padding, no floating bordered cards. Depth comes
   from the surface ramp; the hairline marks real boundaries only.
3. One 44px top bar carrying identity, the open workspace, and core health as a
   dot with an anchored popover.
4. A 26px app status bar at the bottom of the window, below the terminal dock,
   carrying build facts on the left and the active document on the right.
5. Borderless controls: a translucent ghost fill, no outlines, `--primary` /
   `--danger` / `--bare` modifiers.
6. Pill tabs, shared by the editor and the terminal dock. The markdown view
   switch becomes a segmented control inside the tab row.
7. One notice shape — tinted wash, hairline top and bottom, inline actions —
   used by the editor banners, session notices, the drop notice and the
   explorer notice.
8. Monaco and xterm read the palette from the computed CSS custom properties and
   re-read it when the colour scheme changes.

## Failure behavior

- A token that resolves to a non-hex colour (`oklch`, `rgba`, `color-mix`) is
  filtered out of Monaco's theme rather than passed through: Monaco throws on a
  colour it cannot parse, and an absent key falls back to the base theme.
- A missing token falls back to a literal at every read site.
- The core being unreachable is a first-class state: the top-bar dot goes red
  and the status bar's build slot goes red, rather than a green line that lies.
- The sidebar clips and its tree scrolls; stacked (under 60rem) the whole
  sidebar row scrolls instead, so a notice above the tree is never cut off.

## Acceptance criteria

- `pnpm check` green.
- The shell renders edge-to-edge with no bordered cards.
- Light and dark both legible, with no dark-on-dark or light-on-light leftovers.
- Monaco's background matches `--surface-editor`; its font matches the tree's.
- A live terminal re-themes when the OS colour scheme changes.
- Dragging either gutter still writes CSS variables directly and does not
  re-render the shell.

## Tests required

The existing suite is pure logic; no test renders a component, so this task
adds none and relies on `tsc` plus the manual checks below.

## Commands to run

```
pnpm format
pnpm check
pnpm dev
```

## Completion report

**Summary.** Applied artboard 2a's visual language to every existing region and
introduced `styles/tokens.css` as the single source of truth for the palette,
type and geometry (ADR-016). Structural changes: the header became a 44px app
bar; the status bar moved out of the editor pane to the bottom of the window;
the markdown view switch moved into the tab row as a segmented control; the
explorer's wrapping toolbar became a 32px header plus a popover. The agent rail
was deliberately not built.

**Files changed.**

- new: `apps/desktop/src/styles/tokens.css`,
  `apps/desktop/src/features/shell/app-bar.tsx`,
  `apps/desktop/src/features/editor/use-close-flow.ts`,
  `docs/decisions/ADR-016-design-tokens.md`, this file
- `apps/desktop/src/app.css` — every section retokenized; `.shell`,
  `.shell__body`, `.explorer`, `.editor__bar`, `.tabs`, `.statusbar`,
  `.terminal-*` restructured
- `apps/desktop/src/App.tsx` — renders `AppBar`, `StatusBar` and `CloseDialog`;
  holds the workspace for the top bar and the close flow
- `apps/desktop/src/features/editor/editor-pane.tsx` — lost the pane title, the
  status bar and the close dialog; takes `onRequestClose`
- `apps/desktop/src/features/editor/status-bar.tsx` — app-wide, always present,
  takes `health`
- `apps/desktop/src/features/explorer/explorer-panel.tsx` — 32px header,
  options popover, reports the workspace up via `onWorkspaceChange`
- `apps/desktop/src/features/markdown/markdown-toolbar.tsx` — segmented control
- `apps/desktop/src/features/terminal/terminal-view.tsx` — full ANSI theme,
  re-applied on colour-scheme change
- `apps/desktop/src/features/session/persisted-layout.ts` — sidebar default
  320 → 244
- `packages/editor/src/monaco/theme.ts` — `gritty-dark` / `gritty-light` built
  from the tokens
- `packages/editor/src/monaco/editor-host.ts` — `defineEditorTheme`
- `packages/editor/src/editor-options.ts` — `fontFamily`, `lineHeight`

**Tests actually executed.**

- `pnpm check` — green (Biome, tsc across 4 projects, vitest, cargo
  fmt/clippy/test, generated-types freshness).
- `npx vite build` — bundles; the `@import` resolves and the tokens land in the
  emitted CSS.
- Rendered at 1400×900 and 800×600, dark and light, in Chrome against a vite
  dev server. Console clean apart from the expected Tauri-IPC failures (no
  `invoke` outside the shell) — in particular Monaco defined and applied its
  theme without throwing.

**Two defects were found this way and fixed:** the sidebar's children
overlapped when a notice appeared above the tree, and the stacked breakpoint
clipped the sidebar instead of scrolling it.

**Architectural decisions.** ADR-016. Tokens stay in `apps/desktop` rather than
a new `packages/ui`. `EDITOR_FONT_STACK` duplicates `--font-mono` because Monaco
measures on a canvas where `var()` does not resolve.

**Known limitations.**

- **Light values are derived, not designed** — 2a is dark-only. They need a
  human eye before they ship.
- Not yet exercised inside the running Tauri shell with a real workspace, a
  real Monaco document or a live PTY. The browser run covers rendering and the
  theme code path, not the core-connected states.
- The `⋯` explorer popover and the health popover are `<details>` elements:
  they do not close on outside click or Escape.
- No component tests exist, so nothing guards these regions against a visual
  regression.

**Follow-up tasks.**

- Manual pass in the Tauri shell (see below); the light ramp is the first thing
  to look at.
- A dismiss-on-outside-click behaviour for the `<details>` popovers.
- Re-measure §C of the Phase 0 spike: perceived latency was measured against the
  old UI.
- The agent rail when T200/T210 land — the three-column grid is shaped for it.
