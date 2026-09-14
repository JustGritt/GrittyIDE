# Task T120-M — Mount the settings tab, and the comma chord

- **Phase:** 2
- **Status:** review
- **Depends on:** T120-U (the page), **T310-U** (the files), T101-D (queue order)

## Objective

Wire the settings page built by T120-U into the shell: construct its controller
in `App.tsx`, render it as a surface in `editor-pane.tsx`, give it a tab group
in `tab-strip.tsx`, and open it with ⌘,. Roughly thirty lines across three
files, and nothing else.

## Why this is its own ticket

Its three mount points all carry T310-U's uncommitted, reviewed-but-unlanded
work — the same wall T101-D is behind:

> Its three mount points — `editor-pane.tsx`, `tab-strip.tsx` and `App.tsx` —
> all carry T310-U's uncommitted work (at review, session ended). Building
> there now would entangle two tickets in one diff.

Splitting the mount out of T120-U is what keeps the whole settings surface —
the ADR, the contract, the crate, the page and the theme — buildable now. This
ticket is the only part that waits, and it is deliberately small enough to
rebase by hand against whatever T310-U and T101-D leave in those files.

## Unblocks when

T310-U lands. Then it queues behind T101-D, which reached the same files first.

## Required reading

- `docs/decisions/ADR-026-agent-console-tab.md` — the pattern being extended to
  a second kind of non-document tab, which is its stated "Revisit when"
- `apps/desktop/src/features/editor/editor-pane.tsx` — the `.editor__surfaces`
  block and the `showConsole` gating
- `apps/desktop/src/features/editor/tab-strip.tsx:23-45` — the two load-bearing
  rules: a non-file tab is a second source, never a fake `DocumentTab`, and the
  groups are never interleaved
- `apps/desktop/src/App.tsx` — where `useAgentConsole` is constructed and where
  the status-bar facts are built

## Allowed scope

- `apps/desktop/src/App.tsx`
- `apps/desktop/src/features/editor/editor-pane.tsx`
- `apps/desktop/src/features/editor/tab-strip.tsx`
- `apps/desktop/src/features/settings/` — only what mounting requires

All three are shared magnets carrying other tickets' work. **Stage hunks, not
files**, and check `git diff --cached` before committing.

## Explicit non-goals

- Any change to the settings page itself — that is T120-U's, and a change here
  means the split failed.
- **A shortcut registry.** ⌘, is the *eighth* hand-gated `keydown` listener and
  four task files already apologise for there being no registry. It is real
  debt and a real ticket; bundling it here doubles this one (rule 14). Record
  it as a follow-up, again.

## Required behavior

1. The settings controller is constructed in `App.tsx` beside `useAgentConsole`.
2. `editor-pane.tsx` renders the page as a sibling inside `.editor__surfaces`,
   leaving Monaco `hidden` rather than unmounted. The existing suppressions that
   fire while a console shows — the markdown toolbar, the preferences popover,
   the banners, the empty state — must fire for the settings tab too; generalise
   `showConsole` rather than adding a parallel condition beside it.
3. `tab-strip.tsx` gets the settings tab as a group after the agent group,
   never interleaved with documents, keeping the rule that a document tab is not
   active while a non-document tab shows.
4. ⌘, opens the tab (⌃, on Linux — derive it from the core's reported OS with
   `acceleratorForOs()`, as the existing chords do). Opening it twice focuses
   the existing tab rather than opening a second.
5. The status bar does not describe a file the user is not looking at while the
   settings tab shows, matching what `consoleFacts` already does for consoles.

## Failure behavior

- The settings layer unavailable → the tab still opens and the page explains
  why. A control that would do nothing is never rendered as if it would.
- Closing the tab hands the pane back to the documents without guessing a tab,
  exactly as closing a console does.

## Acceptance criteria

- ⌘, opens the settings tab; it closes like a console; a second ⌘, focuses
  rather than duplicates.
