# Task T130-V — The quick-open component, its controller, and the two test tiers

- **Phase:** 2
- **Status:** review
- **Depends on:** T130-C (the wire), T130-U (the pure modules), T130-S (the outline)

## Objective

The half of T130-U that could not be built: a React bar that renders
`quickOpenList(state)`, and the controller that assembles that state from
`workspace.findFiles`, the active document and the command array. Everything
except the mount, which is T130-M's.

## Motivation

T130-U delivered its wire-free half — parser, row model, keymap, command array,
79 tests — and stopped, because the component, the controller and the two
remaining test tiers all need a files mode, and `workspace.findFiles` did not
exist. It has existed since `bdd2018`. This ticket is the rest of T130-U, split
out rather than reopened: T130-U is sitting at review having said plainly what
it left undone, and the same split is what T120-M was to T120-U.

The pure modules are total and tested. What is missing is the two things a pure
module cannot be: a React tree, and a thing that talks to the core.

## Required reading

- `apps/desktop/src/features/quick-open/quick-open-model.ts` — `quickOpenList`,
  `rowsOf`, `rowIds`, and the two declarations marked *provisional until T130-C
  lands*, which it now has
- `docs/tasks/T130-U-quick-open-bar.md` — requirements 2, 3, 4, 5, 7, 10, 11 and
  the three test tiers, which are this ticket's, unchanged
- `apps/desktop/src/features/explorer/explorer-selection.ts` — `moveFocus` and
  `focusAt`, generic over `readonly string[]`; requirement 9's reuse
- `apps/desktop/src/features/shell/menu.tsx:54-103` — the outside-click and
  viewport-clamping effects to lift, not the component
- `apps/desktop/src/features/runs/run-console-render.test.ts:26` — `tagsOf()`,
  the attribute-blanking scan the XSS assertion uses
- `apps/desktop/src/features/explorer/explorer-tree.dom.test.tsx:30-60` — the
  `vi.hoisted` transport seam, since `createFakeTransport` is not reachable from
  `apps/desktop` (the package publishes only `.`)
- `packages/protocol/src/generated/FoundFile.ts` — and specifically that
  `ranges` are **char** indices

## Allowed scope

- `apps/desktop/src/features/quick-open/**`
- `docs/tasks/T130-V-quick-open-component.md`

Nothing else. `App.tsx`, `app.css`, `app-bar.tsx` and the Monaco host are
T130-M's, and touching them here is what the split exists to prevent.

## Explicit non-goals

- **No mount and no chord.** Export a component and a controller; let T130-M
  place them.
- **No shortcut registry and no command registry.** The fifth task file to
  record this follow-up.
- **No new abstraction over the typed client.** The controller calls
  `core.workspace.findFiles` directly and the dom test swaps the module
  singleton, which is what every other feature in `apps/desktop` does. An
  injected port was considered while T130-C was blocked and is not needed now
  that it is not.
- **No recent-files ranking, no cross-workspace search, no LSP symbols.** Each
  is its own follow-up.

## Required behavior

1. **The component computes nothing.** It renders a `QuickOpenList` and the
   controller's callbacks. Every state the model names is drawn: `loading`,
   `results`, `truncated` with its notice, `empty` with its reason's message,
   and `error`.
2. **A combobox, not a menu.** The input keeps focus and owns the keyboard; the
   list is `role="listbox"` addressed by `aria-activedescendant`. The model's
   positional row ids (`file:0`) exist for exactly this.
3. **Highlighting slices by code point.** `FoundFile.ranges` are char indices
   and JavaScript string indices are UTF-16 code units, so
   `String.prototype.slice` is wrong for any path past the BMP. The split lives
   in the model as a pure function, so it is a node test rather than a render
   test.
4. **One files query in flight.** Debounced; a new query supersedes the previous
   one through `operations.cancel`, and a late response for a superseded query
   is dropped rather than rendered.
5. **A cancelled query changes nothing** — not the rows, not an error row.
   Cancellation is not a failure.
6. **The pending arm is handled from the first line.** On any real repository it
   is the normal answer. `ready` and the operation's `OperationCompleted.result`
   carry the same `FoundFiles`, so they converge on one mapper.
7. **Reuse the explorer's list algebra.** `moveFocus` / `focusAt` over
   `rowIds(list)`.
8. **Paths are untrusted text** (rule 13). Escaped, never interpolated into
   markup, never logged.

## Failure behavior

- The command failing → the error row, carrying the envelope, with the field
  still usable. A failed query never closes the bar.
- No workspace open → files and symbols say so; commands still work.
- A command whose `run()` throws → the bar closes and the error surfaces where
  that command's own errors go. Quick open does not become an error sink.
- An empty result is a stated empty state, never a blank panel.

## Acceptance criteria

- Typing `epn` lists `editor-pane.tsx` first and Enter opens it as a preview tab.
- `>` `:` `@` switch mode, and switching mid-query fires no stale request.
- Escape dismisses and restores focus to where it came from.
- A path containing `<script>` and a quote renders as text.
- No `@tauri-apps/api` import anywhere in the feature (Biome enforces it).
- `pnpm check` green.

## Tests required

- **`quick-open-model.test.ts`** extended: the highlight split, including a path
  with an astral character.
- **`quick-open-render.test.ts`** (node, `renderToStaticMarkup`): every list
  arm; ordering via `indexOf`; the truncated notice present; the XSS assertion
  via `tagsOf()`; every controller callback a `refuse()` that throws.
