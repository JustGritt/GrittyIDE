# Task T232 — The timeline view, and the files an agent touched

- **Phase:** 3
- **Status:** not yet started
- **Depends on:** T030 (documents), T230 (timeline), T231 (rail)

## Objective

Render the selected session's feed inside the rail — prompts, replies, tool
calls, results — and list the files it has touched, so that clicking one opens
it in the editor next to the agent that changed it.

That last step is the point of the whole rail. A multiplexer can show you an
agent's output; only an IDE can put you in the file it just edited.

## Dependencies

- **T230** for `AgentTimelineEntry`, `agents.openTimeline` and
  `AgentTimelineAppended`, including the `FileTouched` entry that already
  carries a resolved `WorkspacePath` when the file is inside the open workspace.
- **T231** for the rail, the selection and the body this renders into.
- **T030** for `documents.open` and `documents.status`.

## Required reading

- `docs/architecture/documents.md` — external modification states, and that the
  check is **on demand** today
- `docs/architecture/agent-sessions.md`, `docs/architecture/security.md`
- `apps/desktop/src/features/documents/documents-reducer.ts` — `DocumentTab`,
  `findTabByPath`, and how an open document is reused
- `apps/desktop/src/features/editor/editor-pane.tsx` — how a file becomes visible
- `apps/desktop/src/features/markdown/` — the existing precedent for rendering
  untrusted text safely

## Allowed scope

- `apps/desktop/src/features/agents/**`
- `apps/desktop/src/{App.tsx,app.css}` (wiring the open-file callback only)
- `docs/architecture/agent-sessions.md`, `docs/tasks/README.md`, this file

## Explicit non-goals

- **No diff view.** The before-copies exist — `file-history-delta.backup`
  names one and it is on disk under `~/.claude/file-history/<session>/<hash>@v<n>`
  — but reading them means reading outside the workspace root, which is a
  boundary decision with its own security argument. Record it as a follow-up,
  with the observation that ADR-007's `ChangeSet` may be the right owner rather
  than this rail.
- **No editing, no reverting, no restoring a backup.**
- **No control of the session.** No prompting, answering or approving.
- **No transcript export, no "open the raw JSONL".** The rail shows a bounded
  view, not a file viewer.
- **No Markdown or HTML rendering of agent output.** See §7.
- **No Rust changes.** If the view needs a field the core does not send, that is
  a T230 gap to record, not to work around here.

## Required behavior

### The feed

1. Entries render newest-last, appended as `AgentTimelineAppended` arrives, with
   the list pinned to the bottom while the user is at the bottom and **not**
   yanked down when they have scrolled up to read something.
2. Each kind is visually distinct: prompt, assistant text, thinking, tool call,
   tool result, notice, and — when T221 is installed — a pending permission
   prompt, which is the one the user is meant to act on and reads as such.
3. A tool call shows its name and the core's bounded detail; its result shows
   success or failure. They are paired by tool use id so a result attaches to
   its call rather than floating.
4. Subagent entries (`sidechain`) are marked and visually subordinate. They are
   not hidden — T220's report lists dropping them as a known limitation, and
   this is where that is fixed.
5. `dropped` renders as an explicit gap ("earlier entries dropped"), never
   silently. `restarted` renders as an explicit reset. Both exist because the
   core reports its own losses instead of hiding them, and the UI must not undo
   that.
6. An empty timeline, a session with no transcript, and a session that has ended
   are three different states and read differently.

### Safety of what is rendered

7. **Every string is text.** Never `innerHTML`, never Markdown, never a link the
   view resolves, never a path anything opens by concatenation, never a command
   anything runs. Agent output and repository content are untrusted input
   (hard rule 13) and the core has already bounded every field; the renderer's
   only job is to not undo that.
8. Long single-line content wraps or scrolls inside its own container. The rail
   must never scroll horizontally as a whole.

### Files touched

9. A files-touched list for the selected session, deduplicated by path, most
   recent first, with the time it was last touched.
10. A file inside the open workspace is a button that opens it in the editor.
    A file outside it is shown as text and is **not** openable — T230 already
    sends no path for those, so this is enforced by the protocol, not by the
    renderer's restraint.
11. **Opening calls `documents.status` first.** External modification is checked
    on demand today (T070 does not exist), so a file the agent rewrote a second
    ago would otherwise open from a stale buffer, or reuse an already-open tab
    whose contents are old. Checking first turns that into the normal
    external-change flow the document contract already defines, and hard rule 7
    is satisfied because nothing is resolved silently: a dirty buffer with a
    changed file surfaces the conflict the same way it does everywhere else.
12. Opening reuses an already-open tab for that path rather than creating a
    second one, using the existing `findTabByPath`.

## Failure behavior

| Situation | Behavior |
|---|---|
| `documents.status` reports the file changed under a dirty buffer | The existing conflict flow, unchanged. This task adds no new resolution path |
| The touched file was deleted since the agent wrote it | The row stays, opening reports `NotFound` in the rail's error envelope; the list is history, not a live directory |
| The touched file is outside the open workspace | Text only, not clickable |
| `documents.open` fails for any other reason | Error envelope; the feed keeps updating |
| A very long tool result | Already bounded by the core; the view scrolls it inside its own container |
| Thousands of entries | The core's ring bounds the count; the view renders what it is given without its own unbounded growth |
| Timeline reset (`restarted`) while scrolled up | The gap is shown in place; the scroll position is not stolen |
| The session ends mid-render | The feed stops growing and says so; nothing is removed |

## Acceptance criteria

- With Claude Code working in another terminal, the rail's feed shows its
  prompts, replies and tool calls live, and pairs each result with its call.
- Scrolling up and staying there is not interrupted by new entries.
- A file the agent just edited appears in files-touched and opens in the editor
  showing the **current** contents, not a stale buffer — verified by editing a
  file in the agent while it is open in Gritty.
- A file the agent touched outside the open workspace is visible and not
  clickable.
- A dropped or restarted timeline renders an explicit gap.
- Subagent entries are visible and marked.
- No agent-supplied string is ever interpreted as markup — verified by a test
  with a fixture entry containing HTML and a path-like string.
- `pnpm check` green.

## Tests required

- **Unit (TS):** the entry → view-model mapping is total over the entry kinds,
  including one added later, so an unknown kind renders as text rather than
  vanishing.
- **Unit (TS):** tool calls pair with results by id; an unpaired call renders as
  still running.
- **Unit (TS):** files-touched dedup and ordering; an external file yields no
  openable target.
- **Unit (TS):** a fixture entry containing `<script>` and `../../etc/passwd`
  renders as text and produces no markup and no path anything opens.
- **Unit (TS):** the open-file flow calls `documents.status` before
  `documents.open`, and reuses an existing tab for the same path.
- **Unit (TS):** `dropped` and `restarted` render as gaps; the reducer does not
  splice around them.
- **Manual:** with a file open in Gritty, have Claude Code edit it in another
  terminal, then open it from files-touched and confirm the contents are current
  and any dirty-buffer conflict surfaces normally.

## Commands to run

```
pnpm check
```

## Completion report

_(filled in by the implementing session — including the diff-view follow-up
from the non-goals, with a recommendation on whether it belongs here or on
ADR-007's `ChangeSet`)_
