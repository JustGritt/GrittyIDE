# Task T130-M — Mount the bar in the app bar, and the quick-open chord

- **Phase:** 2
- **Status:** todo
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

_Blocked; not started._
