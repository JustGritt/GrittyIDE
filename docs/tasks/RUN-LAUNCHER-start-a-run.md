# Task RUN-LAUNCHER — Start a Claude run from a release build

- **Phase:** M0 (ADR-035: demoable)
- **Status:** doing
- **Depends on:** T310-B (`runs.start` for Claude Code), T310-U (the run console)

## Objective

A release build of Gritty has no way to start an agent. The only control that
calls `runs.start` is `StartClaudeForm` in `features/runs/runs-panel.tsx`,
mounted under `import.meta.env.DEV` and labelled "Agent runs (development)";
it also refuses to render until a workstream exists, which a new user has no
reason to have made.

This task gives the sidebar's Agents section a **New run** control: type what
Claude should do, pick a workstream or let one be made from the prompt, start
the run, and land in its console. It is the first step of ADR-035's core loop
that a person can walk without the dev panel.

**Renderer only.** No command, event or type changes; `runs.start` and
`workstreams.create` already exist.

## Dependencies

All present on `main`:

- `runs.start` (`StartRunRequest`: workspace, workstream, provider, optional
  prompt of at most 16 KiB) and `RunsController.startClaude` in
  `apps/desktop/src/features/runs/use-runs.ts`.
- `workstreams.create` (title at most 200 characters) and
  `WorkstreamsController.create` in `features/workstreams/use-workstreams.ts`.
- `AgentConsoleController.show` / `keep` and `runConsoleKey` in
  `features/agents/`.

## Required reading

