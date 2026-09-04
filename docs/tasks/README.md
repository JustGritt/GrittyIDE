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
| [T050](T050-markdown.md) | Markdown | 0/1B | T030, T040 | done |
| [T060](T060-session-persistence.md) | Session persistence + recovery | 0/1B | T030, T040 | done |
| [T090](T090-terminal-pty.md) | Terminal (PTY) spike | 0/2 | T010 | review |
| Phase 0 tests | `docs/plans/phase-0-spike.md` sections B–D | 0 | T060, T090 | todo |
| ADR-001 | Confirm or reject Tauri | 0 | Phase 0 tests | todo |
| T070 | File watcher | 1A/2 | T020, ADR-001 | not yet specified |
| T080 | Search | 2 | T020, ADR-001 | not yet specified |
| T100 | Git baseline | 2 | ADR-001 | not yet specified |
| T110 | Diagnostics | 2 | ADR-001 | not yet specified |
| T200–T410 | Workstreams, agents, orchestration | 3–5 | Phase 2 | not yet specified |

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

T200 Workstream model
 └─ T210 Workstream UI
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