- Clicking a file hands the pane back.
- No document tab is active while the settings tab shows.
- The tab is not persisted across a restart (ADR-026).
- `pnpm check` green.
- The diff touches only the three files plus the settings feature directory,
  and sweeps in none of T310-U's or T101-D's hunks.

## Tests required

- A tab-strip render test covering the third group and the ordering rule.
- A `.dom.test.tsx` for the chord and for focus-rather-than-duplicate.
- The existing agent-console tab tests still pass unchanged — the generalisation
  in `editor-pane.tsx` must not alter console behaviour.

## Commands to run

```
pnpm check
pnpm dev
```

## Completion report

### Summary

The settings page is reachable: ⌘, / Ctrl+, opens it as a tab, it renders as a
surface inside `.editor__surfaces` with Monaco `hidden` beside it, it closes
like a console, and no document tab is active while it shows. Roughly the
thirty lines the ticket forecast, across the three files it named.

**One thing the ticket asked for is deliberately not done: the three popovers
are still there.** See "The popovers stay" below — it is the most important
paragraph in this report.

### Files changed

| File | What |
|---|---|
| `App.tsx` | `useSettings`, the tab reducer, `showSettings`, the chord, the surface prop, the status-bar facts |
| `features/editor/editor-pane.tsx` | `showSurface` generalisation, the page as a surface, the strip's third group, the hand-backs |
| `features/editor/tab-strip.tsx` | the settings group after the consoles, and the document-active rule widened |
| `features/settings/settings-chord.ts` (new) | pure: ⌘, / Ctrl+, |
| `features/settings/use-settings-chord.ts` (new) | the window listener |
| `features/settings/settings-chord.test.ts` (new) | 6 tests |
| `features/settings/settings-mount.dom.test.tsx` (new) | 9 tests: the chord, focus-not-duplicate, the textarea case |
| `features/editor/tab-strip-render.test.tsx` (new) | 11 tests: the three groups, ordering, what is active |

### Tests actually executed

- `npx vitest run src/features/settings/ src/features/editor/` → **124 passed**
  (10 files). The existing agent-console tab tests pass unchanged.
- `pnpm check` → **exit 0**: Biome, tsc, 1259 vitest tests across 118 files,
  `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`,
  generated types fresh.
- `npx vite build` → exit 0. Run deliberately: `App.tsx` now imports the
  settings feature statically, and `monaco-deferred.test.ts`'s property is that
  nothing reaches Monaco that way. Monaco is still its own chunk.

### The popovers stay, and this is the finding

T120-U's task file says "the old popovers are deleted by T120-M". **They are
not, and deleting them would have shipped a regression.**

Nothing applies the editor and terminal settings yet. `workbench.theme` is
applied (T120-T) and `agents.notifications` is armed by the core at startup
(T120-R). The other eight — `editor.fontSize`, `editor.tabSize`,
`editor.wordWrap`, `editor.minimap`, `terminal.shell`,
`terminal.customShells`, `terminal.fontFamily`, `terminal.fontSize` — are read
by **nothing** outside `features/settings/` (checked by grep). Monaco still
takes its preferences from the `preferences` blob through
`useEditorPreferences`, and the terminal still takes its font and shells from
the `layout` blob.

So today the popovers are the only controls that *do* anything for those eight,
and the settings page writes a file no one reads them from. Deleting the
popovers would have left a build in which font size cannot be changed at all.

Two further reasons not to force it: their mount points (`terminal-panel.tsx`,
the agents section) are outside **both** T120-U's and T120-M's allowed scope, so
neither ticket could have removed them cleanly; and this ticket's own acceptance
criterion says the diff touches only the three files plus the settings feature
directory.

The missing piece is a ticket nobody has written: **make the editor and the
terminal obey their settings.** It is what turns the page from a writer into a
control surface, and it is what makes deleting the popovers correct rather than
destructive. Until it lands the page is honest but partly inert, and that should
be weighed before this reaches a user.

