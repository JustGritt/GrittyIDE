# Task T120-M — Mount the settings tab, and the comma chord

- **Phase:** 2
- **Status:** todo
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

*(filled on completion)*
