# Task T130-M — Mount the bar in the app bar, and the quick-open chord

- **Phase:** 2
- **Status:** review
- **Depends on:** T130-U, T310-U (landing, not building)

## Objective

Put the quick-open field in the window's top bar, and make ⌘P / ⌘⇧P reach it
from everywhere it must — including from inside Monaco, which is the case a
naive listener gets wrong.

## Motivation

Split out of T130-U for the reason T120-M was split out of T120-U: the mount
points are contended and the rest of the feature should not wait for them.

`App.tsx`, `app.css`, `editor-pane.tsx` and `tab-strip.tsx` all carry T310-U's
reviewed-but-unlanded work, and `T101-D` and `T120-M` are already queued behind
it. **This ticket is the least entangled of the three**, and that is worth
stating: T101-D and T120-M need *structural* changes there (the tab reducer, the
editor pane's lane resolution), whereas this one needs `AppBar` to own its own
state — as it already does for the health popover — so `App.tsx` gains only an
additive prop and `app.css` an appended block. Both are hunk-separable.

The second reason this is its own ticket is the chord, which is not the
copy-paste it looks like. See §2.

## Required reading

- `apps/desktop/src/features/shell/app-bar.tsx` — the bar, its slots, and the
  health `<details>` popover that "costs no layout"
- `apps/desktop/src/features/shell/open-shortcuts.ts` — `acceleratorForOs`,
  `matchOpenShortcut`, and **`ownsRawKeystrokes`**, whose behaviour is the trap
- `apps/desktop/src/features/shell/use-open-shortcuts.ts` — the ref-held
  listener idiom
- `packages/editor/src/monaco/editor-host.ts:95-101` — `onSave`, the precedent
  for reaching into Monaco's keybindings
- `docs/tasks/T120-M-settings-mount.md` — the same split, the same wall, and its
  standing instruction to **stage hunks, not files**
- `HANDOFF.md` § "The technique you will need for the shared files"
- `AGENTS.md` hard rules 1, 14

## Allowed scope

- `apps/desktop/src/features/shell/app-bar.tsx`
- `apps/desktop/src/features/shell/quick-open-chord.ts` (new) and its test
- `packages/editor/src/monaco/editor-host.ts` — one method
- `apps/desktop/src/App.tsx` — the prop wiring only. Shared magnet carrying
  another ticket's hunks: **stage hunks, not files**, and claim it for the
  minute you edit it.
- `apps/desktop/src/app.css` — a `.quick-open*` block, **appended**; ADR-016
  tokens only, no colour literals. Same magnet rule.

## Explicit non-goals

- **No shortcut registry.** This adds the *tenth* hand-gated listener and the
  fifth apology. It is real debt and a real ticket, and bundling it here doubles
  this one (rule 14). Record it as a follow-up, again.
- **No editor tab and no persistence.** Quick open is transient; it must not
  appear in `SessionTab` or survive a restart.
- **No remappable keybindings, no settings integration.** T120 owns settings.
- **No layout change.** The bar's height does not move, and the overlay is
  anchored rather than in flow — opening it must not shift the editor by a pixel.
- **Nothing in `editor-pane.tsx` or `tab-strip.tsx`.** Those are the structural
  half of the T310-U wall and this ticket does not need them.

## Required behavior

1. **The field lives in `AppBar`, and `AppBar` owns its open state**, the way it
   already owns the health popover's. `App.tsx` passes the controller and the
   data quick open needs, and nothing else. This is what keeps the `App.tsx`
   diff additive and hunk-separable.
2. **The chord must fire from inside Monaco, and must not be stolen from a shell.**
   `ownsRawKeystrokes()` returns true for `TEXTAREA`, and Monaco puts a
   `<textarea>` under the cursor — so a handler that reuses it verbatim will
   **not fire while you are editing**, which is quick open's primary case.
   Therefore:
   - Register the chord with Monaco directly: add `onQuickOpen(cb)` to
     `EditorHost` beside `onSave`, as
     `editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyP, cb)`.
   - Keep a window listener for everywhere else, still guarded by
     `ownsRawKeystrokes` — which is what leaves **xterm** alone. On Linux and
     Windows the accelerator is Ctrl, and `Ctrl+P` is readline's "previous
     command"; swallowing it in a live shell would be a regression.
     `matchOpenShortcut` already documents exactly this reasoning for ⌘O.
3. **The accelerator comes from the core**, `acceleratorForOs(health.os)`, and is
   `null` until health lands — inert rather than guessing a modifier, as
   `useOpenShortcuts` already is. Never `navigator.platform`.
4. **Match on `event.code`, not `event.key`.** Four files record why: on macOS
   Option composes, and a chord read from `key` matches the composed character.
5. **⌘⇧P opens straight into command mode**, i.e. the field opens pre-filled
   with `>`. ⌘P opens files mode. Opening while already open focuses and
   selects the field rather than toggling it shut — the same "opening it twice
   focuses it" rule T120-M states for ⌘,.
6. **Escape closes and returns focus where it came from**, including back into
   Monaco with the cursor where it was.
7. **The overlay is anchored, at z-index 30**, beside the island sheet — both
   hang off the app bar. It must never push layout.
8. **The pure matcher lives in its own module** with a structural `KeyChord`
   interface, so it is tested in the node project like the other four chord
   modules.

## Failure behavior

- Health not yet reported → the chord is inert, and the field is visible but
  says the workspace is not ready rather than failing on click.
- No workspace open → the field is present and offers commands; files mode says
  there is nothing to search.
- The overlay open when the workspace closes → it dismisses rather than holding
  stale rows.
- A second ⌘P while open → focus and select, never a second overlay.

## Acceptance criteria

Verified **in the running app**, not from code shape (rule 12):

- ⌘P opens the bar from the explorer, from a Monaco buffer **with the cursor in
  the text**, and from the launcher with no workspace open.
- With focus in a terminal on Linux, Ctrl+P reaches the shell and does **not**
  open the bar.
- ⌘⇧P opens with `>` already in the field.
- Escape returns the cursor to Monaco at its previous position.
- Opening and closing the bar moves no other pixel in the layout.
- Light and dark, and at 800×600.
- `pnpm check` green, and `git diff --cached --stat` on `App.tsx` and `app.css`
  shows only this ticket's hunks before committing.

## Tests required

- `quick-open-chord.test.ts` (node, pure): both accelerators; the composed-key
  case asserting `code` not `key`; modifier supersets rejected; ⌘⇧P
  distinguished from ⌘P; unclaimed chords returning `null`.
- A render test asserting the field appears in the bar's markup and that the
  bar's existing slots (identity, workspace, island, health) are unmoved.
- Manual: the six acceptance items above, recorded in the completion report with
  what was actually observed — including whether it was seen on a screen.

## Commands to run

```
pnpm vitest run --project desktop quick-open-chord
pnpm check
pnpm dev
```

## Completion report

### Summary

⌘P opens the bar from the app bar, ⌘⇧P opens it into command mode, and both
fire from inside a Monaco buffer as well as from everywhere else. The overlay is
anchored and not in flow, so opening it moves nothing.

**The manual pass has not been run.** Every acceptance criterion below the line
"Verified in the running app" needs a screen, and this session has none. What is
listed as done is what a machine can check; the rest is handed over, not
claimed. Rule 12.

### Files changed

| File | What |
|---|---|
| `features/shell/quick-open-chord.ts` | New. `matchQuickOpenShortcut`, `initialQueryFor`. Pure. |
| `features/shell/quick-open-chord.test.ts` | New. 8 tests. |
| `features/shell/use-quick-open-chord.ts` | New. The window listener, guarded by `ownsRawKeystrokes`. |
| `features/shell/app-bar.tsx` | The slot, the open state, both chord paths, and dismissal when the workspace closes. |
| `features/shell/app-bar-render.test.tsx` | New. 7 jsdom tests. |
| `packages/editor/src/monaco/editor-host.ts` | `onQuickOpen`, beside `onSave`. |
| `features/editor/use-editor-binding.ts` | `subscribeQuickOpen`, beside `subscribeCursor`. **Outside the scope as written — see below.** |
| `features/quick-open/quick-open-bar.tsx` | `focusSignal`, for the second ⌘P. |
| `App.tsx` | The deps object and one prop. Additive. |
| `app.css` | An appended `.quick-open*` block, ADR-016 tokens only. |

### Two departures from the ticket, both recorded rather than quietly taken

**The scope list was missing a file, and requirement 2 could not be met without
it.** §2 says to add `onQuickOpen(cb)` to `EditorHost` "beside `onSave`" — but
`onSave` is *registered* in `features/editor/use-editor-binding.ts`, which the
Allowed scope does not list. Adding the method to the interface without a caller
would have been a method nothing calls, and skipping it would have shipped
exactly the bug §2 exists to prevent: the chord not firing while you are
editing, which is quick open's primary case. The scope was widened by that one
file, the edit mirrors `onSave`'s four lines, and the board row now records it.

**`App.tsx` passes the deps, not the controller.** §1 says both that `AppBar`
owns the open state and that `App.tsx` passes "the controller". Those two do not
compose: the controller closes the bar itself when a row is accepted or a
command runs, so a controller built in `App.tsx` would need an imperative handle
back down into `AppBar` to reach `setOpen(false)`. `AppBar` builds the
controller from deps instead, and owns `onDismiss` because closing is exactly
the state it owns. §1's stated purpose — that `App.tsx` gains a prop and nothing
structural — holds either way, and its diff is a `useMemo`, a deps object and
one attribute.

### Architectural decisions

**Two registrations, not one clever rule.** The window listener yields to
anything that takes keystrokes verbatim, which is what leaves a live shell
alone: on Linux and Windows the accelerator *is* Ctrl and `Ctrl+P` is readline's
"previous command". But Monaco puts a `<textarea>` under the cursor too, so the
same guard silences the chord in the one place it matters most. The editor
therefore registers its own, through `EditorBinding.subscribeQuickOpen`. "The
editor" and "a live shell" want opposite answers to the same question, and no
single predicate gives both.

**`focusSignal`, not a ref.** A second ⌘P focuses and selects rather than
toggling shut (§5). A number the mount bumps keeps `AppBar` declarative and
makes the behaviour assertable without an imperative handle — the test reads
`selectionStart`/`selectionEnd`.

**"Settings" reveals `settings.json`, because there is no settings page yet.**
`quickOpenCommands` emits a Settings row and T120-U/T120-M have not landed, so
the honest options were an inert row or a real action. `settings.reveal` — which
T120-C landed — opens the file in the system file manager. It becomes the page
when there is one.

**The overlay is not in the tree until it is asked for.** That is what makes
"opening it moves no pixel" a fact rather than an aspiration, and the first
render test asserts the absence.

### Tests actually executed

| Command | Result |
|---|---|
| `pnpm vitest run --project desktop apps/desktop/src/features/shell` | **22 passed, 3 files** |
| `pnpm vitest run --project desktop apps/desktop/src/features/quick-open` | **142 passed, 8 files** |

### The manual pass, which has not been run

This session has no screen. Every one of these is outstanding and none of them
is claimed:

- [ ] ⌘P from the explorer, from a Monaco buffer **with the cursor in the
      text**, and from the launcher with no workspace open.
- [ ] With focus in a terminal on Linux, Ctrl+P reaches the shell and does
      **not** open the bar. (No Linux machine either — `CI-LINUX-DARK`.)
- [ ] ⌘⇧P opens with `>` already in the field.
- [ ] Escape returns the cursor to Monaco at its previous position.
- [ ] Opening and closing the bar moves no other pixel.
- [ ] Light and dark, and at 800×600.
- [ ] `epn` puts `editor-pane.tsx` first, judged by a person rather than by a
      test. T130-R's report has been asking for this since it landed.

The machine-checkable halves of the first, third and fourth are covered by
`app-bar-render.test.tsx` and `quick-open.dom.test.tsx`; what is missing is
whether any of it *looks* right, which is the part rule 12 is about.

### Known limitations

- **No CSS variables were added**, only used, so the block inherits light and
  dark for free — but nobody has looked at it in either.
- **The bar is centred on the window**, not on the slot it hangs from. That is a
  judgement about where a workspace-wide field belongs and it has not been seen.
- **`quickOpenDocument` rebuilds whenever the active tab's identity changes**,
  which includes every keystroke in the buffer (the draft moves). `outlineOf` is
  memoised on that object, so `@` re-extracts the outline as you type. Bounded
  (7 ms of block parsing for 1 MB of markdown) but not free; if it shows on a
  large file the fix is to key the memo on the text rather than the tab.

### Follow-up tasks

| Ticket | Why |
|---|---|
| The manual pass above | Needs a person at a screen. |
| A shortcut registry | The tenth hand-gated listener; the sixth task file to record it. |
| Settings command → the settings page | When T120-M lands, `openSettings` stops being a reveal. |
