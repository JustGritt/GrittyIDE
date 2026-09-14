# Task SIDEBAR-ICONS — sidebar section tabs as icons, for horizontal room

- **Phase:** 2
- **Status:** doing
- **Depends on:** T100-U (the sidebar header as a real tablist)

## Why

The sidebar header is a 32px bar holding the section tabs (**FILES**,
**CHANGES**), the hidden-entries badge, **Open…** and the options disclosure.
The worded tabs and button took most of that width in a narrow sidebar. Asked
for by Alexis: draw them as icons instead.

## Allowed scope

- `apps/desktop/src/features/git/sidebar-icons.tsx` (new)
- `apps/desktop/src/features/git/sidebar-tabs.tsx` and its DOM test
- `apps/desktop/src/features/explorer/explorer-panel.tsx`
- `apps/desktop/src/app.css` — the `.explorer__tab` rule, and (widened
  2026-09-14, asked for by Alexis with a screenshot) the ⋯ options menu, which
  ran off the sidebar's left edge

## Acceptance criteria

- Each section tab draws a 16px glyph, not its label.
- The label stays the tab's accessible name (`aria-label`) and its tooltip
  (`title`), so `getByRole('tab', { name: 'Files' })` and a hover both still
  say what the glyph means.
- Tablist semantics are unchanged: roving tabindex, Arrow/Home/End, one
  `aria-controls`.
- **Open…** becomes a folder icon button named "Open folder…".
- No new dependency.

## Completion report

**Summary.** Three inline SVG glyphs (files, branch, folder) in
`sidebar-icons.tsx`, stroked with `currentColor` so the existing muted and
selected tab colours apply. `SIDEBAR_TABS` gains an `Icon` per entry; each tab
renders it with `aria-label` and `title` set to the label. The open button in
`explorer-panel.tsx` uses the folder glyph. The tab CSS drops the uppercase
label treatment for a square 24px hit target.

**The ⋯ menu overflow.** `.explorer__menu` was `position: absolute; right: 0;
width: 21rem` inside the ⋯ `<details>`, and `.explorer` has
`overflow: hidden` — so in a sidebar narrower than about 23rem the menu's left
side, including the path input and the checkbox, was cut off. The menu is now
positioned against `.explorer__toolbar` (which becomes `position: relative`),
sits `--gap-md` in from the right, and is `min(21rem, 100% - 2 × --gap-md)`
wide. A scratch page loading the real `app.css` with the explorer markup
measured it at sidebar widths 220, 360 and 600px: a 12px gutter on both sides
at 220 and 360, and 21rem pinned to the right edge at 600.

**Decisions.** Inline SVG, not an icon package: nothing else in the window
draws an icon yet, and three glyphs do not justify a dependency. If a second
surface wants icons, hoisting `sidebar-icons.tsx` into a shared module is the
follow-up, not a package.

**Tests executed.**

- `npx vitest run` over `src/features/git/`, `src/features/explorer/` and
  `shell-restore.dom.test.tsx` — 13 files, 178 tests passed. The tablist test
  now reads accessible names instead of `textContent`, and a new case asserts
  each tab has no text, an `aria-hidden` SVG, and a tooltip equal to its name.
- `npx tsc --noEmit -p apps/desktop` — clean.
- `npx biome check` on the changed files — clean.
- The glyphs were rendered at 16px and 8x in a scratch page to check them by
  eye; the files glyph was redrawn once after that check (the pages overlapped).

**Not done.** Not exercised in the running Tauri window; full `pnpm check` not
run (the working tree carries other sessions' uncommitted work).

**Follow-ups.** The Changes panel's **Refresh** button and the
Changes/History toggle are still worded; the toggle sits on its own row, so it
costs no header width.
