# Task T310-U-FILES — Files touched, on an owned run's console

- **Phase:** 4 (orchestration)
- **Status:** todo
- **Depends on:** T310-RUN-PATHS (a run's file paths, made workspace-relative), T310-U (the prompt box, and an owned run in the console)

## Objective

An observed session's console lists the files the session wrote, above its
feed, each one a button that opens the file (T232). A run Gritty owns shows
the same writes only as feed rows ("Writing …", "Wrote …"): there is no list,
and nothing opens. T310-U's report filed it — "the files-touched list on a
run's console, from `FILE_WRITTEN` paths, clickable through the same
`openTouchedFile` path T232 built" (`docs/tasks/completed/T310-claude-code-adapter.md`).

This task gives the run console that list, reusing T232's component, model
shape and open path rather than writing a second copy.

**Why it depends on T310-RUN-PATHS.** A real Claude run reports absolute paths
that the driver does not yet classify against the workspace, so every
`FILE_WRITTEN` arrives with no openable `path`. Built before that lands, this
list would show every file as "elsewhere" for every real run and look correct
only against the fake driver.

## Dependencies

- **T310-RUN-PATHS**: `AgentRunPath.path` present for writes inside the workspace.
- **T232**:
  - `TouchedFile` and `touchedFiles` in `features/agents/agent-timeline-model.ts`
  - the private `TouchedFileList` in `features/agents/agent-timeline.tsx`
  - `openTouchedFile` in `features/agents/open-touched-file.ts`
  - `openFile`, `openingPath`, `canOpenFiles` and `error` on the console
    controller in `features/agents/use-agent-console.ts`
- **T310-U**: `RunConsole`, mounted in `features/editor/editor-pane.tsx` beside
  `AgentConsole`, where `consoleController` is already in scope.

## Required reading

- `docs/tasks/completed/T232-timeline-view.md` §9–§12 (the list, the boundary,
  `documents.status` first, tab reuse)
- ADR-010 (what a `WorkspacePath` lets the renderer open)
- `apps/desktop/src/features/runs/run-console.tsx`, `run-console-model.ts`
- `apps/desktop/src/features/agents/agent-timeline.tsx` (`TouchedFileList`),
  `agent-timeline-model.ts` (`touchedFiles`), `open-touched-file.ts`,
  `use-agent-console.ts` (`openFile`)
- `packages/protocol/src/generated/AgentRunEventRecord.ts`, `AgentRunPath.ts`

## Allowed scope

- `apps/desktop/src/features/runs/run-console.tsx`, `run-console-model.ts` and their tests
- `apps/desktop/src/features/agents/agent-timeline.tsx`: extracting
  `TouchedFileList`, no behaviour change
- `apps/desktop/src/features/agents/touched-file-list.tsx` (new, the extracted component) and a test
- `apps/desktop/src/features/editor/editor-pane.tsx`: the `RunConsole` mount only
- this file

## Explicit non-goals

- **No core or wire change.** Paths are T310-RUN-PATHS'; this is the renderer.
- **No second implementation** of the list, the dedupe or the open path. The
  runs feature already imports from `features/agents` (`PermissionPrompt`,
  `isPinnedToBottom`); one more shared component follows that precedent.
- **Not requested writes.** A `FILE_WRITE_REQUESTED` that was denied never
  becomes a `FILE_WRITTEN`, and a list of files a run *wanted* to write would
  claim changes that did not happen. Requests stay in the feed.
- No files-touched summary on `AgentRun`, and no persistence (ADR-021's
  terminal summary is its own task).
- No diff view.

## Required behavior

### Decisions taken in this spec (a reviewer may overturn them)

1. **Source: `FILE_WRITTEN` only.** Rationale above. `FILE_READ` is not a touch,
   matching T232, whose list holds only `FILE_TOUCHED`.
2. **One model shape for both consoles.** Add `runTouchedFiles(log)` to
   `run-console-model.ts`, returning T232's `TouchedFile[]`:
   - key `in:<path>` or `outside:<display>`;
   - `openPath` is the event's `path`, or `null`;
   - `at` is the record's `at` in milliseconds;
   - `seq` is the record's `seq`;
   - deduplicated, most recent first by `seq`.

   The ring's bound (`MAX_RUN_EVENTS`) is the list's horizon, as T232 accepts
   for its own.
3. **One component.** Move `TouchedFileList` out of `agent-timeline.tsx` into
   `features/agents/touched-file-list.tsx`, exported, with its current props
   and markup unchanged. `AgentTimeline` imports it back. The markup
   (`agent-files*` classes, "elsewhere", "opening…") is then identical in both
   consoles by construction.
4. **One open path, reached through the console controller.** `RunConsole` gains
   an optional `files` prop, a narrow port with `openingPath`, `canOpen`,
   `open(path)`, `error` and `clearError`. `editor-pane.tsx` passes it from
   `consoleController`, which already routes through `openTouchedFile`
   (`documents.status` first, then `openFile` quietly, reusing an open tab).
   Without the prop, the list renders as text only.

### What is drawn

5. The list sits above the run feed, inside `agent-console__timeline`, in the
   same place `AgentTimeline` puts it, and only when non-empty.
6. A file inside the workspace is a button. One outside is text marked
   "elsewhere", because the core sent no path, not because the renderer chose
   to hide one.
7. An open that fails (`NotFound`: the run deleted the file since) shows in the
   console's error envelope and leaves the list as it was.

## Failure behavior

- A run whose ring evicted early writes lists only what the ring still holds.
- A file deleted after the run wrote it: the open fails with `NotFound` through
  the envelope, as it does on the observed side.
- No workspace open (`canOpen` false): every row is text.
- A dirty buffer over a file the run changed: the ordinary external-change
  banner. Nothing is resolved silently (hard rule 7), because the open path
  already checks `documents.status` first.

## Acceptance criteria

- A run that writes `src/a.ts` twice and `/tmp/x` once shows two rows:
  `src/a.ts` (a button) above `/tmp/x` (elsewhere), in most-recent-first order.
- A denied write adds no row.
- Clicking a row opens the file in the editor, reusing its tab when it is
  already open.
- `AgentConsole`'s list renders exactly as before the extraction (its existing
  render tests pass unchanged).
- Exercised in `pnpm dev` against a real owned run once T310-RUN-PATHS has
  landed. The report says what was opened.

## Tests required

- `run-console-model.test.ts`: `runTouchedFiles` covering dedupe, order,
  inside vs outside, `FILE_WRITE_REQUESTED` and `FILE_READ` ignored.
- `run-console.dom.test.tsx`: the list renders above the feed; a click calls
  `files.open` with the `WorkspacePath`; no `files` prop means text rows; an
  open error shows in the envelope.
- `touched-file-list` test: moved from wherever the private component was
  covered; `agent-timeline` render tests unchanged.
- Mutation check, stated in the report: make `runTouchedFiles` include
  `FILE_WRITE_REQUESTED` and a test fails.

## Commands to run

```
pnpm vitest run apps/desktop/src/features/runs apps/desktop/src/features/agents
pnpm check
pnpm dev    # manual, after T310-RUN-PATHS: a run writes a file; open it from the list
```

## Completion report

_Not started._
