# Completed tasks

Tickets that are **done** or **superseded**, and their task files. The open
ones are in [the task index](../README.md), which also carries the dependency
graph and the workflow.

**Generated** by `scripts/agent-board.py render --index`, from the same board
as the index. When the reviewer flips a ticket to done, its row and its task
file move here, and every `docs/tasks/…` path that names the file is repointed;
a reopened ticket moves back the same way. Do not move files in or out by hand.

<!-- board:table start -->
| ID | Task | Phase | Depends on | Status |
|---|---|---|---|---|
| [T000](T000-bootstrap-workspace.md) | Bootstrap workspace | 0 | – | done |
| [T010](T010-core-protocol.md) | Core protocol + operation/event/error conventions (Gate A) | 0 | T000 | done |
| [T020](T020-workspace-open-list.md) | Workspace open/list + fixtures | 0/1A | T010 | done |
| [T030](T030-file-io-documents.md) | File I/O + document/buffer contract (Gate B) | 0/1A | T020 | done |
| [T040](T040-monaco-shell.md) | Monaco shell | 0/1A | T030 | done |
| [T041](T041-preview-tabs.md) | Preview tabs, and closing more than one at a time | 1A | T020, T030, T040 | done |
| [T042](T042-explorer-file-operations.md) | Editing the file tree: create, rename, move, delete, undo | 1A | T020, T030, T040, T070 | done |
| [T045](T045-drag-and-drop.md) | Drag and drop files/folders onto the window | 1A | T030, T040 | done |
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
| [T098](T098-terminal-colours-and-rooting.md) | Terminal session colours, and starting where you work | 0/2 | T047, T090, T095, T096 | done |
| [PTY-FLAKE](PTY-FLAKE-process-tree-kill.md) | `pty.rs::kill_terminates_the_whole_process_tree` is flaky under load | 2 | T090 | done |
| ADR-001 | Confirm or reject Tauri | 0 | Phase 0 tests | done |
| MANUAL-PASS-COVERAGE | The manual pass covers every task sitting at review (review-findings finding 6) | 0 | T041, T045, T051, T210, T220, T221, T231, T232, T325 | done |
| [T070](T070-file-watcher.md) | File watcher | 1A/2 | T020, T030, ADR-001 | done |
| [T080](T080-search.md) | Search: the walk, the match, the streamed result (the core half) | 2 | T020, T070, ADR-001 | done |
| [T100](T100-git-baseline.md) | Git baseline: gix status and typed diff (the core half) | 2 | T020, ADR-001 | done |
| [T200](T200-workstream-model.md) | Workstream model + decisions (Gate C) | 3 | T010, T020, T060 | done |
| [T210](T210-workstream-ui.md) | Workstream UI: the island and the attention queue | 3 | T200 | done |
| [T220](T220-agent-session-discovery.md) | Agent session discovery: what is running, and on what | 3 | T010, T020, T210 | done |
| [T221](T221-agent-hook-bridge.md) | Agent hook bridge: earning `BLOCKED` | 3 | T220, T230, ADR-018 | done |
| [T222](T222-agent-providers.md) | More providers: Codex first, then what can be verified | 3 | T220, T221 | superseded |
| [T223](T223-agent-attention.md) | Attention: notify when an agent needs you, and link it to the work | 3 | T200, T210, T220, T221 | done |
| [T225](T225-hook-sidecar.md) | Bundle the agent hook as a sidecar | 3 | T221 | done |
| [T230](T230-agent-timeline.md) | Agent timeline: what a session is actually doing | 3 | T020, T220 | done |
| [T231](T231-agent-rail.md) | The agent rail: a dedicated region for running agents | 3 | T046, T060, T220, T230 | done |
| [T232](T232-timeline-view.md) | The timeline view, and the files an agent touched | 3 | T030, T230, T231 | done |
| [JSDOM-VITEST](JSDOM-VITEST-component-tests.md) | A jsdom vitest project, so a `.tsx` can be tested against a DOM | 3 | T096, T232 | done |
| [T300](T300-agent-protocol-fake.md) | Agent event protocol + deterministic fake agent (Gate D) | 4 | T200, T210 | superseded |
| [T310](T310-claude-code-adapter.md) | First real adapter: a Claude Code run Gritty owns, and can prompt | 4 | T300-WIRE | superseded |
| T310-A | gritty-agent-claude: the process, the framing, the translation | 4 | T300-WIRE | done |
| T310-B | CLAUDE_CODE, the run inbox, and runs.start/runs.prompt | 4 | T310-A | done |
| T315 | Live agent activity | 4 | T310 | superseded by T220/T230/T231/T232 |
| T310-U | The prompt box, and an owned run in the console | 4 | T310-B | done |
| [T325](T325-workspace-ownership.md) | Workspace ownership registry | 4 | T200, T210, ADR-019 | done |
| [T326](T326-ownership-violations.md) | Observed ownership violations | 4 | T223, T230, T325 | done |
| [T335](T335-agent-permissions.md) | Permissions: asked in place, answered in a queue (12a/12b) | 4 | T210, T221, T232, ADR-020 | done |
| ADR-017 | Decide the terminal command-block contract (drafted, undecided) | 2 | – | done |
| ADR-018A | ADR-018 amendment: a hook that may refuse a write | 4 | T326 | done |
| ADR-021 | Persistence for a run Gritty owns | 4 | T300-C | done |
| ADR-022 | Search engine: in-process grep/ignore, or spawn ripgrep | 2 | – | done |
| ADR-023 | Git library: gix, git2, or porcelain | 2 | – | done |
| ADR-029 | Git writes run through the git binary, not gix | 2 | – | done |
| ADR-031 | Settings are a core-owned settings.json, user scope only | 2 | – | done |
| ADR-032 | Record the UI framework decision (React 19) | 0 | – | done |
| ADR-033 | What [profile.dev] should be: debuginfo and split-debuginfo on macOS | 2 | – | done |
| CI-COST | CI: self-hosted macOS runner, and stop burning metered minutes |  | – | done |
| [CI-RUNNER-GC](CI-RUNNER-GC-disk-retention.md) | Self-hosted runner never cleans up: target/ and stale bundles grow unbounded on the dev Mac |  | – | done |
| [EVENT-LISTEN-GAP](EVENT-LISTEN-GAP-startup-events.md) | An event emitted before the WebView is listening reaches nobody | 1 | T010 | done |
| [EXPLORER-FORCE-REPLAY](EXPLORER-FORCE-REPLAY-dirty-document-replay.md) | deleteAnyway is dead code and the notice replays the wrong action | 2 | – | done |
| MAIN-RED-T102C | main does not typecheck: git-write-client.test.ts TS2339 (2 lines) |  | – | done |
| [MONACO-BARREL](MONACO-BARREL-startup-chunk.md) | packages/editor imports monaco whole; the main chunk is 4.6 MB |  | – | superseded |
| [MONACO-DEFER](MONACO-DEFER-startup-import.md) | Load Monaco on demand, not at startup (measured: -80.8% of the chunk) |  | MONACO-BARREL | done |
| PLAN-FANOUT | Split the remaining work into fan-out-sized tickets, and the lanes to run them in |  | – | done |
| [PROTO-W2-GIT](PROTO-W2-GIT-git-wire.md) | The git wire: git.status and git.diff reach the shell | 2 | T100,ADR-023 | done |
| PROTO-W3 | Wave 3 wire, in one PR: search | 2 | ADR-022 | done |
| T100-U | Git status and diff, in the shell | 2 | PROTO-W2, T100 | done |
| [T101-C](T101-C-git-read-contract.md) | The read contract: git.log and git.show | 2 | T100 | done |
| T101-U | History in the git panel, on git.log | 2 | T101-R | done |
| [T102-C](T102-C-git-write-contract.md) | The write contract: stage, unstage, discard, commit | 2 | ADR-029 | done |
| [T102-R](T102-R-git-write-adapter.md) | gritty-git-write: the subprocess adapter and the git probe | 2 | T102-C | done |
| [T120-C](T120-C-settings-contract.md) | The settings contract: settings.* and SettingsChanged | 2 | ADR-031 | done |
| [T120-R](T120-R-settings-service.md) | gritty-settings: the registry, the file, and the failure policy | 2 | T120-C | done |
| [T130-C](T130-C-quick-open-contract.md) | The quick-open contract: workspace.findFiles and ranked paths | 2 | T080 | done |
| [T130-M](T130-M-quick-open-mount.md) | Mount the bar in the app bar, and the quick-open chord | 2 | T130-U, T310-U | done |
| [T130-R](T130-R-path-ranking.md) | A fuzzy path scorer, and the walk that ranks with it | 2 | T080 | done |
| [T130-S](T130-S-symbol-outline.md) | @ symbols: a regex outline, honestly named | 2 | T130-U | done |
| [T130-U](T130-U-quick-open-bar.md) | The quick-open bar, and its four modes | 2 | T130-C, T130-R | done |
| [T130-V](T130-V-quick-open-component.md) | The quick-open component, its controller, and the two test tiers | 2 | T130-C, T130-U | done |
| [T140-PROFILE](T140-PROFILE-dev-profile-debuginfo.md) | Measure, then set the dev profile debuginfo | 2 | ADR-033 | done |
| [T140-SWEEP](T140-SWEEP-local-gc-trigger.md) | Fire the build-cache sweeper where the growth is | 2 | – | done |
| [T233](T233-agents-sidebar.md) | Agents in the left sidebar (design artboard 12a) | 3 | – | done |
| T300-C | The run contract, alone (Gate D) | 4 | T200, T210 | done |
| T300-CMD | runs.* adapters, and the minimal surface | 4 | T300-C | done |
| T300-CORE | The run manager: supervision, cancellation, bounded events | 4 | T300-C | done |
| T300-FAKE | gritty-agent-fake: a scripted, deterministic adapter | 4 | T300-C | done |
| [T300-WIRE](T300-WIRE-gate-d-glue.md) | Connect the three Gate D halves: fake driver, manager, runs.* adapters | 4 | T300-CORE | done |
| T233-M | The roster row model: owned runs and observed sessions in one list | 3 | T300-C | done |
| T233-S | The Agents section in the left sidebar, and the sidebar tab strip (12a) | 3 | T233-M | done |
| T233-C | The full-height agent console, and retiring the right rail (12a) | 3 | T233-S,T232 | done |
<!-- board:table end -->
