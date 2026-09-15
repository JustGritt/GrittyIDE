# Task SEARCH-CHORD — ⌘⇧F opens Search and focuses the field

- **Phase:** 2
- **Status:** review
- **Depends on:** T080-U (the search panel)

## Objective

T080-U put find-in-files in the sidebar and recorded the chord as its first
follow-up: the panel is reachable only by clicking its tab. ⌘⇧F on macOS,
Ctrl+Shift+F elsewhere, opens the Search tab and puts the cursor in its field,
from anywhere in the window — the editor above all.

## Dependencies

- **T080-U** for the panel, its shell-owned controller and the sidebar tab.
- **T120-M** for the pattern: `settings-chord.ts` (pure matcher) and
  `use-settings-chord.ts` (a window listener that does not yield to text
  areas).

## Required reading

- `apps/desktop/src/features/settings/settings-chord.ts`, `use-settings-chord.ts`
- `apps/desktop/src/features/shell/quick-open-chord.ts`, `use-quick-open-chord.ts`
  — why ⌘P yields to text areas and needed a Monaco registration
- `docs/tasks/T080-U-search-panel.md` — the controller, and the panel's
  focus-on-mount rule

## Allowed scope

- `apps/desktop/src/features/search/**`
- `apps/desktop/src/App.tsx` — mount the chord
- `docs/tasks/README.md` (generated), this file

## Explicit non-goals

- **No Monaco registration.** Monaco 0.56 binds ⌘F, ⌘⌥F, ⇧⌥F and ⌘K ⌘F
  (`findController.js`, `formatActions.js`) and nothing on ⌘⇧F, so the
  keystroke reaches the window from inside a buffer, as ⌘, already does.
- **No prefill from the editor selection** (`SEARCH-PREFILL`, separate).
- **No shortcut registry**, no palette entry. The command palette has no
  shortcut labels to show one with.

## Required behavior

1. `KeyF` with the platform accelerator and Shift, and nothing else, is the
   chord. ⌘⌥⇧F, ⌘⌃⇧F and plain ⌘F are not; ⌘F stays Monaco's find.
2. It is matched on `code`, not `key`, like every other chord in the shell.
3. It does **not** yield to text areas: the editor is the case that matters.
   Readline has no Ctrl+Shift+F, and legacy terminal encoding cannot tell it
   from Ctrl+F, so a live shell loses nothing it could have received.
4. It selects the Search tab, closes the attention queue if that view has
   replaced the shell body, focuses the query field and selects its text, so
   typing replaces the last query.
5. Pressed while Search is already showing, it focuses the field again — no
   remount is needed for that.
6. Inert until the core reports its platform, and with no workspace open.

## Failure behavior

| Situation | Behavior |
|---|---|
| No workspace | Nothing happens; the key is not claimed |
| Platform not yet known | Nothing happens |
| Superset modifiers | Not claimed |
| Pressed repeatedly | Idempotent: same tab, field focused |

## Acceptance criteria

- In `pnpm dev`, with the cursor in a file, ⌘⇧F shows the Search tab with the
  cursor in its field.
- Pressed again after clicking elsewhere, the field takes focus back.
- ⌘F still opens Monaco's find widget.
- `pnpm check` green.

## Tests required

- **Unit:** the matcher — both accelerators, supersets, `code` not `key`.
- **Hook (jsdom):** the listener fires from a textarea, prevents default, and
  is inert with a null accelerator or when disabled.
- **Panel (jsdom):** a focus request focuses and selects the field when the
  panel is already mounted.
- **Manual:** the acceptance criteria in `pnpm dev`. Record whether run.

## Commands to run

```
pnpm check
```

## Completion report

### Summary

⌘⇧F on macOS, Ctrl+Shift+F elsewhere, opens the Search tab, closes the
attention queue if it covers the sidebar, and puts the cursor in the query
field with the last query selected. Pressed while Search is showing, it takes
focus back to the field. The keystroke is claimed from anywhere, the editor
included, with no Monaco registration.

### Files changed

| File | What |
|---|---|
| `apps/desktop/src/features/search/search-chord.ts` (new) | the pure matcher |
| `apps/desktop/src/features/search/use-search-chord.ts` (new) | the window listener: no yield to text areas, inert without a platform or a workspace |
| `apps/desktop/src/features/search/use-search.ts` | `focusRequest`, `requestFocus`, `takeFocusRequest` on the controller |
| `apps/desktop/src/features/search/search-panel.tsx` | a focus request focuses and selects the field, and beats the tablist rule |
| `apps/desktop/src/App.tsx` | the chord mounted: queue closed, tab selected, focus requested |
| `apps/desktop/src/features/search/search-chord.test.ts`, `search-chord.dom.test.tsx` (new); `search-panel-render.test.ts` | tests; the render fixture gained the three new controller fields |

### Tests actually executed

- `npx vitest run apps/desktop/src/features/search` — 6 files, **79 passed**
  (12 new: matcher 4, listener 4, panel focus 4 against the real `useSearch`
  over a core that never answers).
- **Mutation check:** 9 hand mutations (Shift dropped, the other platform's
  modifier accepted, `key` instead of `code`, no `preventDefault`, the
  workspace gate removed, no `select()`, the ask ignored, a first-run guard,
  the take-once check). 8 killed. The survivor was a first-run guard that
  could not be reached, because the effect re-runs only when a request moved;
  it was deleted rather than left untested.
- `biome check`, `tsc --noEmit` — clean.
- `pnpm check` — **exit 0** (Biome, tsc, vitest 1552, fmt, clippy, cargo test, generated types fresh).
- **Manual in `pnpm dev`: not run by this session** (no display). Owed:
  ⌘⇧F from inside a file, again after clicking elsewhere, and ⌘F still opening
  Monaco's find.

### Architectural decisions

1. **Window listener only, no Monaco registration.** Monaco 0.56's defaults
   bind ⌘F, ⌘⌥F, ⇧⌥F and ⌘K ⌘F (`findController.js:439,816`,
   `formatActions.js:192,218`) and nothing on ⌘⇧F, so the keydown bubbles out
   of the buffer unhandled. That is the same path ⌘, (T120-M) already relies on.
2. **No yield to text areas**, unlike ⌘P: readline has no Ctrl+Shift+F.
3. **Focus is a request counter taken once**, not a flag the panel clears,
   so it reaches a panel already mounted and a panel mounting because of it,
   and a later remount does not replay an old chord.

### Known limitations

1. **Inside a terminal on Linux or Windows the chord likely never arrives.**
   xterm.js handles Ctrl-letter combinations itself and stops the event.
   Unverified either way: macOS only (rule 11). On macOS xterm leaves ⌘ chords
   alone.
2. **Monaco's ⌘K chord prefix** would swallow a ⌘⇧F pressed straight after ⌘K.
3. **Nothing on screen names the chord.** The command palette has no shortcut
   labels, and the Search tab's tooltip is its label alone.

### Follow-up tasks

| Proposed ticket | Why |
|---|---|
| `SEARCH-PREFILL` | ⌘⇧F with a selection in the editor should search for it |
| `SEARCH-SHORTCUT-HINT` | say ⌘⇧F somewhere: the Search tab's tooltip, or a palette entry |
| a shortcut registry | the ninth hand-gated chord; recorded debt, not this ticket's |

### Remaining risks

- The "Monaco lets it bubble" claim rests on reading Monaco's default keybinding
  table and on ⌘, working the same way; it is the first thing the manual check
  confirms.
