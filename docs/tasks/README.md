# Task index

One file per task, following `docs/agent/task-template.md`. IDs come from the
project plan (§17). Respect the dependency graph: never implement a downstream
domain abstraction before its upstream contract exists.

**The table below is generated**, and so is the `- **Status:**` line of every
task file. Both come from the live board (`scripts/agent-board.py`, see *Working
a task*); edit either by hand and the next `render --index` overwrites it. Only
the table between the markers is written — this prose, the dependency graph and
the workflow below are hand-maintained.

<!-- board:table start -->
| ID | Task | Phase | Depends on | Status |
|---|---|---|---|---|
| [T000](T000-bootstrap-workspace.md) | Bootstrap workspace | 0 | – | done |
| [T010](T010-core-protocol.md) | Core protocol + operation/event/error conventions (Gate A) | 0 | T000 | done |
| [T020](T020-workspace-open-list.md) | Workspace open/list + fixtures | 0/1A | T010 | done |
| [T030](T030-file-io-documents.md) | File I/O + document/buffer contract (Gate B) | 0/1A | T020 | done |
| [T040](T040-monaco-shell.md) | Monaco shell | 0/1A | T030 | done |
| [T041](T041-preview-tabs.md) | Preview tabs, and closing more than one at a time | 1A | T020, T030, T040 | review |
| [T042](T042-explorer-file-operations.md) | Editing the file tree: create, rename, move, delete, undo | 1A | T020, T030, T040, T070 | review |
| [T045](T045-drag-and-drop.md) | Drag and drop files/folders onto the window | 1A | T030, T040 | review |
| [T046](T046-resizable-panes.md) | Resizable panes, and a terminal on opening | 1A | T040, T060, T095 | done |
| [T047](T047-terminal-names.md) | Naming terminal sessions | 1A | T090, T095 | done |
| [T048](T048-ui-revamp.md) | Shell UI revamp from design artboard 2a | 0 | T020, T030, T040, T046, T050, T060, T090 | done |
| [T049](T049-startup-screen.md) | Startup screen from design artboard 3a | 0 | T020, T030, T048, T060, T095 | done |
| [T050](T050-markdown.md) | Markdown | 0/1B | T030, T040 | done |
| [T051](T051-launcher-resume-rail.md) | Launcher resume rail from design artboard 3b | 0/1B | T049, T060 | done |
| [T060](T060-session-persistence.md) | Session persistence + recovery | 0/1B | T030, T040 | done |
| [T090](T090-terminal-pty.md) | Terminal (PTY) spike | 0/2 | T010 | done |
| [T095](T095-terminal-profiles.md) | Terminal shell profiles and rooting | 0/2 | T090, T060 | done |
| [T096](T096-terminal-chrome.md) | Terminal chrome from design artboard 8a | 0/2 | T047, T048, T090, T095 | done |
| [T097](T097-shell-integration-blocks.md) | Shell integration (OSC 133) and command blocks | 2 | T090, T095, T096 | todo |
| [T098](T098-terminal-colours-and-rooting.md) | Terminal session colours, and starting where you work | 0/2 | T047, T090, T095, T096 | done |
| PTY-FLAKE | `pty.rs::kill_terminates_the_whole_process_tree` is flaky under load | 2 | T090 | review |
| Phase 0 tests | `docs/plans/phase-0-spike.md` sections B–D | 0 | T060, T090 | doing |
| ADR-001 | Confirm or reject Tauri | 0 | Phase 0 tests | todo |
| MANUAL-PASS-COVERAGE | The manual pass covers every task sitting at review (review-findings finding 6) | 0 | T041, T045, T051, T210, T220, T221, T231, T232, T325 | review |
| [T070](T070-file-watcher.md) | File watcher | 1A/2 | T020, T030, ADR-001 | review |
| [T080](T080-search.md) | Search: the walk, the match, the streamed result (the core half) | 2 | T020, T070, ADR-001 | review |
| [T100](T100-git-baseline.md) | Git baseline: gix status and typed diff (the core half) | 2 | T020, ADR-001 | review |
| [T110](T110-diagnostics.md) | Diagnostics | 2 | T060, T090, T220, T325, ADR-001 | todo |
| [T200](T200-workstream-model.md) | Workstream model + decisions (Gate C) | 3 | T010, T020, T060 | done |
| [T210](T210-workstream-ui.md) | Workstream UI: the island and the attention queue | 3 | T200 | review |
| [T220](T220-agent-session-discovery.md) | Agent session discovery: what is running, and on what | 3 | T010, T020, T210 | done |
| [T221](T221-agent-hook-bridge.md) | Agent hook bridge: earning `BLOCKED` | 3 | T220, T230, ADR-018 | review |
| [T222](T222-agent-providers.md) | More providers: Codex first, then what can be verified | 3 | T220, T221 | superseded |
| [T223](T223-agent-attention.md) | Attention: notify when an agent needs you, and link it to the work | 3 | T200, T210, T220, T221 | done |
| [T224](T224-reveal-terminal.md) | Reveal the terminal a session is running in | 3 | T220 | todo |
| [T225](T225-hook-sidecar.md) | Bundle the agent hook as a sidecar | 3 | T221 | review |
| [T230](T230-agent-timeline.md) | Agent timeline: what a session is actually doing | 3 | T020, T220 | done |
| [T231](T231-agent-rail.md) | The agent rail: a dedicated region for running agents | 3 | T046, T060, T220, T230 | done |
| [T232](T232-timeline-view.md) | The timeline view, and the files an agent touched | 3 | T030, T230, T231 | review |
| [JSDOM-VITEST](JSDOM-VITEST-component-tests.md) | A jsdom vitest project, so a `.tsx` can be tested against a DOM | 3 | T096, T232 | review |
| [T300](T300-agent-protocol-fake.md) | Agent event protocol + deterministic fake agent (Gate D) | 4 | T200, T210 | superseded |
| T305 | Agent capability model | 4 | T300 | unspec |
| T310 | First real adapter: a Claude Code run Gritty owns, and can prompt | 4 | T300-WIRE | superseded |
| T310-A | gritty-agent-claude: the process, the framing, the translation | 4 | T300-WIRE | review |
| T310-B | CLAUDE_CODE, the run inbox, and runs.start/runs.prompt | 4 | T310-A | review |
| T315 | Live agent activity | 4 | T310 | superseded by T220/T230/T231/T232 |
| T310-U | The prompt box, and an owned run in the console | 4 | T310-B | review |
| T310-ANSWER | Arm the host permission verdict for an owned run | 4 | T310-B | todo |
| T310-VERDICT-EVENT | Record a permission verdict in a run's ring | 4 | T310-B | unspec |
| T320 | Context manifest | 4 | T300 | unspec |
| T310-RESUME | Take over an observed session as an owned run | 4 | T310-B | unspec |
| T310-U-FILES | Files touched, on an owned run's console | 4 | T310-U | unspec |
| T310-U-CANCEL | Stop a run from the console it is showing in | 4 | T310-U | unspec |
| [T325](T325-workspace-ownership.md) | Workspace ownership registry | 4 | T200, T210, ADR-019 | done |
| [T326](T326-ownership-violations.md) | Observed ownership violations | 4 | T223, T230, T325 | review |
| T330 | Write interception (enforced ownership) | 4 | T325, ADR-018 amendment | unspec |
| [T335](T335-agent-permissions.md) | Permissions: asked in place, answered in a queue (12a/12b) | 4 | T210, T221, T232, ADR-020 | review |
| T400 | Worktree isolation | 5 | T100, T325 | unspec |
| T405 | Ownership-aware scheduler | 5 | T325, T300 | unspec |
| T410 | Attention queue | 5 | T210, T325, T335 | unspec |
| ADR-017 | Decide the terminal command-block contract (drafted, undecided) | 2 | – | todo |
| ADR-018A | ADR-018 amendment: a hook that may refuse a write | 4 | T326 | todo |
| ADR-021 | Persistence for a run Gritty owns | 4 | T300-C | review |
| ADR-022 | Search engine: in-process grep/ignore, or spawn ripgrep | 2 | – | review |
| ADR-023 | Git library: gix, git2, or porcelain | 2 | – | review |
| ADR-029 | Git writes run through the git binary, not gix | 2 | – | unspec |
| ADR-031 | Settings are a core-owned settings.json, user scope only | 2 | – | review |
| ADR-032 | Record the UI framework decision (React 19) | 0 | – | review |
| CI-COST | CI: self-hosted macOS runner, and stop burning metered minutes |  | – | review |
| CI-LINUX-DARK | Linux CI is dark until the minute allowance resets (1 Oct) or a spending limit is set |  | – | blocked |
| [CI-RUNNER-GC](CI-RUNNER-GC-disk-retention.md) | Self-hosted runner never cleans up: target/ and stale bundles grow unbounded on the dev Mac |  | – | review |
| CONFIRM-DIALOG | Extract a shared confirm dialog before a third copy is written | 2 | – | unspec |
| [EVENT-LISTEN-GAP](EVENT-LISTEN-GAP-startup-events.md) | An event emitted before the WebView is listening reaches nobody | 1 | T010 | review |
| [EXPLORER-FORCE-REPLAY](EXPLORER-FORCE-REPLAY-dirty-document-replay.md) | deleteAnyway is dead code and the notice replays the wrong action | 2 | – | review |
| MARKDOWN-BENCH | A markdown benchmark with a recorded baseline, if a perf gate is wanted | 3 | MARKDOWN-BUDGET-FLAKE | unspec |
| MARKDOWN-BUDGET-FLAKE | render.test.ts 2000ms wall-clock budget fails under concurrent-build load | 3 | – | review |
| MONACO-BARREL | packages/editor imports monaco whole; the main chunk is 4.6 MB |  | – | todo |
| MONACO-JSDOM-FLAKE | model-eol.dom.test.ts times out its 10s beforeAll under load | 3 | JSDOM-VITEST | review |
| MONACO-PREBUNDLE | Pre-bundle monaco-editor so the 9s transform is not paid every run | 3 | MONACO-JSDOM-FLAKE | unspec |
| PLAN-FANOUT | Split the remaining work into fan-out-sized tickets, and the lanes to run them in |  | – | review |
| PROTO-W2 | Wave 2 wire, in one PR: watcher, git, terminal blocks, diagnostics, violations | 2 | ADR-001, ADR-017, ADR-023 | todo |
| PROTO-W2-GIT | The git wire: git.status and git.diff reach the shell | 2 | T100,ADR-023 | review |
| PROTO-W3 | Wave 3 wire, in one PR: search | 2 | ADR-022 | review |
| RUNS-EVENT-FILTER | use-runs.ts subscribes to every event of every kind |  | – | todo |
| SIDECAR-BUNDLE-CONFIG | Move externalBin to a bundle-time config so a compile never needs a staged binary | 3 | SIDECAR-FRESH-CHECKOUT | unspec |
| SIDECAR-FRESH-CHECKOUT | A fresh clone or worktree cannot build gritty-desktop until T225's sidecar is staged by hand | 3 | T225 | review |
| T080-U | The search panel | 2 | PROTO-W3, T080 | todo |
| T100-U | Git status and diff, in the shell | 2 | PROTO-W2, T100 | review |
| T101-C | The read contract: git.log and git.show | 2 | T100 | review |
| T101-D | The diff as an editor tab (Monaco diff editor) on git.show | 2 | T101-R | todo |
| T101-R | gix: a paged commit walk, and a blob at a revision | 2 | T101-C | review |
| T101-U | History in the git panel, on git.log | 2 | T101-R | review |
| T102-C | The write contract: stage, unstage, discard, commit | 2 | ADR-029 | review |
| T102-R | gritty-git-write: the subprocess adapter and the git probe | 2 | T102-C | review |
| T102-U | Staging, discard and the commit box in the Changes panel | 2 | T102-R | todo |
| T120-C | The settings contract: settings.* and SettingsChanged | 2 | ADR-031 | todo |
| T120-M | Mount the settings tab, and the comma chord | 2 | T120-U | todo |
| T120-R | gritty-settings: the registry, the file, and the failure policy | 2 | T120-C | doing |
| T120-T | A theme the user picks: system, light or dark | 2 | T120-C | todo |
| T120-U | The settings page, and retiring the three popovers | 2 | T120-C | todo |
| T130-C | The quick-open contract: workspace.findFiles and ranked paths | 2 | T080 | blocked |
| T130-M | Mount the bar in the app bar, and the quick-open chord | 2 | T130-U, T310-U | todo |
| T130-R | A fuzzy path scorer, and the walk that ranks with it | 2 | T080 | review |
| T130-S | @ symbols: a regex outline, honestly named | 2 | T130-U | doing |
| T130-U | The quick-open bar, and its four modes | 2 | T130-C, T130-R | doing |
| T222-CODEX | Codex, as one provider file | 3 | T222-SEAM | todo |
| T222-SEAM | A provider registry, so providers stop being serial | 3 | T220 | unspec |
| [T233](T233-agents-sidebar.md) | Agents in the left sidebar (design artboard 12a) | 3 | – | unspec |
| T300-ANSWER | Answer a permission or decision on a run Gritty owns | 4 | T300-WIRE | unspec |
| T300-C | The run contract, alone (Gate D) | 4 | T200, T210 | review |
| T300-CMD | runs.* adapters, and the minimal surface | 4 | T300-C | review |
| T300-CORE | The run manager: supervision, cancellation, bounded events | 4 | T300-C | review |
| T300-FAKE | gritty-agent-fake: a scripted, deterministic adapter | 4 | T300-C | review |
| T300-PANEL | A run belongs on its workstream's island row, not a fourth roster | 4 | T300-WIRE | unspec |
| T300-WIRE | Connect the three Gate D halves: fake driver, manager, runs.* adapters | 4 | T300-CORE | review |
| T400-R | Worktree create and remove | 5 | T100 | unspec |
| WATCH-FLAKE | watch.rs::a_directory_created_after_the_watch_started_is_followed is load-sensitive |  | – | todo |
| T233-M | The roster row model: owned runs and observed sessions in one list | 3 | T300-C | review |
| T233-S | The Agents section in the left sidebar, and the sidebar tab strip (12a) | 3 | T233-M | review |
| T233-C | The full-height agent console, and retiring the right rail (12a) | 3 | T233-S,T232 | review |
<!-- board:table end -->

