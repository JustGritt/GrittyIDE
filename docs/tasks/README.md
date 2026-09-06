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
| PTY-FLAKE | `pty.rs::kill_terminates_the_whole_process_tree` is flaky under load | 2 | T090 | todo |
| Phase 0 tests | `docs/plans/phase-0-spike.md` sections B–D | 0 | T060, T090 | doing |
| ADR-001 | Confirm or reject Tauri | 0 | Phase 0 tests | todo |
| MANUAL-PASS-COVERAGE | The manual pass covers every task sitting at review (review-findings finding 6) | 0 | T041, T045, T051, T210, T220, T221, T231, T232, T325 | review |
| [T070](T070-file-watcher.md) | File watcher | 1A/2 | T020, T030, ADR-001 | todo |
| [T080](T080-search.md) | Search | 2 | T020, T070, ADR-001 | todo |
| [T100](T100-git-baseline.md) | Git baseline | 2 | T020, ADR-001 | todo |
| [T110](T110-diagnostics.md) | Diagnostics | 2 | T060, T090, T220, T325, ADR-001 | todo |
| [T200](T200-workstream-model.md) | Workstream model + decisions (Gate C) | 3 | T010, T020, T060 | done |
| [T210](T210-workstream-ui.md) | Workstream UI: the island and the attention queue | 3 | T200 | review |
| [T220](T220-agent-session-discovery.md) | Agent session discovery: what is running, and on what | 3 | T010, T020, T210 | done |
| [T221](T221-agent-hook-bridge.md) | Agent hook bridge: earning `BLOCKED` | 3 | T220, T230, ADR-018 | review |
| [T222](T222-agent-providers.md) | More providers: Codex first, then what can be verified | 3 | T220, T221 | todo |
| [T223](T223-agent-attention.md) | Attention: notify when an agent needs you, and link it to the work | 3 | T200, T210, T220, T221 | done |
| [T224](T224-reveal-terminal.md) | Reveal the terminal a session is running in | 3 | T220 | todo |
| [T225](T225-hook-sidecar.md) | Bundle the agent hook as a sidecar | 3 | T221 | review |
| [T230](T230-agent-timeline.md) | Agent timeline: what a session is actually doing | 3 | T020, T220 | done |
| [T231](T231-agent-rail.md) | The agent rail: a dedicated region for running agents | 3 | T046, T060, T220, T230 | done |
| [T232](T232-timeline-view.md) | The timeline view, and the files an agent touched | 3 | T030, T230, T231 | review |
| [JSDOM-VITEST](JSDOM-VITEST-component-tests.md) | A jsdom vitest project, so a `.tsx` can be tested against a DOM | 3 | T096, T232 | review |
| [T300](T300-agent-protocol-fake.md) | Agent event protocol + deterministic fake agent (Gate D) | 4 | T200, T210 | todo |
| T305 | Agent capability model | 4 | T300 | unspec |
| T310 | First real adapter | 4 | T300, T305 | unspec |
| T315 | Live agent activity | 4 | T310 | superseded by T220/T230/T231/T232 |
| T320 | Context manifest | 4 | T300 | unspec |
| [T325](T325-workspace-ownership.md) | Workspace ownership registry | 4 | T200, T210, ADR-019 | done |
| [T326](T326-ownership-violations.md) | Observed ownership violations | 4 | T223, T230, T325 | todo |
| T330 | Write interception (enforced ownership) | 4 | T325, ADR-018 amendment | unspec |
| [T335](T335-agent-permissions.md) | Permissions: asked in place, answered in a queue (12a/12b) | 4 | T210, T221, T232, ADR-020 | review |
| T400 | Worktree isolation | 5 | T100, T325 | unspec |
| T405 | Ownership-aware scheduler | 5 | T325, T300 | unspec |
| T410 | Attention queue | 5 | T210, T325, T335 | unspec |
| CI-COST | CI: self-hosted macOS runner, and stop burning metered minutes |  | – | review |
| CI-LINUX-DARK | Linux CI is dark until the minute allowance resets (1 Oct) or a spending limit is set |  | – | blocked |
| CI-RUNNER-GC | Self-hosted runner never cleans up: target/ and stale bundles grow unbounded on the dev Mac |  | – | unspec |
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
