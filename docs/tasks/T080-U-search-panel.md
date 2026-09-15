# Task T080-U — The search panel

- **Phase:** 2
- **Status:** doing
- **Depends on:** T080 (`search.query`, the walk), PROTO-W3 (the wire: `SearchMatchesFound`, `SearchCompletion`)

## Objective

T080 shipped find-in-files with no way to type a query: `search.query` streams
matches over `SearchMatchesFound` and nothing in the renderer consumes it
(`docs/tasks/completed/T080-search.md`, known limitation 8). This task is the
panel: a third sidebar section beside **Files** and **Changes**, where a query
runs as the user types, the results stream in grouped by file, a result opens
the file at the match, and a running search can be stopped.

## Dependencies

- **T080 / PROTO-W3** for `core.search.query`, `SearchMatchesFound` and the
  `SearchCompletion` result — all on the wire; nothing here changes it.
- **T100-U** for the sidebar tablist (`features/git/sidebar-tabs.tsx`) and the
  frame that switches its panels (`features/explorer/explorer-panel.tsx`).
- **T041 / T030** for `documents.openFile` (preview on single click) and
  `EditorBinding.revealPosition`, which already waits for a document that is
  not active yet.

## Required reading

- `docs/tasks/completed/T080-search.md` — bounds, the 20 000 cap, one match per
  line, UTF-16 columns, no replay after a reload
- `packages/protocol/src/client.ts` — `search` and its doc comment
- `docs/decisions/ADR-009*` — a command is ordered against the listener
  registration, not against the events that follow its response
- `apps/desktop/src/features/quick-open/use-quick-open.ts` — debounce,
  superseding, and the race `awaitOperation` closes
- `docs/decisions/ADR-034-meaning-of-ignored.md` — why the include-ignored
  toggle is not here

## Allowed scope

- `apps/desktop/src/features/search/**` (new)
- `apps/desktop/src/features/git/sidebar-tabs.tsx`, `sidebar-icons.tsx`,
  `sidebar-tabs.dom.test.tsx` — the third tab
- `apps/desktop/src/features/explorer/explorer-panel.tsx` — the frame switches
  a third panel (a few lines; held by WATCH-DEGRADED at the start, sequenced)
- `apps/desktop/src/App.tsx` — mount the controller and the panel
- `apps/desktop/src/app.css` — one appended `.search` block (held by T099-S at
  the start, sequenced)
- `docs/tasks/README.md` (generated), this file

## Explicit non-goals

- **No include-ignored toggle.** ADR-034 gives "ignored" a meaning and
  `IGNORE-UI` owns the toggle in search and quick-open. The panel reports how
  many ignored directories were skipped, and nothing more.
- **No replace.** `search.query` reads; T080 records replace as its own risk.
- **No ⌘⇧F chord.** The window half alone would not fire inside Monaco — the
  primary case — and the Monaco half lives in `packages/editor` and
  `use-editor-binding.ts`. A chord that works everywhere except the editor is
  worse than none; recorded as a follow-up.
- **No wire change**, no protocol bump, no Rust.
- **No replay after a reload** (T080 non-goal): the panel starts empty.
- **No include/exclude globs** — the request has no field for them.

## Required behavior

1. A third sidebar tab, **Search**, with an icon like its two siblings. Its
   state lives in a controller the shell owns, so switching to Files and back
   keeps the query, the options and the results, and a running search keeps
   running while its tab is hidden.
2. A query field and three toggles — case-sensitive, whole word, regular
   expression — plus the scope (contents and names, contents, names). The
   search runs as the user types after a short debounce, and at once on Enter.
   An empty or whitespace-only field sends nothing and clears the results.
3. A new query, or a changed option, **supersedes** the running one: its
   operation is cancelled, and nothing it still emits reaches the results.
4. Events that arrive before `search.query`'s response are not lost (ADR-009):
   they are held, bounded, and folded once the operation id is known. Held
   batches that overflow the bound count into the undelivered total rather than
   vanishing.
5. Results group by file in arrival order: the file's name and directory, its
   match count, then one row per match with the line number and the snippet,
   the match highlighted from `matchStart`/`matchLength`. A truncated snippet
   says so. A filename match is a row of its own. A file group collapses.
6. Activating a match opens its file as a preview tab and moves the cursor to
   the match's line and column; a double click keeps the tab. A filename match
   opens the file. Up/Down move between rows.