- **`quick-open.dom.test.tsx`** (jsdom): focus on open, arrows moving
  `aria-activedescendant`, Escape restoring focus, Enter calling `openFile`
  with `{preview: true}`, outside-click dismissing. **Render the bar, not
  `App`.**
- **`use-quick-open.dom.test.tsx`**: the controller's own behaviour — debounce,
  a superseded query dropped, a cancelled query leaving rows alone, the pending
  arm completing through `OperationCompleted`.

## Commands to run

```
pnpm vitest run --project desktop quick-open
pnpm check
```

## Completion report

### Summary

The bar and its controller, and the two test tiers T130-U deferred. The feature
directory goes from 102 tests to 142. The bar is now complete except for being
placed on a screen, which is T130-M's.

### Files changed

| File | What |
|---|---|
| `quick-open-bar.tsx` | New. The combobox, all five list arms, highlighting, outside-click, focus restoration. |
| `use-quick-open.ts` | New. The controller, and `findFilesAwaitingOperation` — the two response arms converging on one payload. |
| `quick-open-render.test.ts` | New. 14 tests over `renderToStaticMarkup`. |
| `quick-open.dom.test.tsx` | New. 10 tests: focus, arrows, Enter, pointer, dismissal. |
| `use-quick-open.dom.test.tsx` | New. 11 tests: debounce, supersession, cancellation, the pending arm. |
| `quick-open-model.ts` | `ranges` on `FileMatch` and the `file` row; `highlightSegments`; the header's provisional note replaced. |
| `quick-open-model.test.ts` | 5 highlight tests, and the fixture grew `ranges`. |

### Tests actually executed

`pnpm exec vitest run --project desktop apps/desktop/src/features/quick-open` —
**142 passed, 8 files**.

### Architectural decisions

**No port over the typed client.** While T130-C was blocked the plan was an
injected `findFiles`, so the UI could be built against a shape rather than a
command. The command landed first, so the controller calls
`core.workspace.findFiles` and the dom tests swap the module singleton — the
seam `explorer-tree.dom.test.tsx` and five other features already use. The port
would have been a second way to do one thing, justified only by a block that no
longer existed.

**`highlightSegments` is in the model, not the component.** It is where the
char-index hazard lives, and a hazard that has to be *reviewed* in JSX rather
than *asserted* in a node test is one that comes back. The astral-plane case is
a test with the counter-example written next to it.

**The pending arm buffers terminal events, and that closes a real race.** A walk
that finishes a millisecond after its budget emits `OperationCompleted` while
the `pending` response is still crossing the IPC — so a subscription opened
*after* the response resolves can miss it, and the bar sits in `loading` for
ever. ADR-009 orders a command against listener registration, not against events
that follow the response. Terminal events for `workspace.findFiles` are
therefore recorded from module load into a bounded map (16 entries, rule 8) and
checked before waiting. `does not hang when the operation completed before
anyone was listening` fails without it.

**`tabIndex={-1}` on the options.** Biome asked for the role to be focusable;
`0` would be wrong, because in a combobox the input keeps focus and the option
is only ever named by `aria-activedescendant`. Tabbing through fifty results to
reach whatever is after the bar is the failure `-1` avoids.

**Focus restoration is the component's, not the mount's.** The bar steals the
caret from a Monaco buffer, so it gives it back — on unmount rather than in the
Escape handler, which covers all four ways it closes (Escape, an accepted row, a
command that ran, an outside click). It restores only when the focus is still
inside the bar or was orphaned to `<body>`; something that deliberately took the
focus keeps it.

**`moveFocus`/`focusAt` reused unchanged.** They already clamp rather than wrap,
and already land Down on the first row when nothing is focused — both of which
§9 asks for by name. A one-row `Selection` is the whole adaptation.

### Two defects the dom tier caught that nothing else would have

- **The focus-restore guard was too strict.** It only restored when the focus
  was still inside the bar — but removing the focused input drops the focus to
  `<body>`, so on unmount it always bailed and the caret was lost. Fixed by
  treating an orphaned focus as ours.
- **A test of mine conflated two dismissals.** "A click inside does not dismiss"
  failed on a row, correctly: clicking a row *accepts* it, and accepting closes
  the bar. The outside-click listener was right; the assertion was not.

### Known limitations

- **Nothing mounts it, so nobody has seen it.** T130-M places it, adds the
  `.quick-open*` block to `app.css` and the ⌘P chord. Until then the class names
  here resolve to nothing and the acceptance criteria that need a screen are
  unmet — stated rather than implied, per rule 12.
- **The outline is recomputed whenever the `document` prop's identity changes.**
  `outlineOf` is bounded but not free (a 1 MB markdown file is 7 ms of block
  parsing), so the mount must pass a stable object and not a fresh literal per
  render. Worth a memo on T130-M's side; nothing here can enforce it.
- **`ranges` are trusted to be ascending and non-overlapping**, which the
  contract guarantees. `highlightSegments` clamps rather than validating, so a
  contract violation degrades to a wrong-looking highlight, never a throw.
- **The ranking still has not been judged by a person.** Two fixtures, nine
  hand-run queries and 142 tests are not somebody typing `epn` and looking.

### Follow-up tasks

| Ticket | Why |
|---|---|
| T130-M (mount the bar in the app bar) | The last piece. Unblocked: T310-U landed at `f51c83c`. |
| A shortcut registry | The fifth task file to record it. |
| `createFakeTransport` unreachable from `apps/desktop` | `packages/protocol` publishes only `.`, so three dom tests now hand-roll the same transport seam. Already recorded at `shell-restore.dom.test.tsx:86`; this ticket is the third copy. |
