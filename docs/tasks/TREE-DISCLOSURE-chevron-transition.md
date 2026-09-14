# Task TREE-DISCLOSURE — the tree's chevron and its expand/collapse transition

- **Phase:** 2
- **Status:** review
- **Depends on:** FILE-ICONS (file-type icons in the explorer tree and Changes
  list, `d3cba8a`), which introduced the SVG chevron

## Why

Alexis asked, after FILE-ICONS landed, to "refine the chevron/collapsible
transition" of the explorer tree. **What "refine" means is not yet specified.**
Ask before building: which part feels wrong (the chevron's turn, children
popping in, collapse), and what it should feel like. The reference Alexis gave
for the sidebar is their current editor, Cursor with the Seti icon theme.

## What exists today (verified at `d3cba8a`)

- **The chevron.** `ChevronIcon` in `apps/desktop/src/components/icons.tsx`, a
  16px SVG pointing right. In `explorer-tree.tsx` an expandable row renders
  `<button class="tree__toggle[ tree__toggle--open]">` around it. A file row
  renders `<span class="tree__glyph">` with its `FileIcon` in the same 16px
  column instead.
- **The only motion.** `app.css`:
  `.tree__toggle .glyph { transition: transform 80ms ease-out }` and
  `.tree__toggle--open .glyph { transform: rotate(90deg) }`.
- **Children appear and vanish instantly.** Nothing animates rows.
- **No motion tokens.** `tokens.css` and `app.css` have no `--motion`,
  `--ease` or `--duration` tokens and no `prefers-reduced-motion` rule
  anywhere. Adding motion tokens is an ADR-016 question.

## Constraints any row animation has to survive

- **Rows are one flat list, not nested DOM.** `selectRows(state)` in
  `explorer-reducer.ts` walks the expanded nodes and returns
  `ExplorerRow[]` with a `depth`; `explorer-tree.tsx` renders
  `rows.map(...)` as siblings. There is no per-folder container whose height
  could be animated.
- **Expand is lazy.** The first expand of an unlisted folder pushes a `status`
  row (`Listing…`, or `Listing (operation)…` with Cancel) before its children
  arrive from `workspace.list`. A listing can fail (Retry row) or be truncated
  (`Load N more…` row).
- **Collapse removes rows at once.** An exit animation would need rows kept
  mounted after the reducer dropped them.
- **Folders can be huge.** Do not animate thousands of rows (AGENTS.md rules 8
  and 10: no renderer-blocking work). Cap what animates, or skip past a size.
- **Keyboard and a11y.** ArrowRight/ArrowLeft expand/collapse via
  `explorer-keymap.ts`. The treeitem carries `aria-expanded`, and focus is a
  roving `tabIndex`. The row's own click toggles too; the toggle button stops
  propagation.
- Respect `prefers-reduced-motion`.

## Allowed scope (proposed; widen deliberately and record it)

- `apps/desktop/src/features/explorer/explorer-tree.tsx` and its DOM test
- `apps/desktop/src/components/icons.tsx` (the chevron)
- `apps/desktop/src/app.css`: the `.tree__toggle` / `.tree__glyph` rules.
  This is a shared file, so claim it only for the minute you edit it.
- `apps/desktop/src/styles/tokens.css` only if motion tokens are agreed

## Verification notes

- The agent has no screen access to the Tauri window. A transition has to be
  seen by Alexis in `pnpm dev`.
- Static appearance can be checked without a screen: server-render the real
  components through Vite (`createServer` + `ssrLoadModule`, with `react` and
  `react-dom/server` loaded through `createRequire`), write HTML that links a
  copy of `app.css` and `styles/tokens.css`, then screenshot it with the
  chrome-devtools MCP. This cannot show motion.

## What Alexis asked for (2026-09-14)

Asked before building. Alexis picked all four parts: the chevron's turn,
children appearing, children leaving, and the chevron's look. For the feel,
they picked "quick and subtle": a turn of about 120ms, rows fading in, capped,
and off under reduced motion. For the look, the one problem is that the chevron
is **too big**.

One thing for reference: VS Code's tree CSS (`monaco-tl-twistie`), which Cursor
forks, has no transition on the chevron and no row motion. Its only tree
animation is a 100ms fade on the indent guides. So this goes beyond the
reference on purpose.

"Quick and subtle" as first offered kept collapse instant, but Alexis also
ticked "children leaving". Collapse therefore folds: the removed rows shrink to
zero height while fading. A fade alone would hold their space for 120ms and then
drop the rows below all at once, which is worse than no animation.

## Completion report