7. Rendering is bounded: the first page of matches is drawn and the rest are
   one "Show more" away. The count never pretends the list is complete.
8. While running: a live count and a **Stop** that cancels the operation.
9. When it ends, the panel says what happened, in words:
   - completed — "N results in M files";
   - `limitReached` — stopped at the core's cap, results are partial;
   - `truncatedMatches > 0` — matches found and not delivered;
   - files skipped by reason (binary, not UTF-8, too large, unreadable, outside
     the root) and ignored directories not searched;
   - cancelled — stopped, and what was found before stays on screen;
   - failed — the error envelope.
10. `InvalidInput` (a regex that does not compile) is shown beside the field,
    not as an alert, and the previous results are cleared rather than left
    under a query they do not answer.
11. A workspace closing or changing cancels a running search and clears the
    panel.

## Failure behavior

| Situation | Behavior |
|---|---|
| Regex does not compile | `InvalidInput` from the command; inline message, no operation, no results |
| No workspace | The field is disabled and the panel says so |
| The operation fails | Error envelope; matches already delivered stay |
| Stop pressed before the response arrives | Cancelled as soon as the id is known |
| A superseded operation's events | Ignored; its operation cancelled |
| The response never arrives / the command rejects | Error envelope, run ends |
| Workspace closes mid-search | Cancel sent, panel cleared |
| Tab hidden mid-search | Search continues; results waiting on return |
| Renderer reload | Panel empty; the orphaned search ends on its own (bounded by the cap) |

## Acceptance criteria

- Typing a known token in the running app lists its matches grouped by file;
  clicking one opens the file with the cursor on the match.
- Changing a toggle re-runs; a bad regex says so beside the field.
- Stop ends a long search and keeps what it found.
- Switching sidebar tabs does not lose results.
- `pnpm check` green.

## Tests required

- **Unit (TS):** the reducer — batches fold by file; events before the
  response are held and folded; the hold is bounded and overflow is counted;
  a superseded operation's events are dropped; completion, cancellation,
  failure and `InvalidInput` arms.
- **Unit (TS):** the model — highlight segments at UTF-16 offsets, clipped
  ranges, the summary sentences, the render page.
- **Render (TS):** every arm of the panel as static markup.
- **Hook (jsdom):** debounce, supersede-and-cancel, Stop before the response,
  and the workspace change, against a stubbed transport.
- **Tab (jsdom):** the tablist has three tabs and Arrow keys reach Search.
- **Manual:** a query against the real workspace from `pnpm dev`, stopped
  mid-way. Record whether it was run.

## Commands to run

```
pnpm check
```

## Completion report

### Summary

Find-in-files has a panel. **Search** is the sidebar's third tab: a field with
case, whole-word and regex toggles and a scope picker, running `search.query`
250 ms after typing stops (at once on Enter or a toggle), streaming results
grouped by file with the match highlighted, opening a result as a preview tab
with the cursor on the match, and saying in words when the list is not the
whole answer. Nothing on the wire changed.

### Files changed

| File | What |
|---|---|
| `apps/desktop/src/features/search/search-reducer.ts` (new) | the run state machine: tokens, the pre-response hold, folding, the two undelivered counts |
| `apps/desktop/src/features/search/search-model.ts` (new) | highlight segments, the render page, every sentence about a run |
| `apps/desktop/src/features/search/use-search.ts` (new) | the controller: debounce, supersede-and-cancel, Stop, workspace change |
| `apps/desktop/src/features/search/search-panel.tsx` (new) | the view and its keyboard routing |
| `apps/desktop/src/features/search/*.test.ts(x)` (new) | four test files, below |
| `apps/desktop/src/features/git/sidebar-tabs.tsx`, `sidebar-icons.tsx`, `sidebar-tabs.dom.test.tsx` | the third tab, its magnifier glyph, the tablist test widened to three |
| `apps/desktop/src/features/explorer/explorer-panel.tsx` | a `search` panel slot beside `changes`; the switch names the selected tab's ids |
| `apps/desktop/src/App.tsx` | `useSearch` next to the explorer; the panel mounted with open-at-match |
| `apps/desktop/src/app.css` | one appended `.search` block |

### Tests actually executed

- `npx vitest run apps/desktop/src/features/search` — 4 files, **67 passed**
  (reducer 23, model 17, render 15, hook 12).
