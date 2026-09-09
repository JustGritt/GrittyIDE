# Task T130-U — The quick-open bar, and its four modes

- **Phase:** 2
- **Status:** review
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
   is insignificant.

   **Corrected 2026-09-09** — the original text said "a second prefix character
   is part of the term (`>>foo` is the command `>foo`), so a path beginning with
   a prefix character is still reachable", and those two halves contradict each
   other: if `>>foo` stays in *commands* mode then no path starting with `>` is
   reachable, which was the stated purpose. The rule that makes the rationale
   true is the one to implement: **a doubled prefix escapes it** — `>>foo` is
   *files* mode with the term `>foo` — and only the first doubling is consumed
   (`>>>x` → files, term `>>x`), so the mode does not flicker as a run of prefix
   characters is typed.
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

### Summary — the wire-free half only

Four pure modules and their tests: the prefix parser, the row model, the keymap
and the command array. **The component, the controller, the render test, the dom
test and the mount are not built**, because all five need
`workspace.findFiles` — T130-C (the quick-open contract), which is still behind
another lane's protocol lease. This ticket stays at review as a half, and says so
rather than reading as finished.

### Files changed

All new, in `apps/desktop/src/features/quick-open/`:

| File | What |
|---|---|
| `quick-open-parser.ts` | `parseQuery`, and `QUICK_OPEN_PREFIX` as a `Record<QuickOpenMode, string>` so adding a mode is a compile error until it decides whether it has a prefix |
| `quick-open-model.ts` | `quickOpenList(state)`, the total union, seven named empty reasons, four row kinds, `parseLineTarget` |
| `quick-open-keymap.ts` | `quickOpenIntentFor(chord)` over a structural chord; `null` for unclaimed keys |
| `quick-open-commands.ts` | `quickOpenCommands(deps): readonly QuickCommand[]`, four real entries |
| `quick-open-types.ts` | The seam frozen before the parallel work started |
| four `*.test.ts` | 79 tests: parser 15, model 38, keymap 14, commands 12 |

### Tests actually executed

`pnpm exec vitest run --project desktop apps/desktop/src/features/quick-open` —
**102 passed, 5 files** (79 this ticket's, 23 T130-S's). biome clean; typecheck
clean. Whole JS gate green: biome 302 files, typecheck done, **vitest 1060
passed / 102 files**. `pnpm check` overall red at `cargo fmt --all --check` on
another lane's uncommitted settings crate.

### Architectural decisions

**No registry, as the non-goals require.** `quickOpenCommands(deps)` is a pure
function returning a typed array over a plain callback interface. No
registration API, no contribution points, nothing to plug into — which is what
keeps it clear of hard rule 9.

**Row ids are positional** (`file:0`, `command:2`), not the natural key: HTML5
forbids whitespace in an `id`, and both a path and a Markdown heading routinely
contain one, so `aria-activedescendant` would point at nothing. The natural key
stays on the row. It also means the keyboard holds its *position* as debounced
results settle, rather than chasing a path.

**Movement clamps rather than wrapping.** The acceptance criterion says "as the
explorer tree's do" and `focusAt` clamps, so reusing `moveFocus`/`focusAt`
unchanged satisfies both the reuse requirement and the criterion.

**Trailing whitespace is kept, leading whitespace stripped.** A file name may
legally end in a space, and eating it would make such a path the one thing quick
open cannot reach. Line mode trims its own term, where a space is never part of
the answer.

**A term with a query still settling draws `loading`, not "no match"** — that is
the debounce window, and "no match" would be a lie the user watches being
corrected.

**Truncation is one state with two causes** (the core's flag, and the 50-row
cap): the user's question is "is this all of them?" and the answer is the same.
A list that exactly fills the cap is `results`, not `truncated`.

### Deviations from the task text

**The ticket's escape rule contradicted itself and has been corrected in place**
(see requirement 1). "`>>foo` is the command `>foo`" and "so a path beginning
with a prefix character is still reachable" cannot both hold. Implemented the
reading that makes the rationale true — a doubled prefix escapes to files mode —
and only the first doubling is consumed, so the mode does not flicker as a run
of prefix characters is typed.

**Symbols' third sub-case folds into the second.** "Rules ran, file is empty"
and "rules ran, term matched none" share `noOutlineMatch` with different prose,
keeping exactly the two reasons the ticket demands; `noOutlineRules` stays
distinct.

### Known limitations

- `FileMatch` and `FileQueryState` are **this module's provisional seam**, marked
  as such at both declarations. No find-files type is imported from
  `@gritty/protocol` and no `core.*` call exists, deliberately: inventing the
  response shape now would be guessing at T130-C's contract.
- `matchesTerm` decides membership only, never order. T130-R (the fuzzy path
  scorer) replaces it for commands and symbols once it is reachable from the UI;
  files never pass through it.
- Home/End are claimed, costing those caret shortcuts inside the field. ⌘←/⌘→
  still reach the caret on macOS. ⇧Enter and ⌥Enter are left unclaimed rather
  than aliased.

### Follow-up tasks

| Ticket | Why |
|---|---|
| The component, controller and the two remaining test tiers | Blocked with this ticket on T130-C (the quick-open contract). |
| T130-M (mount the bar in the app bar) | Needs the component. Queued behind T101-D (the diff as an editor tab) and T120-M (mount the settings tab) for the same three files. |
| A shortcut registry | Still the follow-up four task files have now recorded. |