## Dependency graph (architecture addendum §2)

```
T000 Bootstrap
 └─ T010 Core protocol
     └─ T020 Workspace
         ├─ T030 File I/O + document/buffer foundation
         │   ├─ T040 Monaco
         │   └─ T050 Markdown
         ├─ T070 File watcher
         └─ T080 Search

T060 Persistence ─────────────┐
T090 Terminal                 ├─ Phase 2 dogfood milestone
T100 Git                      │
T110 Diagnostics ─────────────┘

T200 Workstream model (done: state machine, decisions, persistence)
 └─ T210 Workstream UI
     ├─ T220 Agent session discovery (read-only; observes agents Gritty
     │    │   does not own, so it does not gate on T300)
     │    ├─ T230 Agent timeline (follower, domain, protocol — no UI)
     │    │    ├─ T221 Hook bridge → BLOCKED (ADR-018; needs T230's fast loop)
     │    │    │    ├─ T222 More providers (Codex …)
     │    │    │    ├─ T223 Notifications + workstream link
     │    │    │    └─ T335 Permissions answered (ADR-020 amends ADR-018 §5;
     │    │    │         the hook may wait for a verdict, bounded, fail-open)
     │    │    └─ T231 Agent rail (third region, status colours)
     │    │         └─ T232 Timeline view + files touched
     │    └─ T224 Reveal the terminal (independent of the rest)
     ├─ T300 Agent event protocol + deterministic fake agent (Gate D)
     │    ├─ T305 Capability model
     │    │    └─ T310 First real adapter
     │    │         └─ T315 Live agent activity — already built by T220–T232
     │    └─ T320 Context manifest
     └─ T325 Workspace ownership registry (observed; holds a workstream's
          │   claims, refuses a second one — it does not stop a write)
          ├─ T326 Observed violations (needs T223's session↔workstream link)
          ├─ T330 Write interception (ADR-020 is the amendment it needed)
          ├─ T400 Worktree isolation
          ├─ T405 Ownership-aware scheduler
          └─ T410 Attention queue
```