### Architectural decisions

1. **`showConsole` became `showSurface`, as §2 asked.** Every suppression in the
   pane asked "is a console showing"; what each meant is "does something other
   than a document have the pane". All eight sites moved to the general
   predicate rather than gaining `&& !showSettings`, which is how the third kind
   of surface gets missed.
2. **The chord does not yield to `ownsRawKeystrokes`**, and that is the one way
   it differs from `useQuickOpenChord`. That listener stands aside for a
   `<textarea>` because on Linux and Windows `Ctrl+P` is readline's "previous
   command". `Comma` is not a readline binding, and "I am editing a file and I
   press ⌘," is the case this chord exists for — yielding would cost exactly it
   and buy nothing. Quick open needed a second registration inside Monaco to
   solve the same problem; this does not.
3. **The status bar rides the existing `agentConsole` prop.** That slot already
   means "what the non-document surface says"; widening its name is a change to
   `status-bar.tsx`, which this ticket may not touch. While settings shows, the
   bar reads `Settings · N changed` rather than describing a file the user is
   not looking at (§5). Recorded as a rename follow-up.
4. **Opening settings and activating a console are the same event to each
   other.** Each takes the pane and hands it away from the other; neither
   closes the other's tab. `dispatch({type:'documents'})` and
   `consoleController.showDocuments()` are the two halves.

### Known limitations

- **The one-time seeding does not run.** `useSettings` is constructed with a
  `null` seed source. The planner and its 12 tests exist (`settings-seed.ts`),
  but the raw `preferences` / `layout` blobs are parsed and discarded inside
  `restore-flow.ts`, which `use-session.ts` owns — another ticket's file.
  Seeding from the *parsed* values was considered and rejected: a parsed blob
  cannot be told apart from an absent one, and ADR-031 §8 is explicit that
  seeding a default nobody chose pins them to it forever. Skipping a migration
  is recoverable; performing it wrongly is not, because a key once written is
  reported `explicit` and never seeded again.
- `pnpm dev` was not run — see the risks below.

### Follow-up tasks

1. **Apply the settings** — the blocking one. `editor.*` into
   `useEditorPreferences` / Monaco, `terminal.*` into `use-terminals` and
   `terminal-view`. Only then can the popovers be deleted, which is the second
   half of the same ticket and touches `terminal-panel.tsx` and the agents
   section.
2. **Wire the seed**: capture the raw blobs where `restore-flow.ts` currently
   parses and drops them, and hand them to `useSettings`.
3. **Repoint the quick-open command.** `App.tsx`'s `openSettings` still calls
   `core.settings.reveal()` — the palette entry opens the file manager rather
   than the tab. Left alone here because it is a behaviour change to an existing
   command rather than part of the mount; it should now open the tab.
4. Rename `StatusBar`'s `agentConsole` prop to something that admits it has two
   kinds of caller.
5. **A shortcut registry.** ⌘, is the eighth hand-gated `keydown` listener and
   the sixth file to apologise for there being no registry.

### Remaining risks

- **The manual pass was not run.** `pnpm dev` needs a window and eyes on it, and
  I have neither. Everything asserted here is asserted by a test or a build: the
  chord opens the tab and a second press does not duplicate it (jsdom), the
  three groups draw in order and only one tab is selected (static markup), the
  pane's suppressions all moved to one predicate (typecheck plus the unchanged
  console tests), the bundle builds. What is **not** verified is how it looks
  and feels in the real window — the surface filling the pane, Monaco hidden
  rather than unmounted, and T120-T's theme actually flipping three surfaces,
  which is the pass that ticket also owes and which is only now performable,
  because the picker exists.
- Another session is editing `docs/decisions/ADR-032-ui-framework.md` and
  `docs/testing/phase-0-manual-pass.md` in this working tree. Neither is touched
  here; noted so whoever commits this does not sweep them in.