- `npx vitest run` (whole workspace) — **134 files, 1540 passed**.
- **Mutation check** of the guards the tests are meant to hold: 11 hand-made
  mutations (the ADR-009 hold, hold overflow accounting, terminal-event
  survival, completion total, cancel of a stale response, Stop before the
  response, synchronous run tracking, cancel on supersede, clear on workspace
  change, indentation trim, the page's name-match cost). **All 11 killed.**
- `biome check` and `tsc --noEmit` on the touched files — clean.
- `pnpm check` — **exit 0** (Biome, tsc, vitest 1540, `cargo fmt --check`,
  clippy `-D warnings`, `cargo test --workspace`, generated types fresh). Run
  on the shared tree, which also carried T099-S's uncommitted terminal work.
- **Visual, without a screen:** the panel SSR-rendered through Vite with the
  real `app.css` and screenshotted in headless Chromium, dark and light, in
  five states (completed with notes, running, collapsed with the page cut,
  invalid regex, no workspace). It found one defect the tests could not —
  four-digit line numbers and the `name` label pushed their snippets out of
  column — fixed by a 4ch gutter and re-shot.
- **Not run:** a query against the real workspace in `pnpm dev`. This session
  has no display. The manual check in "Tests required" is still owed.

### Architectural decisions

1. **The controller belongs to the shell, the panel is a view.** The sidebar
   mounts only the selected section; a controller inside the panel would lose
   the results on every look at the tree and abandon a running search.
2. **A run is named by a token until its operation id exists.** Superseded
   answers are recognised by token and their operations cancelled.
3. **Events before the response are held, not dropped (ADR-009).** The hold is
   bounded at 64; progress keeps only the latest per operation; a terminal
   event is never evicted; evicted batches are counted per operation id and
   charged to the run only if the id turns out to be its own.
4. **"Undelivered" is the core's `truncatedMatches` plus the hold's own
   drops**, and the panel shows the sum.
5. **What the hook forwards is decided by a synchronous ref, not by
   `state.run`**, because an event, the response and Stop can all land in the
   render gap after a dispatch.
6. **Rendering is paged at 500 results**; a collapsed file costs nothing
   against the page, which is how a user reaches files beyond it.
7. **An `InvalidInput` refusal is inline**, beside the field, with
   `aria-invalid`; every other failure is the error envelope with the partial
   results kept.
8. **Focus on opening the tab goes to the field**, unless focus is on the
   tablist (a keyboard user arrowing through tabs keeps it there).

### Known limitations

1. **No ⌘⇧F.** The window listener alone would not fire inside Monaco. Follow-up.
2. **The match is not selected**, only the cursor placed at its start:
   `EditorBinding.revealPosition` takes a position, not a range.
3. **Results go stale** when files change after the search; line numbers can
   drift until it is re-run (Enter re-runs the same query).
4. **After a renderer reload the panel is empty** and an orphaned search runs
   to its own end (bounded by the core's 20 000 cap). T080's non-goal.
5. **No include-ignored toggle** — `IGNORE-UI`'s, per ADR-034. The panel says
   how many ignored directories were skipped.
6. **Arrow keys walk rendered rows by DOM query**, which is linear in the page
   (≤ 500 matches plus headers) per keypress. Fine at the page size; revisit
   if the page grows.

### Follow-up tasks

| Proposed ticket | Why |
|---|---|
| `SEARCH-CHORD` (⌘⇧F opens Search and focuses the field) | needs the Monaco half in `packages/editor` and `use-editor-binding.ts`, like ⌘P |
| `SEARCH-SELECT-MATCH` (select the match, not just place the cursor) | `revealPosition` would need a range variant |
| `SEARCH-PREFILL` (prefill the field from the editor selection) | the usual find-in-files affordance |
| `docs/plans/execution-board.md` still lists T080-U as unbuilt | held by PLAN-OF-RECORD at the time; not edited here |

### Remaining risks

- **Never exercised against a live core.** The stubbed transport matches the
  wire types, and T080's own integration tests cover the core side, but the
  first real run is the manual check.
- **Lease sequencing:** `app.css` was held by T099-S (split terminals), whose
  session had stopped on an API error. With the user's go-ahead its lease was
  narrowed off `app.css`, only this ticket's appended block was committed from
  a private index, and T099-S's uncommitted lines in the file were left as
  they were.
