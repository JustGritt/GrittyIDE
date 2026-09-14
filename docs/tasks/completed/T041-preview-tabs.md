# Task T041 — Preview tabs, and closing more than one at a time

- **Phase:** 1A
- **Status:** done
- **Depends on:** T020 (explorer), T030 (document/buffer contract), T040 (Monaco shell)

> **This spec was written after the code, not before it.** The feature landed on
> `main` inside `cb9e08a` — "Checkpoint: T096/T098 terminal chrome, T220 agent
> discovery, preview tabs" — a commit that carries three other tasks, is the
> only non-merge commit in `main`'s entire first-parent history, and went
> through no pull request. It had no task file, no board row, no line in
> `docs/tasks/README.md` and no step in the manual pass, and it changes
> user-visible editor behaviour: a single click in the explorer now opens a tab
> that the next single click replaces.
>
> `docs/testing/review-findings.md` finding 4 is where that was caught. This
> ticket exists so the behaviour has something to be checked *against*, and so a
> reviewer walking Session 2 of the manual pass does not meet it unannounced.
> The Required behavior below is **reconstructed from the diff and the code as
> it stands**, not handed to an author in advance — so a mismatch between this
> file and the code is as likely to be an error in this file as a defect, and
> should be resolved by reading `documents-reducer.ts` rather than by assuming.

## Objective

Opening a file to look at it should not cost a tab. A single click in the
explorer opens the file in a *preview* tab; the next single click replaces it.
Anything that says "I mean to keep this" — a double click, a drag, or the first
edit — turns it into an ordinary tab.

The rule that carries the weight is the edit rule: **a buffer with unsaved work
in it must never be the tab a later click silently closes.**

Alongside it, closing more than one tab at a time ("close tabs to the right")
without dropping an unsaved buffer to save a click.

## Dependencies

- T030 for the document/buffer contract: `documents.open` / `documents.close`,
  and the dirty state a preview must not discard.
- T040 for the tab strip and the editor binding.
- T020 for the explorer tree the single click comes from.

## Required reading

- `docs/decisions/ADR-011-document-buffer-contract.md`,
  `docs/decisions/ADR-012-monaco-editor-binding.md`
- `apps/desktop/src/features/documents/documents-reducer.ts` — `previewId`,
  `keep`, `open`, `remove`
- `apps/desktop/src/features/documents/use-documents.ts` — `openFile`,
  `retirePreview`
- `apps/desktop/src/features/editor/{tab-strip.tsx,close-plan.ts,use-close-flow.ts}`
- `docs/testing/review-findings.md` finding 4

## Allowed scope

- `apps/desktop/src/features/documents/**`
- `apps/desktop/src/features/editor/**`
- `apps/desktop/src/features/explorer/explorer-panel.tsx` (the click handlers only)
- `apps/desktop/src/{App.tsx,app.css}`
- `docs/testing/phase-0-manual-pass.md`, `docs/tasks/README.md`, this file

## Explicit non-goals

- **No protocol change.** Preview is a property of the *tab*, not of the
  document: the core has one buffer per path either way, and nothing about this
  reaches the wire.
- **No persistence.** A preview does not survive a restart as a preview.
- **No pinned tabs, no tab groups, no split editors, no "reopen closed tab".**
- **No change to what `documents.close` means**, and no new close path: retiring
  a preview goes through the same command a × does.
- The tab context menu reuses `components/menu.tsx`, which T096 introduced for
  the terminal pill. Do not fork it.

## Required behavior

### The preview tab

1. **A single click in the explorer opens a preview.** A double click opens an
   ordinary tab. The double click fires its single click first, so the two must
   converge on "kept" whatever order the opens resolve in.
2. **At most one preview exists.** Opening another preview replaces it: the
   previous one is closed through `documents.close`, after the new tab exists,
   so the editor never blinks through an empty pane.
3. **A preview is visually distinct** — italics, the convention every editor
   with this behaviour uses — and says so in its title attribute.
4. **A double click on the tab keeps it.** So does a drag that reorders it.
5. **The first edit keeps it.** A draft that arrives without a touch keeps it
   too. An edit that matches what the core already holds does not — nothing
   changed, so nothing was decided.
6. **A deliberate open of a file that is already a preview keeps it**, and a
   single click on a tab the user chose to keep never demotes it back.
7. **Closing the preview clears the mark**, whichever path closed it: the ×, a
   batch close, or the core reporting the document gone.
8. **Preview is never persisted.** A tab restored from a session is an ordinary
   tab — restoring one as a preview would be a tab the user never asked to keep,
   silently disposable on the next click.

### Closing more than one

9. **The tab context menu offers Close and "close tabs to the right"**, the
   latter disabled when there are none. It opens from a right click and from
   the keyboard.
10. **A batch close is a queue, not a loop.** Everything up to the first unsaved
    document closes; that one raises the existing unsaved-changes dialog; the
    rest waits for the answer and is replanned against the state that follows.
    Closing several tabs must never drop an unsaved buffer to save a click.

## Failure behavior

