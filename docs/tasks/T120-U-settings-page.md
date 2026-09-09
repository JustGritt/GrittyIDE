# Task T120-U — The settings page, and retiring the three popovers

- **Phase:** 2
- **Status:** todo
- **Depends on:** T120-C (the contract). Independent of T120-T.

## Objective

Build `apps/desktop/src/features/settings/`: a schema-driven settings page with
a category rail, search, per-setting modified marks and reset, rendered as an
editor-pane surface. Retire the three scattered `<details>` popovers by moving
what they do into it. **New files only** — the mounting is T120-M.

## Motivation

There are three unrelated settings popovers today (editor pane, terminal bar,
agents panel) and no central page, so there is no answer to "where do I change
that". The page is also what makes T120-R's registry worth having: a setting
declared once should produce its own control.

Splitting the mount out is deliberate. `App.tsx`, `editor-pane.tsx` and
`tab-strip.tsx` all carry another ticket's uncommitted work — the wall T101-D
is already behind — and everything in *this* ticket is new files that collide
with nothing.

## Dependencies

`settings.describe` / `get` / `set` / `reset` from T120-C. The page renders
whatever the schema describes, so it does not depend on which settings exist.

## Required reading

- `AGENTS.md` hard rules 1, 2, 14
- `docs/decisions/ADR-031-settings-storage.md` §3 (the registry drives the form)
  and §7 (settings vs layout)
- `docs/decisions/ADR-026-agent-console-tab.md` — the non-file-tab pattern, and
  its "Revisit when: a second kind of non-document tab is wanted"
- `apps/desktop/src/features/runs/run-console-model.ts` — `PromptGate`, the
  shape to copy for "why is this control unavailable"
- `apps/desktop/src/features/runs/run-console.tsx` and its three tests — the
  house three-tier test split
- `apps/desktop/src/features/agents/agent-tabs-reducer.ts` — `ConsoleKey`
- The three surfaces being retired: `features/editor/preferences-popover.tsx`,
  `features/terminal/terminal-settings.tsx`,
  `features/agents/agent-integration-settings.tsx` (+ `agent-notification-setting.tsx`)
- `docs/tasks/JSDOM-VITEST-component-tests.md` — when a `.dom.test.tsx` is warranted

## Allowed scope

- `apps/desktop/src/features/settings/` (new)
- `apps/desktop/src/app.css` — a `.settings*` block, **appended**; tokens only,
  no colour literals. Shared magnet carrying two other tickets' hunks: stage
  hunks, not files.
- The three popover components, to the extent they are re-parented rather than
  rewritten.

## Explicit non-goals

- **Mounting the tab** — `App.tsx`, `editor-pane.tsx`, `tab-strip.tsx` are
  T120-M's, and touching them here is what the split exists to prevent.
- The theme plumbing (T120-T). This ticket renders `workbench.theme` as an
  ordinary enum control; making the application obey it is the other ticket.
- A shortcut registry, a command palette, remappable keybindings — all recorded
  as follow-ups, none built here (rule 14).
- Inventing settings the registry does not declare.

## Required behavior

1. **`settings-model.ts`, pure.** Schema → sections → a widget kind per setting
   (`toggle | number | select | text | list | custom`); modified-vs-default;
   search over key, label and description. A discriminated union says *why* a
   control is unavailable — `ready | readonly{reason} | absent` — in
   `run-console-model.ts`'s shape. Every disabled state has a named sentence;
   none is left to a default nobody chose.
2. **`settings-page.tsx`.** Category rail left, settings right, a search box, a
   modified mark with reset per row, and the `applies` hint rendered where a
   setting does not take effect immediately. Renders as a sibling inside
   `.editor__surfaces`, leaving Monaco `hidden` rather than unmounted, exactly
   as the consoles do.
