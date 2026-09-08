# ADR-026: An agent console is a tab in the editor, and the agent rail is retired

- **Status:** Accepted (implemented by T233-C)
- **Date:** 2026-09-07
- **Supersedes:** T231's "No agent entries in the document tab strip", and the
  third shell region ADR-016's layout notes assume

## Context

T231 put the machine's agent sessions in a third region of the shell, a
right-anchored rail beside the editor, and T232 filled it with the timeline.
The rail's own task file states the decision this ADR reverses:

> **It is deliberately not a tab.** Agent sessions and open files are different
> things and mixing them in one strip makes both harder to read. Keeping them
> apart also avoids the only expensive part of the alternative: `SessionTab.path`
> is a required `WorkspacePath` backed by a non-nullable SQLite column
> (`crates/persistence/src/store.rs`, `TabRow`), so a non-document tab would need
> a wire change, a ts-rs regeneration and a migration.

Two things happened after that.

**T233-S put the same roster in the left sidebar.** Artboard 12a's body has two
columns — a 300px sidebar and the main area — and no rail; the agents moved to a
pinned section above the file tree over design turns 10 and 11. With T233-S
shipped and the rail still present, the shell listed the same sessions twice, in
two places, with two different counts.

**The rail's width was coming out of the editor.** A feed of tool calls,
diffs and command output does not fit a 320px column, and the column was taken
from the pane beside it: a markdown file in Split view got the middle third of
the window, source and preview both.

## Decision

**An agent console is a tab in the document tab strip, and the rail is deleted.**

Selecting an agent opens a console as a tab beside the open files, with exactly
a file's semantics (T041): a single click opens a preview tab the next single
click replaces, and a double click keeps it. While a console is the active tab
it fills the editor area.

**Agent tabs are never persisted, and that is what makes the rest free.**

This is the half of T231's reasoning that was a real cost rather than a
judgement, and it does not apply, because a console never enters
`documents.state`:

- `buildSessionTabs` (`persisted-layout.ts`) is the **only** producer of
  `SessionTab`. It walks `documents.state.order`, and additionally drops any id
  that does not resolve to a `DocumentTab` of the current workspace.
- `request.tabs` is assigned in exactly one place, `session-writer.ts`.
- `tabs::replace` has exactly one non-test caller,
  `crates/gritty-core/src/session/mod.rs`.

There is no code path from an agent tab to a `TabRow`. **No wire change, no
ts-rs regeneration, no migration.** `persisted-layout.test.ts` carries the guard
that fails if this stops being true.

Not persisting them is independently right, and T231 argued it itself for the
rail's selection: a session id is ephemeral and rediscovered each launch, so
restoring one that no longer exists is worse than restoring nothing.

Three consequences of the shape follow:

- **`documents.state.activeId` is never cleared while a console shows.** The
  console's active id lives in its own reducer. So the last active *document* is
  still flagged `active: true` by `buildSessionTabs`, and quitting with a
  console on screen reopens the file you were on — rather than falling through
  to the leftmost tab, which is what clearing it would have silently done.
- **Agent tabs are grouped after the document tabs, never interleaved.** That
  is the legible half of T231's objection kept: one strip, two kinds, still
  told apart at a glance. Drag-reorder stays a document affair.
- **The close flow is untouched.** `planClose` skips ids with no `DocumentTab`,
  so routing a console through it would have made its `×` silently do nothing.
  A console has nothing to save and nothing to ask about, so it gets its own
  close path.

## What moved rather than died

Deleting the rail would have taken four things with it that live nowhere else.
Each was moved deliberately, because none of them is about being a column:

- **`activitySourceLabel` (T221 §24).** A *required behaviour* — "a row can say
  where its verdict came from, so a `BLOCKED` row is attributable rather than
  magic" — whose only caller in the app was the rail's row, and whose only test
  was the rail's render test. The console states it in words; the sidebar row
  carries it in its status tooltip; the assertions moved to
  `agent-console-render.test.ts`.
- **The `processEvidence` note.** The one sentence explaining why every row
  reads as `Ended` when the process table cannot be read. It moved to the
  sidebar section.
- **The roster's error envelope.** Without it a failed `agents.list` or
  `agents.refresh` would have become silent.
- **Sessions running outside the open workspace.** The rail was machine-wide and
  was the only *list* of them. `rosterRows` now flags rather than filters, and
  the sidebar keeps them behind one reveal — hidden by default is fine, gone is
  not.

The timeline itself (T232) is reused verbatim. Its four load-bearing rules — the
feed accumulates a delta keyed on `seq`, `dropped` and `restarted` are drawn
rather than repaired, a tool result attaches to its call by `toolUseId`, and
opening a touched file calls `documents.status` first — are unchanged and were
never the rail's.

The follower lifecycle is likewise moved and not rewritten: exactly one
`openTimeline` at a time, closed before the next opens, closed when a document
takes the pane, closed when the tab closes, and closed on unmount.

## Consequences

- The editor gets the ~320px the rail was taking, which is the whole of the
  reported problem.
- ⌥G is freed and now folds the sidebar's Agents section. It is the sixth
  hand-gated `keydown` listener; the shortcut registry T335 asked for is still
  owed and is now a little more owed.
- `LAYOUT_VERSION` 6 drops `agentRail` and `panes.agentRailWidth`. Neither is
  migrated away — `parseLayout` never reads the version and every field falls
  back independently, so a blob written by the previous build simply opens.
- The shell is back to three regions: sidebar, editor, dock. The island stays
  machine-wide; the sidebar section is scoped to the open workspace; the
  attention queue is still where an answer is given.
- **A run Gritty owns still has no console.** `AgentRun` carries an
  `AgentRunEventLog`, which is a different shape from `AgentTimeline`, and the
  only provider is `FAKE`. Owned rows keep their Cancel and nothing else until
  T310 lands a real adapter worth building that view for.

## Revisit when

A second kind of non-document tab is wanted, or an agent tab needs to survive a
restart — the second would mean earning the `SessionTab` change this decision
was able to avoid.
