# Task index

One file per task, following `docs/agent/task-template.md`. IDs come from the
project plan (§17). Respect the dependency graph: never implement a downstream
domain abstraction before its upstream contract exists.

**The table below is generated**, and so is the `- **Status:**` line of every
task file. Both come from the live board (`scripts/agent-board.py`, see *Working
a task*); edit either by hand and the next `render --index` overwrites it. Only
the table between the markers is written — this prose, the dependency graph and
the workflow below are hand-maintained.

The table lists **open work only**. When the reviewer flips a ticket to done, or
it is superseded, the same command moves its row to
[completed/README.md](completed/README.md) and its task file to `completed/`,
and repoints every `docs/tasks/…` path that names the file. A reopened ticket
moves back. Never move a task file by hand.

<!-- board:table start -->
| ID | Task | Phase | Depends on | Status |
|---|---|---|---|---|
| [T097](T097-shell-integration-blocks.md) | Shell integration (OSC 133) and command blocks | 2 | T090, T095, T096 | todo |
| [T099-S](T099-S-terminal-splits.md) | Split terminals: shells side by side in the dock | 2 | T096, T098, T046, T120-T, T120-A | todo |
| Phase 0 tests | `docs/plans/phase-0-spike.md` sections B–D | 0 | T060, T090 | doing |
| [T099-E](T099-E-terminal-editor-tab.md) | A terminal as an editor tab | 2 | T099-S, T120-M | todo |
| [T110](T110-diagnostics.md) | Diagnostics | 2 | T060, T090, T220, T325, ADR-001 | todo |
| [T224](T224-reveal-terminal.md) | Reveal the terminal a session is running in | 3 | T220 | todo |
| T305 | Agent capability model | 4 | T300 | unspec |
| [T310-ANSWER](T310-ANSWER-host-permissions.md) | Arm the host permission verdict for an owned run | 4 | T310-B | review |
| T310-VERDICT-EVENT | Record a permission verdict in a run's ring | 4 | T310-B | unspec |
| T320 | Context manifest | 4 | T300 | unspec |
| T310-RESUME | Take over an observed session as an owned run | 4 | T310-B | unspec |
| T310-U-FILES | Files touched, on an owned run's console | 4 | T310-U | unspec |
| T310-U-CANCEL | Stop a run from the console it is showing in | 4 | T310-U | unspec |
| T330 | Write interception (enforced ownership) | 4 | T325, ADR-018 amendment | unspec |
| T400 | Worktree isolation | 5 | T100, T325 | unspec |
| T405 | Ownership-aware scheduler | 5 | T325, T300 | unspec |
| T410 | Attention queue | 5 | T210, T325, T335 | unspec |
| [AGENT-TURN-END](AGENT-TURN-END-roster-done-while-generating.md) | The agent roster says Done while the model is still generating | 3 | T220, T221, T223, T231 | review |
| BOARD-ARCHIVE | Done and superseded tickets move to a generated archive, so the index lists open work | tools | – | review |
| BOARD-DROP-DONE | drop --to done skips the reviewer rule move enforces | tools | – | unspec |
| [BOARD-STALE-REASON](BOARD-STALE-REASON-labelled-reasons.md) | A block reason outlives its block, and show prints it as present tense | tools | – | review |
| [CARGO-FLAKE](CARGO-FLAKE-desktop-command-tests.md) | Three desktop command tests fail under a loaded machine | 2 | – | todo |
| CI-LINUX-DARK | Linux CI is dark until the minute allowance resets (1 Oct) or a spending limit is set |  | – | blocked |
| CONFIRM-DIALOG | Extract a shared confirm dialog before a third copy is written | 2 | – | unspec |
| [EXPLORER-DISCARD-NOTICE](docs/tasks/EXPLORER-DISCARD-NOTICE-dirty-documents-only.md) | The discard notice replays only a dirty-document refusal | maintenance | T042, EXPLORER-FORCE-REPLAY | review |
| [FILE-ICONS](docs/tasks/FILE-ICONS-file-type-icons.md) | File-type icons in the explorer tree and Changes list | 2 | – | review |
| [FILES-PARTIAL](docs/tasks/FILES-PARTIAL-settle-what-was-done.md) | A move or delete that stops partway settles what it did | maintenance | T042 | review |
| [GC-BUDGET](GC-BUDGET-sweeper-defaults.md) | Re-derive the sweeper's cache budget and disk floor for ADR-033's smaller builds | tools | ADR-024, ADR-033 | review |
| GC-DEV-SKIP | A running tauri dev stops every sweep of its checkout: the process scan outranks a free .cargo-lock | tools | GC-BUDGET | todo |
| [GIT-STATES](GIT-STATES-in-progress-state-fixtures.md) | Test the in-progress repository states git status names but never exercised | 2 | T100, ADR-023 | review |
| IGNORE-RULES | IgnoreRules in gritty-git: Git's exclude rules plus the index, checked against git check-ignore | 2 | ADR-034 | unspec |
| IGNORE-SEAM | One ignored predicate handed to the listing, the watcher, search and quick-open | 2 | IGNORE-RULES | unspec |
| IGNORE-UI | The include-ignored toggle in search and quick-open, and how the explorer marks ignored rows | 2 | IGNORE-WIRE | unspec |
| IGNORE-WIRE | ignoredHint on files, both includeIgnored docs, includeIgnored on workspace.findFiles | 2 | IGNORE-SEAM | unspec |
| [LAST-PROMPT-NOTIFY](docs/tasks/LAST-PROMPT-NOTIFY-task-notifications.md) | A task notification is not a session's last prompt | maintenance | LAST-PROMPT-SHELL | review |
| [LAST-PROMPT-SHELL](docs/tasks/LAST-PROMPT-SHELL-bang-commands.md) | A ! shell command is not a session's last prompt | maintenance | AGENT-TURN-END | review |
| [LEASE-NOTIFY](docs/tasks/LEASE-NOTIFY-lease-change-notices.md) | Tell a session when the lease registry changes under it | tooling | – | review |
| MARKDOWN-BENCH | A markdown benchmark with a recorded baseline, if a perf gate is wanted | 3 | MARKDOWN-BUDGET-FLAKE | unspec |
| MARKDOWN-BUDGET-FLAKE | render.test.ts 2000ms wall-clock budget fails under concurrent-build load | 3 | – | review |
| MONACO-JSDOM-FLAKE | model-eol.dom.test.ts times out its 10s beforeAll under load | 3 | JSDOM-VITEST | review |
| MONACO-PREBUNDLE | Pre-bundle monaco-editor so the 9s transform is not paid every run | 3 | MONACO-JSDOM-FLAKE | unspec |
| [NOTIFY-SETTING-ORPHAN](docs/tasks/NOTIFY-SETTING-ORPHAN-delete.md) | Delete the notification switch nothing mounts | maintenance | T120-U, T120-M | review |
| PROTO-W2 | Wave 2 wire, in one PR: watcher, git, terminal blocks, diagnostics, violations | 2 | ADR-001, ADR-017, ADR-023 | todo |
| [RUNS-EVENT-FILTER](RUNS-EVENT-FILTER-subscription.md) | use-runs.ts subscribes to every event of every kind |  | – | review |
| [SEARCH-WALK-TIME](SEARCH-WALK-TIME-large-repo-timing.md) | Time a search over large-repo against ADR-022's revisit trigger | 2 | T080, ADR-022 | review |
| [SIDEBAR-ICONS](docs/tasks/SIDEBAR-ICONS-section-tab-icons.md) | Sidebar section tabs as icons, for horizontal room | 2 | – | review |
| SIDECAR-BUNDLE-CONFIG | Move externalBin to a bundle-time config so a compile never needs a staged binary | 3 | SIDECAR-FRESH-CHECKOUT | unspec |
| SIDECAR-FRESH-CHECKOUT | A fresh clone or worktree cannot build gritty-desktop until T225's sidecar is staged by hand | 3 | T225 | review |
| T080-U | The search panel | 2 | PROTO-W3, T080 | todo |
| T101-D | The diff as an editor tab (Monaco diff editor) on git.show | 2 | T101-R | todo |
| T101-R | gix: a paged commit walk, and a blob at a revision | 2 | T101-C | review |
| [T102-U](T102-U-git-write-ui.md) | Staging, discard and the commit box in the Changes panel | 2 | T102-R | doing |
| [T120-A](T120-A-settings-apply.md) | Make the editor and terminal obey their settings, then delete the popovers | 2 | T120-M | review |
| [T120-M](T120-M-settings-mount.md) | Mount the settings tab, and the comma chord | 2 | T120-U | review |
| [T120-T](T120-T-theme.md) | A theme the user picks: system, light or dark | 2 | T120-C | review |
| [T120-U](T120-U-settings-page.md) | The settings page, and retiring the three popovers | 2 | T120-C | review |
| T222-CODEX | Codex, as one provider file | 3 | T222-SEAM | todo |
| T222-SEAM | A provider registry, so providers stop being serial | 3 | T220 | unspec |
| T300-ANSWER | Answer a permission or decision on a run Gritty owns | 4 | T300-WIRE | unspec |
| T300-PANEL | A run belongs on its workstream's island row, not a fourth roster | 4 | T300-WIRE | unspec |
| T400-R | Worktree create and remove | 5 | T100 | unspec |
| [TREE-DISCLOSURE](docs/tasks/TREE-DISCLOSURE-chevron-transition.md) | Tree chevron and expand/collapse transition | 2 | FILE-ICONS | review |
| [TURN-START-DEQUEUE](docs/tasks/TURN-START-DEQUEUE-measure.md) | Measure whether a dequeued message should start a turn | maintenance | AGENT-TURN-END | review |
| [TURN-START-QUEUED](docs/tasks/TURN-START-QUEUED-dequeue-and-shell.md) | A dequeued message or a ! command starts a turn | maintenance | TURN-START-DEQUEUE, AGENT-TURN-END | review |
| [WATCH-FLAKE](WATCH-FLAKE-load-sensitive.md) | Two load-sensitive tests fail in the full-workspace run, pass alone |  | – | doing |
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