The Phase 4–5 numbering follows `docs/plans/Gritty Agent Runtime
Coordination.md` §36, adopted by ADR-019. **`T330` means write interception
here**; decision interruption, which older documents call T330, is `T335`.
Completion reports keep the number they were written with.

**`T335` did not wait for T300.** It was written as permission prompts, and it
sits under T221's hook bridge rather than under agent runs, because ADR-020
established that a hook may be answered — which is the mechanism T330 was also
waiting for. What still belongs to T300 is a run Gritty *starts*: typing into an
agent, cancelling a turn, and the structured questions a run raises.

## Working a task

1. Take the ticket: `scripts/agent-board.py take T010 --agent <name> --session
   <your-session-uuid>`. That records the work as yours for every other session,
   in every worktree, and claims your lane's file lease with it. Do not edit the
   status here or in the task file by hand — both are generated from the board.
2. Follow `AGENTS.md` (scope, tests, `pnpm check`, completion report).
3. Open a PR named after the task (`T010: core protocol`), paste the completion
   report, then `agent-board.py move T010 --to review --pr <n>` and
   `agent-board.py render --index`, committing what it writes. The reviewer
   flips it to `done`; the tool refuses to let an agent do it. Cut the branch
   from `main`, and check `docs/merge-queue.md` first — it says which version
   constants and migration numbers are already claimed by something in flight
   (`scripts/merge-queue.py` renders the live queue).
4. Newly discovered work becomes a bullet under "Follow-up tasks" in the
   completion report, and a new task file only when it has an owner.