**Summary.** The chevron is smaller: a 3×6 path instead of 4.5×9, centred on
(8, 8), so `rotate()` doesn't wobble it. It turns in 120ms on
`cubic-bezier(0.2, 0, 0, 1)` instead of 80ms `ease-out`. When a folder opens,
its new rows fade in and drop 3px into place over 120ms. When a folder closes,
the rows it hid remain as an inert copy that folds to zero height over 120ms,
so the rows below slide up. Each side animates at most 50 rows. The OS
reduced-motion setting turns off all three, and so does a WebView without Web
Animations.

**Files changed.**
- `apps/desktop/src/features/explorer/use-row-motion.ts` (new): `planRowMotion`
  is a pure function that compares two renders of `selectRows` and returns the
  rows to fade in and the collapsed blocks to fold. The file also holds
  `useRowMotion`, which keeps the plan as state derived from the previous rows,
  plus `motionAllowed` and `animateRowIn`.
- `apps/desktop/src/features/explorer/use-row-motion.test.ts` (new): 9 tests
  for the planner.
- `apps/desktop/src/features/explorer/explorer-tree.tsx`:
  - Rows render inside a keyed `Fragment`.
  - `TreeRow` takes `enter` and attaches `animateRowIn` as a callback ref.
  - New `ExitingRows` renders the folding copy: `aria-hidden`, `inert`, with no
    roles, `data-path` or handlers, so focus and selection can never land on a
    row that is gone.
  - New `GhostRow` renders the same markup without behaviour.
  - Indentation is now computed by `indentOf`.
- `apps/desktop/src/features/explorer/explorer-tree.dom.test.tsx`: four tests
  with `animate` stubbed: children fade in; collapse folds and then removes the
  copy; reopening mid-fold draws the children once; reduced motion does
  nothing.
- `apps/desktop/src/components/icons.tsx`: the smaller `ChevronIcon`. Nothing
  else uses it.
- `apps/desktop/src/app.css`: one hunk, the `.tree__toggle .glyph` transition,
  `.tree__exit { overflow: hidden }`, and a `prefers-reduced-motion` rule.

**Scope.** `use-row-motion.*` is a new module inside the explorer feature. It
isn't in the proposed scope above, but it was claimed with the ticket. No motion
tokens were added: `tokens.css` is untouched.

**Tests actually executed.**
- `npx vitest run` in `apps/desktop`: 90 files, 1088 tests passed.
- `npx tsc --noEmit -p .` in `apps/desktop`: clean.
- `npx biome check` on the five TS/TSX files and on `app.css`: clean.
- Mutations, each restored afterwards:
  - Planning no exits failed 5 tests.
  - Keeping a fold after its folder reopened failed the reopen test.
- The DOM tests caught a real bug before commit. A workspace's first listing
  shows the root's `Listing…` row, so "the previous render was empty" was never
  true, and the whole tree faded in on open. Planning now requires an entry row
  in the previous render, and a unit test pins that.
- Static render: the real `ChevronIcon` and `FileIcon` beside the `d3cba8a`
  chevron, rendered through Vite SSR with the real `app.css` and screenshotted
  in dark and light. The new arrow is centred in the icon column and still
  reads clearly.
- **`pnpm check` not run in full.** Other sessions' uncommitted edits are in
  this tree.

**Architectural decisions.**
- Web Animations, not CSS classes, for row motion:
  - An entering row animates once as it attaches, so a later re-render cannot
    cut the fade short. `rows` changes on every selection change.
  - The fold measures its real height.
- The copy is anchored after the folder row, and a block is dropped as soon as
  that row is gone or open again.
- Durations and the curve live in `use-row-motion.ts`, and the chevron's CSS
  repeats them. Tokens wait for a second surface that wants motion (ADR-016).

**Known limitations.**
- **Nobody has seen the motion.** The agent has no screen. Judge it in
  `pnpm dev`: the 120ms durations, the 3px drop, and the fold curve are all
  first guesses.
- Every new row fades in, not only a folder's children: a page from
  `Load more`, a file the watcher adds, a renamed entry, and dotfiles shown
  again. That's deliberate, since each is a row arriving, but say if the
  rename fade reads as noise.
- Rows removed for any reason other than a collapse vanish at once, as before.
- The copy of a `Listing…` or error row omits the Cancel/Retry button and the
  error code. It is only visible for 120ms.

**Follow-ups.**
- Motion tokens in ADR-016, if a second surface animates.
- The backlog from the FILE-ICONS reference screenshot remains unticketed:
  - icons on editor tabs
  - dimmed ignored entries
  - git-status colours in the tree
  - a UI font in the tree
  - an icon for the Changes panel's Refresh button

**Remaining risks.**
- WKWebView's handling of a `height` animation on a block of rows inside an
  `overflow: auto` tree has not been observed.
- A very fast collapse-expand-collapse creates and drops blocks correctly in
  the tests, but not under a real frame budget.
