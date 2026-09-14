# Task T120-U — The settings page, and retiring the three popovers

- **Phase:** 2
- **Status:** review
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
- `docs/tasks/completed/JSDOM-VITEST-component-tests.md` — when a `.dom.test.tsx` is warranted

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

### Summary

`apps/desktop/src/features/settings/` is a schema-driven settings page: a
category rail, a search box, a modified mark with a per-row reset, and the
three popovers' controls rebuilt on it. It draws whatever `settings.describe`
declares — adding a setting to the Rust registry needs no line here.

New files only, as the split required. The mount (`App.tsx`, `editor-pane.tsx`,
`tab-strip.tsx`) is untouched and remains T120-M's, and the three popovers are
left working where they are: T120-M deletes them once the tab is reachable.

### Files changed

| File | What |
|---|---|
| `features/settings/settings-model.ts` (new) | pure: schema → sections → rows, widget per kind, search, the availability union |
| `features/settings/settings-seed.ts` (new) | pure: the one-time blob → `settings.json` planner (ADR-031 §8) |
| `features/settings/settings-tab.ts` (new) | the singleton tab's open/preview/active state |
| `features/settings/use-settings.ts` (new) | the controller: `describe`/`get`, `SettingsChanged`, writes, the seed caller |
| `features/settings/settings-page.tsx` (new) | the page |
| `features/settings/settings-fields.tsx` (new) | the draft box, the shell list, the notification switch |
| `features/settings/agent-integration-section.tsx` (new) | the hook flow, re-parented off the popover |
| `features/settings/*.test.ts(x)` (new, 5 files) | 87 tests |
| `apps/desktop/src/app.css` | a `.settings*` block, appended; tokens only, no colour literal |

### Tests actually executed

- `npx vitest run src/features/settings/` → **87 passed** (5 files).
- `pnpm check` → **exit 0**: Biome, tsc, 1209 vitest tests across 113 files,
  `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`,
  generated types fresh.

### Architectural decisions

1. **`agents.notifications` is a `custom` widget, not a schema toggle.** A plain
   `settings.set` writes the file and arms nothing until the next launch —
   `arm_notifications` is only called from the composition root at startup — so
   the registry's `applies: LIVE` would have been a lie. The control calls
   `agents.setNotifications` first (the one call in Gritty that can raise a
   notification permission prompt, and it must come from the control the user
   touched, T223 §2) and persists what the platform actually did, not what was
   asked. Persisting `true` after a refusal would leave a modified mark on a
   setting that is not in force.
2. **`absent` in the availability union means the row is not drawn**, and it has
   exactly one cause: `AgentNotificationSettings.supported === false`, which the
   protocol defines as the setting being absent rather than broken. Everything
   else — unwritable storage, a kind this build cannot draw — is
   `readonly{reason}` and is drawn with its value. A `raw` widget carries the
   unrenderable case so the failure policy's "shown read-only rather than
   dropped" holds.
3. **The tab is `settings`, a literal, and never a `ConsoleKey`.** ADR-026's
   pattern extended rather than a third tab mechanism; `parseConsoleKey` returns
   `null` for it, which is correct — there is no id inside it.
4. **The integration flow keeps an explicit Check button.** In the popover the
   read was the `<details>` toggle; a section on a page is visible by default,
   so reading on mount would make opening settings silently open
   `~/.claude/settings.json` — precisely the side effect ADR-018 §1 forbids.
   Install/Uninstall are additionally gated on a status having been read, which
   the popover did not do: without the auto-read there is nothing on screen to
   act on.
5. **`DraftField` stays dirty across the in-flight write.** Clearing the flag on
   the way into the commit let the sync effect put the stored value back for the
   frame between asking and being answered, so an accepted change visibly
   flickered to the old value and back. Found by the DOM test; fixed, and both
   halves are now pinned.

### Known limitations

- **The duplicated bounds are not deleted** (required behaviour 8). `MIN_FONT_SIZE`
  / `MAX_FONT_SIZE` / `TAB_SIZES` (`packages/editor/src/editor-options.ts`) and
  `MAX_CUSTOM_SHELLS` / `TERMINAL_FONT_SIZE_RANGE`
  (`features/session/persisted-layout.ts`) are outside this ticket's allowed
  scope, and both are still load-bearing: they bound `parseEditorPreferences`
  and `parseLayout`, which parse the persisted blobs synchronously at restore,
  where the schema (fetched over `settings.describe`) is not available. ADR-031
  §8 keeps those blobs written as a mirror for one release anyway.

  What the requirement was *for* is delivered: **the page carries no copy of any
  bound**. Every `min`, `max` and `max_len` it enforces comes off the wire, so
  it cannot offer a value the core refuses. Recorded as a follow-up below,
  gated on the same release the mirrors are.
- The rail filters to one category at a time rather than scroll-spying a single
  long list. A search overrides the selection when it empties the chosen
  category.
- `addCustomShell`'s absolute-path test is POSIX-only (`startsWith('/')`); reused
  unchanged, as required. The seed planner's own check accepts a Windows root
  too, so a Windows layout blob still seeds.
- The panel is not a tab stop (`noNoninteractiveTabindex`). Every panel today
  holds focusable controls, so this only bites a panel whose settings are all
  disabled by unwritable storage.

### Follow-up tasks

1. **Delete the duplicated bounds** once ADR-031 §8's legacy mirrors are
   removed: `MIN_FONT_SIZE`, `MAX_FONT_SIZE`, `TAB_SIZES`, `MAX_CUSTOM_SHELLS`,
   `TERMINAL_FONT_SIZE_RANGE`, and the parsers that need them.
2. **T120-M must repoint the quick-open command.** `App.tsx:262` currently has
   `openSettings: () => void core.settings.reveal()` — the palette entry opens
   the *file manager*, not the page. It should open the tab.
3. A shortcut registry. ⌘, will be the eighth hand-gated `keydown` listener
   (recorded by T120-M, not built here).
4. `explorer.showHidden` is a setting by ADR-031 §7's rule and is still in the
   layout blob.
5. Reset-all. `SettingsSnapshot.modified` is documented as "what reset all would
   change" and `settings.reset` with no keys already does it; no control offers
   it, deliberately (rule 14).

### Remaining risks

- The seed runs once, unattended, and is the only code that writes a user's
  existing preferences into a new file. It is pure and covered by 12 tests, but
  it has not been exercised against a real pre-T120 session database — only
  against blobs shaped like `serializeLayout` / `serializeEditorPreferences`
  write them.
- `acceptable()` in the seed planner mirrors `document.rs::validate`. A
  disagreement costs a skipped seed rather than a bad value, but the two are
  kept in step by hand.
- The page has not been exercised in the running app: it is unreachable until
  T120-M mounts it. That is the split working as intended, and it is the first
  thing T120-M should do.