| Situation | Behaviour |
|---|---|
| The file behind a single click fails to open | The error surfaces as the envelope; the current preview is **not** retired — losing the file the user was reading to make room for one that never arrived is the worse outcome |
| `documents.close` refuses when retiring a preview | The tab stays, as an ordinary tab. Not reported: one extra tab costs the user nothing, and the safe state is the one where nothing was lost |
| The preview is edited while the replacing file is still loading | It is kept, and survives. `retirePreview` re-reads the tab and leaves an unsaved one alone — the second guard, after the edit rule |
| A tab in a batch close is already gone when its turn comes | Skipped, not an error |
| The user cancels the dialog mid-batch | The queue stops there. Nothing behind the cancelled tab closes |

## Acceptance criteria

- Single-clicking three files in a row leaves **one** tab open, showing the third.
- Typing one character into a preview and single-clicking another file leaves
  **two** tabs, and the edited one is still dirty.
- Double-clicking a previewed tab, then single-clicking elsewhere, leaves it open.
- "Close tabs to the right" over a run containing two unsaved files asks twice,
  in order, and closes nothing the user did not answer for.
- `pnpm check` green.

## Tests required

- `documents-reducer.test.ts` — at most one preview; opening without preview
  keeps; promotion; the touch/draft/no-op-edit rules; arrange keeps; close
  clears the mark by every path; a re-preview of a kept tab does not demote it,
  in either order of arrival.
- `close-plan.test.ts` — the queue splits at the first unsaved document, skips a
  tab that vanished, and `tabsToTheRight` is empty for the last tab.
- **Manual:** the acceptance criteria above, in the running app. There is no
  component-test infrastructure for click behaviour (see T232's known
  limitation 4), so the click semantics are only ever checked by hand.

## Commands to run

```
pnpm check
pnpm dev   # manual pass
```

## Completion report

**Reconstructed from `cb9e08a`, not written by the author.** No completion
report was filed for this work; what follows is what the diff and the code
show, recorded so the ticket is reviewable. Treat the "tests actually executed"
section as the weakest part of it — the evidence is a commit message.

### Summary

Preview tabs, the tab context menu and the batch-close queue, all in the
renderer. `DocumentsState` gains `previewId` — at most one, replaced rather than
accumulated, and a property of the tab rather than of the document, so nothing
crosses the protocol and nothing is persisted. `keep()` is called from the
double click, from a reorder and from the first edit; `retirePreview` closes the
outgoing one after the incoming tab exists.

`close-plan.ts` is a pure planner: `planClose` splits a queue at the first
unsaved document and `tabsToTheRight` supplies the menu's list.

### Files changed

| File | What |
|---|---|
| `features/documents/documents-reducer.ts` | `previewId`, `isPreview`, `keep`, the `preview` arm of `open`, and clearing on `remove` |
| `features/documents/documents-reducer.test.ts` | 12 preview cases |
| `features/documents/use-documents.ts` | `openFile({ preview })`, `retirePreview`, the `promote` dispatch on a deliberate open |
| `features/editor/close-plan.ts` | new — `planClose`, `tabsToTheRight` |
| `features/editor/close-plan.test.ts` | new — 8 cases |
| `features/editor/tab-strip.tsx` | the italic preview tab, the double click that keeps it, the context menu |
| `features/editor/use-close-flow.ts` | the batch queue and its dialog |
| `features/explorer/explorer-panel.tsx` | single click previews, double click keeps |
| `App.tsx` | passes `preview` through to `documents.openFile` |

`components/menu.tsx` arrived in the same commit and belongs to **T096**, which
introduced it for the terminal pill; the tab strip is its second consumer.
`use-editor-binding.ts`'s change in that commit is **T210**'s — a mountable
editor container, so the attention queue can swap the body — and is not part of
this work, though review finding 4 counted it here.

### Tests actually executed

**Unknown.** No report was filed. The only evidence is `cb9e08a`'s message —
"cargo fmt/clippy clean, cargo test --workspace green, tsc clean, 386 vitest
tests green, biome clean" — which `review-findings.md` corroborated
arithmetically against the tree, but a commit message is not a completion
report. The 20 unit tests above exist and pass today.

### Known limitations

1. **Nothing here has been exercised in the running app.** `cb9e08a`'s own
   message says so of the whole checkpoint. Every click rule in Required
   behavior 1–7 is therefore unverified, and click behaviour is precisely what
   a unit test over the reducer cannot check: the reducer is tested, the
   *gesture that dispatches to it* is not.
2. ~~**No manual-pass step covers it.**~~ Steps **2.18a–2.18h** were added with
   this ticket. They have not been run: they are the worklist, not the result.
3. **The double click is a race by construction.** It fires its single click
   first, so up to three opens of the same path can be in flight. The reducer is
   written so every order of arrival lands on "kept", and that is unit-tested —
   but it has never been produced by a real double click on a real filesystem.
4. **`retirePreview` failures are silent.** Deliberately: an extra tab is a
   better outcome than a lost file. It does mean a core that consistently
   refused `documents.close` would leak tabs with no visible cause.

### Follow-up tasks

1. **Run manual-pass 2.18a–2.18h.** That is the whole of what this ticket is
   waiting on; nothing else here is construction work.
2. **Decide whether a preview should be revealed in the explorer**, and whether
   the keyboard has any way to keep one. Today only the mouse can.
3. **"Close others" and "close saved"** are the obvious neighbours of the one
   menu item that exists; `planClose` already takes an arbitrary queue.