- ADR-035 (why this is M0), ADR-026 (agents live in the sidebar, the console is
  a tab), ADR-020 (a run's permission asks are answered in Gritty).
- `docs/architecture/agent-runs.md`.
- `features/agents/agents-section.tsx`, `features/runs/run-console.tsx` (the
  prompt box this form's keys follow), `features/runs/runs-panel.tsx`.

## Allowed scope

- `apps/desktop/src/features/runs/run-launcher.tsx` (new),
  `run-launcher-model.ts` and its tests (new)
- `apps/desktop/src/features/runs/use-runs.ts` (`startClaude` resolves with the
  run)
- `apps/desktop/src/features/workstreams/use-workstreams.ts` (`create` resolves
  with the workstream)
- `apps/desktop/src/features/agents/agents-section.tsx` and its render test
- `apps/desktop/src/App.tsx` (wiring only)
- `apps/desktop/src/app.css` (the launcher's rules only)
- `docs/architecture/agent-runs.md`, this file

## Explicit non-goals

- Providers other than Claude Code (T222-SEAM, T222-CODEX), and launching into
  a terminal (AGENT-TERMINAL).
- Worktrees or branches (T400-R, T400).
- Moving the workstream through its states when a run starts (T300-PANEL).
- Deleting the dev panel: it still starts fake runs.
- A quick-open command or a global chord for the launcher.

## Required behavior

1. When the Agents section is open, its bar offers **New run** beside Rescan.
   It toggles a form at the top of the section body.
2. The form has a multi-line prompt (required, at most 16 KiB, `maxLength`) and
   a workstream choice whose default is **New workstream from this prompt**,
   followed by every workstream in the workspace.
3. A line under the fields says what starting does, in words: it runs Claude
   Code in this workspace on the user's own account, and tools its settings do
   not already allow will ask here first.
4. **Start** (or ⌘↵ / Ctrl↵ in the prompt) starts the run. Escape closes the
   form. Start is disabled while the prompt is blank or a command is in flight.
5. With "new workstream", the workstream is created first, titled from the
   prompt's first non-blank line, whitespace collapsed, clipped to 80
   characters with an ellipsis.
6. On success the form clears and closes, and the run's console opens as a
   **kept** tab (not a preview a stray click would replace).
7. Nothing optimistic: the run's row and console appear because the core said
   the run exists.

## Failure behavior

- `workstreams.create` refused: nothing is started; the form stays open with
  what was typed and shows the refusal.
- `runs.start` refused after a workstream was created (no `claude` on `PATH`,
  an oversized prompt): the form stays open, shows the refusal, and its
  workstream choice **switches to the workstream just created**, so a retry
  does not create a second one.
- The workspace closes while the form is open: the section unmounts with it;
  nothing is started.

## Acceptance criteria

- A release build (`import.meta.env.DEV` false) can start a Claude run from the
  Agents section with no workstream created beforehand.
- The console of the started run is open and kept afterwards.
- Both refusal paths above leave the prompt in place and create at most one
  workstream across retries.

## Tests required

- Model: title derivation (first non-blank line, collapsing, clipping, a prompt
  of only whitespace), `canLaunch`.
- Render: the bar offers New run only when open; the form renders its fields,
  the account line and a disabled Start for a blank prompt; the empty-state
  text mentions New run.
- DOM: a successful launch with a new workstream calls create, then start with
  its id, then opens and keeps the console; a start refusal leaves the prompt,
  shows the envelope and selects the created workstream; a retry does not call
  create again.

## Commands to run

`pnpm check`.

## Completion report

**Summary.** The Agents section's bar offers **New run**, which opens
`RunLauncher` at the top of the section body. A prompt, a "belongs to" choice
(new workstream from the prompt by default, then every workstream not `DONE` or
`ABANDONED`), a line saying what starting spends and where it acts, Start and
Cancel. `launchRun` runs `workstreams.create` then `runs.start`; on success the
run's console opens as a kept tab and the form closes. Committed as `a581fdb`.

**Files changed.**
- New: `features/runs/run-launcher.tsx`, `run-launcher-model.ts`,
  `run-launcher-model.test.ts`, `run-launcher.dom.test.tsx`.
- `features/runs/use-runs.ts`: `startClaude` resolves with the run.
- `features/workstreams/use-workstreams.ts`: `create` resolves with the
  workstream.
- `features/agents/agents-section.tsx`: `launcher` and `onLauncherToggle`
  props, New run on the bar, the empty text points at it, ⌥G ignores keys typed
  in the section's own textarea or select.
- `features/agents/agents-section-render.test.ts`: five cases for the above.
- `features/runs/run-console.dom.test.tsx`: its stub `startClaude` returns
  `undefined` for the new signature (claimed separately; one line).
- `App.tsx`: wiring, keyed by workspace.
- `docs/architecture/agent-runs.md`: "Starting a run in a release build".

**Tests actually executed.**
- `vitest run` on `features/runs` and the section render test: 9 files, all
  passed, before and after the formatting fixes.
- `pnpm check`: **exit 0**. 138 test files, 1,578 TS tests passed. Every cargo
  test binary reported ok. The generated types were fresh. It ran over a
  working tree that also held T099-S's uncommitted terminal work.

**Architectural decisions.**
- No new command. The launch is two existing commands with no transaction. A
  refused start after a created workstream leaves the workstream (it is real
  and visible on the island) and the form's retry targets it.
- The launcher takes a narrowed `LaunchDeps` rather than both controllers, as
  `ConsoleRuns` does.
- The section takes the form as an element, as the explorer takes Changes, so
  it holds no controller.

**Known limitations.**
- **Not exercised in `pnpm dev`**: no session here has a screen. The feature
  check is Alexis's: open a workspace with no workstreams, New run, type a
  prompt, ⌘↵, and see the console open kept and the run's permission asks
  arrive.
- **No launcher CSS yet.** `app.css` is leased by T099-S (split terminals). The
  form borrows `.agent-prompt`'s rules, so until then it draws a top border
  where a bottom one belongs, a select at its natural width, and a note with
  default paragraph margins.
- ⌥A (island) still fires from the launcher's textarea, as it does from the
  run console's; `isTypingInField` tests `INPUT` only, on purpose, for Monaco.
- The workstream stays in whatever state it was in; nothing moves it to
  `IN_PROGRESS` (T300-PANEL, deferred by ADR-035).
- `maxLength` counts UTF-16 units and the core counts characters, so the field
  is stricter than the core for text outside the BMP, never looser.

**Follow-up tasks.**
- The launcher's three CSS rules once `app.css` is free (this ticket).
- A quick-open `>` command, "Start agent run".
- AGENT-TERMINAL (launch an agent CLI into a terminal in a task's worktree)
  replaces this form's target with a task once T400 (a workstream owns a
  worktree and a branch) lands.
