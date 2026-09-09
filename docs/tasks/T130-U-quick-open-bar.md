# Task T130-U — The quick-open bar, and its four modes

- **Phase:** 2
- **Status:** todo
- **Depends on:** T130-C (the wire), T130-R (the ranking). Independent of T130-S,
  which adds one mode to a surface this ticket builds.

## Objective

The feature directory: a search field that fuzzy-finds workspace files, switches
mode on a prefix (`>` commands, `:` line, `@` symbols), and opens what you pick
as a preview tab. Everything except the mount, which is T130-M's.

## Motivation

A palette has been deferred by name five times — `T049:44` ("and therefore the
⌘K hint 3a shows"), `T080:94`, `T050:197`, `T120-U:63`, `T040:38` — while the
project plan lists it as a **Phase 1 exit criterion** (`Gritty IDE Project
Plan.md:526`). Nothing owns it, so nothing has built it.

The cost of its absence is concrete: the explorer tree is the only route to a
document, and there is no command surface at all — nine hand-gated `window`
keydown listeners, four task files apologising for the missing registry.

## Required reading

- `apps/desktop/src/features/explorer/explorer-selection.ts` — `moveFocus`,
  `focusAt`, `typeahead`: generic over `readonly string[]`, reuse rather than rewrite
- `apps/desktop/src/features/explorer/explorer-keymap.ts:15-56` — a pure
  keystroke→intent mapper, and why it returns `null` for unclaimed keys
- `apps/desktop/src/components/menu.tsx:54-103` — capture-phase outside-click,
  and the viewport-clamping `useLayoutEffect`
- `apps/desktop/src/features/workstreams/island.tsx:67-90` — Escape +
  `pointerdown`-outside, and focus-on-open
- `apps/desktop/src/features/documents/use-documents.ts:307-348` — `openFile`,
  `findTabByPath`, `previewId` and `retirePreview`
- `apps/desktop/src/features/runs/run-console-model.ts` and its three tests —
  the house three-tier test split, and the `PromptGate` union shape to copy
- `docs/tasks/T041-preview-tabs.md` — what a preview tab already guarantees
- `AGENTS.md` hard rules 1, 2, 9, 14

## Allowed scope

- `apps/desktop/src/features/quick-open/**` (new, all of it)

Nothing else. T080-U owns `apps/desktop/src/features/search/**` (the find-in-files
panel) and these must not touch each other.

## Explicit non-goals

- **No shortcut registry, and no command registry.** Hard rule 9 names "plugin
  systems" and "speculative framework layers"; a registry anything can register
  into is both, and it would change the five-module status quo
  (`open-shortcuts.ts`, `terminal-chords.ts`, `agents-section-chords.ts`,
  `permission-chords.ts`, `explorer-keymap.ts`), which is hard rule 2. Build
  `quickOpenCommands(deps): QuickCommand[]` — a pure function returning a typed
  array. A list, not a layer. The registry is a real ticket and a real ADR;
  record it as a follow-up, as `T120-U:63` and `T120-M:58` already did.
- **No mounting, and no chord.** `App.tsx`, `app.css`, `app-bar.tsx` and the
  Monaco host are T130-M's; touching them here is what the split exists to
  prevent. Export a component and its controller; let T130-M place them.
- **No `@` mode implementation** — T130-S's. Leave the parser's `symbols` arm
  returning an empty, honest "no outline" row so the mode exists and says so.
- **No editor tab.** Quick open is a transient overlay. A non-document tab
  trips ADR-026's `Revisit when`, and `SessionTab.path` is a non-nullable
  column — that is a wire change and a migration for a thing that should vanish
  on Escape.
- **No symbol provider, no LSP, no cross-workspace search, no recent-files
  ranking.** Each is its own follow-up.

## Required behavior

1. **The parser is pure and owns every prefix rule.**
   `parseQuery(raw) -> { mode: 'files'|'commands'|'line'|'symbols', term }`.
   `>` commands, `:` line, `@` symbols, anything else files. Leading whitespace
   is insignificant; a second prefix character is part of the term (`>>foo` is
   the command `>foo`), so a path beginning with a prefix character is still
   reachable.
2. **The rows are a discriminated union, total over every state**: results,
   empty-because-no-query, empty-because-no-match, truncated, error, and
   loading. Follow `run-console-model.ts`'s `PromptGate`: every state named,
   none left to a default nobody chose. The render never computes a state.
3. **Files mode calls `workspace.findFiles`, debounced and cancelled.** One
   in-flight query at a time; a new keystroke cancels the previous operation
   through `operations.cancel` rather than racing it. A late response for a
   superseded query is dropped, not rendered.
4. **`truncated` is always said out loud.** T080's contract is explicit that a
   truncated result must not read as a complete one.
5. **Enter opens a preview tab.** `openFile(workspaceId, path, { preview: true })`
   — an already-open path is found by `findTabByPath` and merely activated, and
   an edit or a double-click promotes it. ⌘Enter opens non-preview. This is what
   makes quick open a *look*, matching the explorer's single click.
6. **Commands are a typed array, and a disabled command says why.**
   `QuickCommand { id, title, group, run(), disabled?: string }` — a reason
   string, not a boolean, so the row can explain itself rather than being
   mysteriously inert.
7. **`:` jumps in the active editor.** `:120` goes to line 120, `:120:8` to a
   column. Out of range clamps to the last line rather than failing. With no
   editor open the mode says so.
8. **The keymap is pure and yields unclaimed keys.** Down/Up move, Home/End go
   to the edges, Enter accepts, Escape dismisses, everything else returns `null`
   and reaches the input. Match on `event.code` for chords, never `event.key`.
9. **Reuse the explorer's list algebra.** `moveFocus`/`focusAt` from
   `explorer-selection.ts` are generic over `readonly string[]`; movement wraps
   the way the explorer's does, and nothing focused yet means Down lands on the
   first row.
10. **A combobox, not a menu.** The input keeps focus and owns the keyboard;
    the list is addressed by `aria-activedescendant` with `role="listbox"`.
    `Menu` cannot express this — take its outside-click and viewport-clamping
    effects, not the component.
11. **Paths are untrusted text** (rule 13). Escaped, never interpolated into
    markup, and never logged.

## Failure behavior

- Core unreachable or the command failing → the error row, carrying
  `ErrorEnvelope`, with the field still usable. A failed query never closes the bar.
- No workspace open → files/symbols modes say so; commands still work.
- A cancelled query → no row change and no error; cancellation is not a failure.
- A command whose `run()` throws → the bar closes and the error surfaces where
  that command's own errors already go; quick open does not become an error sink.
- An empty result is a stated empty state, never a blank panel.

## Acceptance criteria

- Typing `epn` lists `editor-pane.tsx` first; Enter opens it as an italic
  preview tab; double-clicking the tab promotes it.
- `>` switches to commands, `:` to line, `@` to symbols, and switching mode
  mid-query does not fire a stale request.
- Escape dismisses and restores focus to where it came from.
- Arrow movement, wrap-around and Home/End behave as the explorer tree's do.
- No `@tauri-apps/api` import anywhere in the feature (Biome enforces it).
- `pnpm check` green.

## Tests required

The house three-tier split, rationed as it is elsewhere — 7 `.dom.test.tsx`
files exist against ~95 test files, so earn the third tier.

- **`*-model.test.ts` (node, pure)** — the bulk. `parseQuery` over every prefix,
  the escape case, and whitespace; the row union asserted *totally*, every state
  named; `quickOpenCommands` including a disabled command's reason; the keymap,
  including that an unclaimed key returns `null`; `:` parsing and clamping.
- **`quick-open-render.test.ts` (node, `renderToStaticMarkup`)** — markup and
  ordering via `indexOf`; the truncated notice present; **escaping of a path
  containing `<script>` and a quote**, asserted with the `tagsOf()`
  attribute-blanking scan `run-console-render.test.ts` uses; every controller
  callback a `refuse()` that throws, so rendering that reaches the core fails loudly.
- **`quick-open.dom.test.tsx` (jsdom docblock)** — only what a string cannot:
  focus on open, arrow movement changing `aria-activedescendant`, Escape
  restoring focus, Enter calling `openFile` with `{preview: true}`, and
  outside-click dismissing. **Render the bar, not `App`** — rendering `App`
  makes this the second consumer of the four jsdom shims inlined in
  `shell-restore.dom.test.tsx`, and JSDOM-VITEST says that is the moment they
  must move to a shared setup file.

## Commands to run

```
pnpm vitest run --project desktop quick-open
pnpm check
```

## Completion report

*(filled on completion)*
