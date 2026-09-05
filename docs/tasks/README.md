# Task index

One file per task, following `docs/agent/task-template.md`. IDs come from the
project plan (§17). Respect the dependency graph: never implement a downstream
domain abstraction before its upstream contract exists.

| ID | Task | Phase | Depends on | Status |
|---|---|---|---|---|
| [T000](T000-bootstrap-workspace.md) | Bootstrap workspace | 0 | – | done |
| [T010](T010-core-protocol.md) | Core protocol + operation/event/error conventions (Gate A) | 0 | T000 | done |
| [T020](T020-workspace-open-list.md) | Workspace open/list + fixtures | 0/1A | T010 | done |
| [T030](T030-file-io-documents.md) | File I/O + document/buffer contract (Gate B) | 0/1A | T020 | done |
| [T040](T040-monaco-shell.md) | Monaco shell | 0/1A | T030 | done |
| [T045](T045-drag-and-drop.md) | Drag and drop files/folders onto the window | 1A | T030, T040 | in-progress |
| [T046](T046-resizable-panes.md) | Resizable panes, and a terminal on opening | 1A | T040, T060, T095 | done |
| [T047](T047-terminal-names.md) | Naming terminal sessions | 1A | T090, T095 | done |
| [T048](T048-ui-revamp.md) | Shell UI revamp from design artboard 2a | 0 | T020, T030, T040, T046, T050, T060, T090 | done |
| [T049](T049-startup-screen.md) | Startup screen from design artboard 3a | 0 | T020, T030, T048, T060, T095 | review |
| [T050](T050-markdown.md) | Markdown | 0/1B | T030, T040 | done |
| [T051](T051-launcher-resume-rail.md) | Launcher resume rail from design artboard 3b | 0/1B | T049, T060 | review |
| [T060](T060-session-persistence.md) | Session persistence + recovery | 0/1B | T030, T040 | done |
| [T090](T090-terminal-pty.md) | Terminal (PTY) spike | 0/2 | T010 | done |
| [T095](T095-terminal-profiles.md) | Terminal shell profiles and rooting | 0/2 | T090, T060 | done |
| [T096](T096-terminal-chrome.md) | Terminal chrome from design artboard 8a | 0/2 | T047, T048, T090, T095 | review |
| [T097](T097-shell-integration-blocks.md) | Shell integration (OSC 133) and command blocks | 2 | T090, T095, T096 | not yet started |
| [T098](T098-terminal-colours-and-rooting.md) | Terminal session colours, and starting where you work | 0/2 | T047, T090, T095, T096 | review |
| Phase 0 tests | `docs/plans/phase-0-spike.md` sections B–D | 0 | T060, T090 | in-progress |
| ADR-001 | Confirm or reject Tauri | 0 | Phase 0 tests | todo |
| T070 | File watcher | 1A/2 | T020, ADR-001 | not yet specified |
| T080 | Search | 2 | T020, ADR-001 | not yet specified |
| T100 | Git baseline | 2 | ADR-001 | not yet specified |
| T110 | Diagnostics | 2 | ADR-001 | not yet specified |
| [T200](T200-workstream-model.md) | Workstream model + decisions (Gate C) | 3 | T010, T020, T060 | review |
| [T210](T210-workstream-ui.md) | Workstream UI: the island and the attention queue | 3 | T200 | review |
| [T220](T220-agent-session-discovery.md) | Agent session discovery: what is running, and on what | 3 | T010, T020, T210 | review |
| [T221](T221-agent-hook-bridge.md) | Agent hook bridge: earning `BLOCKED` | 3 | T220, T230, ADR-018 | review |
| [T222](T222-agent-providers.md) | More providers: Codex first, then what can be verified | 3 | T220, T221 | not yet started |
| [T223](T223-agent-attention.md) | Attention: notify when an agent needs you, and link it to the work | 3 | T200, T210, T220, T221 | not yet started |
| [T224](T224-reveal-terminal.md) | Reveal the terminal a session is running in | 3 | T220 | not yet started |
| [T230](T230-agent-timeline.md) | Agent timeline: what a session is actually doing | 3 | T020, T220 | review |
| [T231](T231-agent-rail.md) | The agent rail: a dedicated region for running agents | 3 | T046, T060, T220, T230 | review |
| [T232](T232-timeline-view.md) | The timeline view, and the files an agent touched | 3 | T030, T230, T231 | not yet started |
| T300–T410 | Agent protocol, runs, orchestration | 4–5 | T200, T210 | not yet specified |

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
     │    │    │    └─ T223 Notifications + workstream link
     │    │    └─ T231 Agent rail (third region, status colours)
     │    │         └─ T232 Timeline view + files touched
     │    └─ T224 Reveal the terminal (independent of the rest)
     └─ T300 Agent protocol + deterministic fake agent
         ├─ T310 First real agent
         ├─ T320 Context manifest
         └─ T330 Decision interruption
             ├─ T400 Worktree isolation
             └─ T410 Attention queue
```

## Working a task

1. Set the task status to `in-progress` in its file and in this table.
2. Follow `AGENTS.md` (scope, tests, `pnpm check`, completion report).
3. Open a PR named after the task (`T010: core protocol`), paste the completion
   report, set status to `review`; the reviewer flips it to `done`.
4. Newly discovered work becomes a bullet under "Follow-up tasks" in the
   completion report, and a new task file only when it has an owner.