3. **`settings-tab.ts`.** The singleton tab's open/preview/active state, reusing
   `agent-tabs-reducer.ts`'s key idea with a literal `settings` key rather than
   a third tab mechanism. Keeps its `documents` action (a file was clicked ⇒
   hand the pane back) and its close semantics. **Not persisted**, per ADR-026.
4. **Absorbing the popovers — building their replacements, not deleting them.**
   Editor preferences are pure schema and become ordinary rows. The two that
   are not pure schema keep their components, re-parented into the page as
   `custom` sections rather than rewritten: terminal `customShells` has
   add/remove editing with path validation (reuse the already-pure, already-
   tested `addCustomShell`), and agent integration has a propose-then-apply
   flow with an `ErrorEnvelope`.

   **The old popovers are deleted by T120-M, not here.** Deleting them before
   the tab is reachable would leave a build in which the editor preferences
   cannot be opened at all, and their mount points are the contended files
   anyway.

   One thing that must not be lost in the move: the agent integration control
   reads on an explicit `<details>` toggle today, which is ADR-018 §1 — Gritty
   must not touch another tool's configuration as a side effect of running. A
   section on a page is visible by default, so reading on mount would make it
   exactly that side effect. Keep an explicit *Check* button.
5. The settings file's absolute path is shown as selectable text, beside a
   Reveal button on `settings.reveal` (T120-C).
6. A refusal from the core is rendered through `ErrorEnvelope`, never swallowed.
7. **The one-time seeding**, as a pure planner plus a thin caller. ADR-031 §8
   puts it here rather than in the core: the blobs are UI-owned and opaque to
   the core (ADR-014), so the UI is what is entitled to translate them. It
   needs no migration flag — a key already reported as explicitly set is never
   seeded again — and a value equal to the schema default is **skipped**, or a
   user who never chose a font size is pinned to today's default forever. Seed
   nothing when the restore returned no blob.
8. **Delete the duplicated bounds** rather than keeping them in step by hand:
   `MAX_CUSTOM_SHELLS`, `TERMINAL_FONT_SIZE_RANGE`, `MIN_FONT_SIZE` /
   `MAX_FONT_SIZE` and `TAB_SIZES` all become schema-derived. A bound that
   disagrees with Rust by one lets the page offer a value the core refuses,
   failing the whole patch with an error naming a field the user did not touch.
9. **A number input commits on blur or Enter**, not per keystroke, with the
   draft reverting to the stored value on a refusal. A fully controlled input
   that refuses to update cannot be typed into; this is the one interaction
   that genuinely needs the jsdom project.

## Failure behavior

- The layer unavailable (no data directory) → the page renders read-only over
  defaults and says why, rather than showing controls that would do nothing.
- An unparseable file → read-only over defaults, the parse position shown, and
  the path named so the user can go fix it.
- A rejected `set` → the control returns to its previous value and the envelope
  is rendered. No optimistic state survives a refusal.
- A setting the build does not know how to render → shown as read-only with its
  raw value rather than dropped, the way `run-console-model.ts` renders unknown
  feed variants.

## Acceptance criteria

- Every setting the registry declares gets a working control without
  per-setting UI code.
- Search filters on key, label and description.
- Reset returns a setting to its default and clears the modified mark.
- Every control the three popovers offer has a working equivalent on the page
  (their deletion is T120-M's).
- No colour literal enters `app.css`.
- `pnpm check` green.

## Tests required

- `settings-model.test.ts` — pure: widget kind per type, modified computation,
  search matching, and the totality of the availability union.
- `settings-page-render.test.ts` — `renderToStaticMarkup`, house style.
- `settings-page.dom.test.tsx` — only what markup cannot assert: toggling,
  reset, search typing, keyboard movement through the rail.
- A regression test that a rejected `set` restores the previous value.
- `settings-seed.test.ts` — pure: an already-explicit key is skipped, a
  default-valued field is skipped, an invalid old value is skipped, and an
  absent blob seeds nothing.

## Commands to run

```
pnpm check
```

## Completion report

*(filled on completion)*
