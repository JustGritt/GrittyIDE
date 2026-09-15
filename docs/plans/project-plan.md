# Gritty — Project Plan

- **Status:** Plan of record, as of 2026-09-15
- **Ticket:** PLAN-OF-RECORD (one project plan replaces the founding plan, its
  addendum and the roadmap)
- **Replaces, as the plan:** `docs/Gritty IDE Project Plan.md` (2 Sep),
  `docs/Gritty IDE Architecture Refinements.md` (2 Sep) and
  `docs/plans/roadmap.md`. The two founding documents stay in the repository
  unchanged, because code, ADRs and task files cite their section numbers; they
  are history, not instructions.
- **Amended by:** ADR-035 (Orca parity on the core loop comes after cleanup and
  before Phases 3–4), which owns the forward sequence in §10.

## 0. How to read this plan

When two sources disagree, the more specific one wins, in this order:

1. **Accepted ADRs** in `docs/decisions/`.
2. **This plan.**
3. **`docs/architecture/*`**, and `docs/plans/Gritty Agent Runtime
   Coordination.md` for Phases 4–5 wherever no ADR overrides it.
4. **The founding documents**, cited only as history ("Plan §7", "addendum §8").

What this plan owns: the product definition, the architecture as built, the
contract gates, the phase map, the dependency graph and the open decisions.

What it does not own:

- **Ticket status.** That lives on the board (`scripts/agent-board.py list`)
  and its committed projection (`docs/tasks/README.md`,
  `docs/tasks/completed/README.md`). A status written here is a dated snapshot,
  not a record.
- **Version and migration numbers.** `docs/merge-queue.md` is the record.
- **The working contract for agents.** `AGENTS.md` owns the rules, the task
  template, the definition of done and the lease protocol.

## 1. What Gritty is

A reliability-first desktop development environment: a dependable code editor
that becomes a cockpit for supervised coding agents, and stays fully useful when
no agent is available. Tauri 2 with a Rust core, and React plus Monaco in the
window.

**Since ADR-035 (2026-09-15)** Gritty is a portfolio and learning project whose
goal is a polished product **on par with Orca on the core loop**:

1. open a repository;
2. create a task, which is a workstream owning a worktree and a branch;
3. launch an agent CLI into a terminal in that worktree;
4. see its status, be notified, and answer its permission prompts;
5. review the diff against the base and send line comments back to the agent;
6. commit, push and open a pull request;
7. remove the task, with unmerged work surfaced first.

Orca's browser, mobile, SSH, cloud, tracker integrations, plugins and its
catalogue of agents are not goals until after M5.

**The product thesis still holds** (Plan §2): the unit of work is a
workstream, not a chat; files and diffs are supporting views of work; agent
activity is structured; human attention is scarce; project knowledge outlives
any one conversation; models and CLIs are replaceable backends.

**Two agent lanes exist, and neither founding document foresaw the first:**

| Lane | What it is | Where |
|---|---|---|
| Observed sessions | Agents the user started elsewhere (Claude Code in any terminal), found from processes, transcripts and a consented hook | T220–T233, ADR-018, ADR-018A, ADR-020, `docs/architecture/agent-sessions.md` |
| Owned runs | Agents Gritty starts and supervises as child processes | T300-*, T310-*, ADR-021, `docs/architecture/agent-runs.md` |

**Positioning** (from the roadmap's 2026-09-06 amendment): Gritty does not
rebuild a terminal multiplexer. It is the layer multiplexers such as Herdr say
they will not be: worktree isolation, approval policy, durable state.

### Goals and non-goals

| Goals | Non-goals |
|---|---|
| A dependable daily editor on macOS | VS Code extension compatibility |
| Monaco editing, Markdown preview, terminal, explorer, search, Git | A new text editor engine, or a new VCS |
| Crash-resistant process architecture and recoverable sessions | Autonomous agents with unlimited permissions |
| Tasks as worktrees, agents in terminals, a review loop (ADR-035) | Multi-user collaboration |
| Agent execution behind provider-neutral contracts | Windows before the loop is proven |
| | Orca's non-loop features (ADR-035 §1) |

## 2. Principles

| Principle | Meaning |
|---|---|
| Editor before agent | Every core editing workflow works with no AI account and no network |
| Failure containment | Agent, PTY, Git, watcher and search failures never take down or freeze the window |
| Persist intent, not transcripts | Store goals, decisions, state and results; raw logs are secondary |
| Portable artifacts | Markdown and ordinary repository files before proprietary formats |
| Measure before optimizing | No bespoke fast path without a measurement |
| Explicit permissions | Agents act under visible, answerable permissions |
| No hidden magic | Context, changes, commands and decisions are inspectable |
| Recoverability | Unexpected termination loses as little as possible, and never silently |

**The priority chain** is `correctness → recoverability → inspectability →
responsiveness → ergonomics → orchestration power`. Since ADR-035 §4 it governs
**how** a feature is built: nothing ships that loses work, lies about state or
blocks the renderer. It no longer decides **which** feature comes next; the
milestones in §10 do.

**Deliberate differences from Orca**, kept (ADR-035 §4):

- permission prompts are answered, never bypassed by default;
- a status names its evidence;
- the core owns document state (an autosave setting is allowed; a Monaco-owned
  buffer is not);
- process trees are killed and output is bounded.

**Every surface should let the user answer eight questions** (addendum §20):
what is happening; who initiated it; what can it modify; can I cancel it; what
changed; what failed; can I recover; what needs my attention.

## 3. Platform

- **Runtime.** ADR-001 accepts Tauri 2, in a narrowed form. Kill criterion 1
  (Monaco and text input on WKWebView) was never evaluated, and criterion 3
  (Linux WebView divergence) could not be. Both survive as *Revisit when*
  triggers that start an Electron evaluation. The core protocol was built to
  survive a shell swap (ADR-003).
- **Order** (ADR-002). macOS first. Linux is a Phase 7 hardening pass. Windows is
  Phase 8, and only if the workflow has proven its value. The Plan's "Windows as
  Phase 2 platform work" (§4.1) was a slip in the founding document; §15 and
  ADR-002 say Phase 8.
- **Linux today.** CI has been dark since early September (CI-LINUX-DARK, Linux
  CI is dark until the minute allowance resets on 1 Oct). Nothing may claim
  Linux support until a Linux run is green again. The founding "keep Linux
  continuously buildable" is currently not true.
- **Window framework** (ADR-032). React 19 renders the chrome. Terminal output,
  Monaco and the Markdown preview are imperative islands, and new fast surfaces
  follow that pattern. Four reopening criteria are recorded, the first of which
  (typing cost attributed to reconciliation) is still unmeasured.

## 4. Architecture as built

### 4.1 Repository map

| Path | Role | Against Plan §6 |
|---|---|---|
| `crates/gritty-core` | Tauri-free core: protocol, events, operations, workspace, documents, session, terminal, git, search, settings, workstreams, ownership, agent sessions, agent runs | planned |
| `crates/workspace` | Canonical root, path policy, lazy listing, `files.*`, `notify` watcher, fixture generators | planned |
| `crates/persistence` | SQLite, forward-only migrations | planned |
| `crates/terminal` | PTY sessions, output batching, process-tree kill | planned |
| `crates/git` | Status, diff, log and show through `gix` | planned |
| `crates/search` | Cancellable in-process walk on ripgrep's engine; no index | planned as `search-index` |
| `crates/git-write` | Stage, unstage, discard, commit through the user's `git` binary | not planned (ADR-029) |
| `crates/settings` | Settings schema registry and atomic `settings.json` writer | not planned (ADR-031) |
| `crates/agents` | Discovery of observed sessions, verdict files, hook installer | not planned (ADR-018) |
| `crates/agent-hook` | Sidecar the agent's hook runs; writes a state file | not planned (ADR-018) |
| `crates/agent-fake` | Deterministic scripted agent | not planned as a crate (ADR-005) |
| `crates/agent-claude` | Claude Code adapter for owned runs | not planned (T310) |
| `apps/desktop/src-tauri` | Composition root and command adapters | planned |
| `apps/desktop/src` | React shell, one folder per feature | planned |
| `packages/protocol` | Typed client, generated types | planned |
| `packages/editor` | Monaco binding, no React | planned |
| `packages/markdown` | markdown-it plus DOMPurify, link resolution | planned |

**Planned and not created:** `process-supervisor` (process-tree kill lives in
`crates/terminal`, run supervision in `gritty-core::agent_run`), `platform`
(platform code sits in `apps/desktop/src-tauri/src`), `packages/ui`
(deliberately not created, ADR-016) and `packages/workstreams`. The rule stays
the one in `AGENTS.md`: a crate or package is created by the task that first
needs it. The `platform` crate becomes due when a second platform's code needs
isolating (Phase 7), not before.

### 4.2 Runtime boundaries

| Component | Responsibility | Must not |
|---|---|---|
| WebView (React, Monaco, xterm) | Render, collect input | Touch the filesystem, run processes, scan repositories, block on core work |
| Desktop shell (`src-tauri`) | Window, tracing, command adapters, notifications | Contain domain logic |
| Gritty core (Rust) | Workspace, documents, operations, persistence, supervision, orchestration | Carry provider-specific concepts in domain types |
| Agent process | Execute a task through granted tools | Mutate Gritty state except through a translated event |
| PTY / shell | Interactive terminal sessions | Run inside the renderer |
| Watcher and search | Report changes, search the tree | Block opening or editing a file |
| SQLite | Durable metadata | Become the source of truth for repository files |

### 4.3 The protocol

- **Conventions** (ADR-009). Commands are `domain.verb` in TypeScript and
  `domain_verb` in Rust. Ids are UUID v7 and timestamps are epoch milliseconds.
  One event channel, `gritty://core/event`, carries
  `{name, seq, workspaceId?, correlationId?, actor, payload}`. A MINOR bump
  means additive. A command that returns state is the authority for that state.
- **Version.** 2.27.0 on `main` on 2026-09-15, with 2.28.0 claimed by
  WATCH-DEGRADED (say when part of the tree isn't being watched).
  `docs/merge-queue.md` is the record.
- **Command domains:** `core`, `operations`, `workspace`, `documents`, `files`,
  `shell`, `session`, `terminal`, `workstreams`, `decisions`, `ownership`,
  `agents` (observed sessions), `runs` (owned runs), `git`, `settings`, `search`.
  Plan §6.2's `workspace.read/write/watch` became `documents.*`, `files.*` and
  a watcher that attaches on open. Its `agents.start/cancel/respond` became
  `runs.*`.
- **Error envelope:** `CoreError { code, message, details?, retryable,
  correlationId? }`, with codes `InvalidInput`, `NotFound`,
  `PermissionDenied`, `Conflict`, `Cancelled`, `Timeout`, `Io`, `Internal` and
  `ProtocolMismatch`.
- **Event families in code** (`crates/gritty-core/src/event/mod.rs`):
  - operation: started, progress, completed, failed, cancelled
  - workspace: opened, closed, entries changed
  - document: opened, changed, saved, external change detected, closed
  - persistence status
  - terminal: output, exited
  - window: paths dropped, drop hover
  - workstream: created, changed, removed
  - decision: requested, resolved
  - ownership changed
  - settings changed
  - agent: sessions changed, timeline appended, run changed, run event appended
  - search matches found

  The addendum's `FileSystemBatchChanged`, `GitStatusChanged`,
  `ChangeSetUpdated` and `AgentRunStarted/Finished` do not exist.
- **Gritty is not event-sourced.** Events notify. SQLite and domain state are
  authoritative for metadata; repository files are authoritative for the project.

## 5. Domain model

Status as of 2026-09-15. *Built* means in code on `main`. *Partial* means some
of the contract is missing. *Planned* means nothing exists.

| Entity | Status | Owner | Difference from the founding documents |
|---|---|---|---|
| Workspace | Built | ADR-010, `workspace.md` | One root at a time. `WorkspaceId` lives per process and is not persisted. Settings are user-global (ADR-031), not per workspace. M1 must admit a second root per repository for worktrees (ADR-035). |
| Document | Built | ADR-011, ADR-028, `documents.md` | UTF-8 and UTF-8-with-BOM only. Path is mutable through `files.rename/move`. Conflict resolution offers reload, keep or overwrite; the addendum's compare and merge are not built. |
| Operation | Built | ADR-009, ADR-014 | Adds `INTERRUPTED`. `workspaceId`, progress and error are optional. Document I/O and agent runs are deliberately *not* operations. |
| Actor | Built | ADR-009 | Exactly the addendum's `USER / SYSTEM / AGENT(runId) / EXTERNAL`. |
| Workstream | Built | T200, `workstreams.md` | States `DRAFT, READY, IN_PROGRESS, BLOCKED, IN_REVIEW, DONE, ABANDONED`. No `PLANNING` and no `NEEDS_DECISION` (an open decision is `BLOCKED`). One reversible `ABANDONED` replaces `CANCELLED/FAILED`. No plan or branch fields yet; T400 adds the worktree and branch. |
| Decision | Built | T200, T210 | Options, chosen, rationale, raised-by actor. No "related files". |
| Agent run (owned) | Partial | T300-*, T310-*, ADR-021 | States `STARTING, RUNNING, WAITING, NEEDS_DECISION, BLOCKED, REVIEW, FINISHED, FAILED, CANCELLED, INTERRUPTED`, carrying the Plan §10 semantics the workstream dropped. Providers `FAKE` and `CLAUDE_CODE`. Persistence is accepted in ADR-021 and **not built** (schema is still 4). No model, permissions or manifest fields. |
| Agent session (observed) | Built | ADR-018, T220–T233 | Not in either founding document. Status is labelled with its evidence (process, transcript, hook). |
| Ownership lease and conflict | Built, observed only | ADR-019, `ownership.md` | Held by a workstream, never a run. It refuses a second claim but does not stop a write. Enforcement is T330-R (the fence, the matcher, and the hook's refusal) under ADR-018A, deferred by ADR-035. |
| Permission verdict | Built | ADR-020, T335, T310-ANSWER | Per-ask human answers (`ALLOW_ONCE`, `ALLOW_FOR_RUN` for observed sessions, `DENY`). This is what shipped instead of addendum §8's capability grants. |
| Capabilities | Planned | T305 (agent capability model) | Addendum §8's scoped `filesystem.write` model was never built. In practice it is replaced by verdicts, leases and the ADR-018A fence. Keep or replace: open decision, deferred (§12). |
| Context manifest | Planned | T320 (context manifest) | Nothing exists. ADR-021 persists only a run's first prompt. |
| ChangeSet | Planned | ADR-007, DIFF-BASE | Not built. ADR-035 makes a task's diff against its base the first ChangeSet: base revision, files and a diff summary, not raw `git diff` text. |
| Step | Not scheduled | — | Plan §7's plan-item entity. A run's event ring and timeline cover the observable part. |
| Terminal session | Built | ADR-015, ADR-017, `terminal.md` | Buffer not persisted. Up to 16 sessions. Home-directory shells since T095. |
| Settings | Built | ADR-031 | `settings.json`, user scope only. `.gritty/` is not introduced. |
| Project decision (ADR) | In practice | `docs/decisions/` | Markdown in the repository, as Plan §7 wanted. |

## 6. Contract gates and freeze points

| Gate | Contract | State |
|---|---|---|
| A | T010 core protocol: commands, events, ids, error envelope, operations, cancellation, versioning | Passed (ADR-009). Frozen: crossing it needs deliberate review and usually an ADR. It was crossed once on purpose, when T095 made `workspaceId` optional and moved to 2.0.0 (ADR-015). |
| B | T030 document and buffer: identity, dirty state, safe save, external change, encoding, recovery inputs | Passed (ADR-011). Amended by ADR-028 for file moves. |
| C | T200 workstream: state machine, transitions, persistence, decisions | Passed. Identity and transitions frozen. The ChangeSet relation arrives with DIFF-BASE (a task's diff against the base it was cut from). The worktree relation arrives with T400 (a workstream owns a worktree and a branch). |
| D | T300 agent run: provider-neutral run and event model, proven with a fake agent first | **Passed in part.** The run lifecycle and the normalized event vocabulary are frozen. Capability representation and manifest structure, which the addendum froze here, do not exist; they freeze when T305 and T320 land. |
| O | T325 workspace ownership (observed) | Passed (ADR-019). Does not gate on T300: a lease is held by a workstream. |

Downstream work consumes these contracts and does not reinterpret them
(`AGENTS.md` hard rule 2).

## 7. Cross-cutting policies

These are pointers. The owner document is authoritative.

- **Persistence and recovery.** ADR-004, ADR-014 and ADR-021. SQLite with
  forward-only migrations, atomic writes, debounced buffer snapshots, a corrupt
  database moved aside, and non-terminal records reconciled to `INTERRUPTED` on
  start. Unsaved buffers are never silently discarded (`AGENTS.md` rule 7).
- **Security.** `docs/architecture/security.md`, and `AGENTS.md` rules 1 and 13:
  - repository content and agent output are untrusted;
  - paths are validated against the root;
  - raw HTML never renders;
  - secrets never reach logs.

  The document has gaps (Appendix B).
- **Performance.** Budgets from Plan §13, beside the measured baseline from
  `docs/plans/phase-0-spike.md` §C (2026-09-04, Apple silicon, 16 GB, release
  bundle):

  | Budget | Target | Measured |
  |---|---|---|
  | Cold launch | ≤ 2 s | 496 ms median |
  | Warm launch | ≤ 1 s | 504 ms median |
  | Idle CPU | ~0 % | 0.064 % of one core over 22.5 min |
  | Open a file | ≤ 100 ms perceived | 0.23 ms core side; perceived not measured |
  | Repository enumeration | never blocks | `workspace.list` 0.29 ms on 10,501 files |
  | Memory | track, compare with Electron | 607 MiB RSS versus Cursor's 1194 MiB; settles to 228 MB footprint; no idle growth |
  | Typing | no dropped frames | **not measured** |

- **Observability.** Structured tracing exists (`GRITTY_LOG`). The diagnostics
  surface is T110 (diagnostics), in M4.
- **Dependencies.** `AGENTS.md` § Dependency policy: pnpm catalog, root
  `Cargo.toml`, exact pins, a one-line justification for each new dependency.
- **Fixtures.** Built, in `fixtures/`: `small-repo`, a generated `large-repo`
  of 10,501 files, `weird-files` and `markdown`. Git fixtures are generated in
  `crates/git`.
- **Verification.** Automated tests for domain logic. A human feature check in
  the running app for each feature as it lands, one at a time. No large batched
  manual pass is a gate (ADR-001 retired `phase-0-manual-pass.md` as one;
  ADR-035 has every milestone end in feature checks). A check records the
  observation, the date and the machine.

## 8. Phases: where each capability area stands

The founding phases remain the map of *what* Gritty must eventually contain.
Since ADR-035 they no longer set the *order*: §10 does. Effort figures from the
founding plan are historical and are dropped.

| Phase | Exit criterion | Status 2026-09-15 | What stands between it and the exit |
|---|---|---|---|
| 0 Runtime spike | ADR-001 go/no-go with measurements | **Closed** by ADR-001 | The human checks it left (IME, dead keys, typing frame time, a real `kill -9` recovery) now fall to the feature checks of the milestones that touch them. |
| 1A Workspace and documents | Safe for basic code editing | **Partly met** | Everything automated. The Input and Editor rows of `docs/testing/platform-matrix.md` have never been checked by a person. |
| 1B Markdown, persistence, recovery | Force-kill and restart do not make routine editing feel unsafe | **Partly met** | Crash recovery is automated. The real force-kill check is outstanding. |
| 2 Developer essentials | Usable for daily repository work | **Not met** | See §10 M0 and M4: search panel, commit UI and settings in flight; ignore rules and diagnostics in M4. ADR-035 replaces the five-day dogfood gate with M3's exit. |
| 3 Workstreams | A workstream managed manually from goal to done | **Partly met** | Workstreams, decisions and the attention queue exist. The ChangeSet does not (M3). |
| 4A Agent protocol and fake agent | Orchestration testable offline | **Partly met** | Fake agent and run model built. Capabilities, context manifest and run persistence are not (deferred, except ADR-021, §12). |
| 4B First real agent | One bounded task delegated, interrupted, resumed, cancelled and reviewed without blocking the UI | **Not met** | A Claude run exists, but its panel is mounted only in development builds and no run has been watched end to end (RUN-LAUNCHER, M0). No review flow (M3). |
| 5 Parallel orchestration | Several workstreams easier than several windows | **Not started** | Worktrees are M1. The scheduler (T405) is deferred. |
| 6 Language intelligence | Navigation and diagnostics improve without extensions | Not started | No trigger yet. |
| 7 Linux hardening | Supported Linux release | Not started | CI returns 1 Oct. Linux packages are M5. |
| 8 Windows | Only if the workflow has proven its value | Not started | After M5 at the earliest. |

### Recorded exceptions

These are gate crossings that were decided rather than drifted into, carried
over from `roadmap.md` so they remain written down:

1. **Agent observability built without the T300 gate.** T220–T233 observe
   agents Gritty does not own, so none of them rests on Gate D (roadmap,
   2026-09-06).
2. **Phases 1–2 started before ADR-001 was accepted.** Alexis' decision of
   2026-09-06, recorded in advance. ADR-001 was accepted on 2026-09-09.
3. **The first real adapter built without the capability model.**
   `T310-claude-code-adapter.md` makes T305 deliberately not a dependency.
4. **Permission verdicts built under the hook bridge, not under T300.** T335
   rests on ADR-020, which established that a hook may be answered.
5. **Orca parity reorders Phases 3–5** (ADR-035). Worktrees come before
   capabilities, manifest and write interception.

## 9. Dependency graph

This is the single canonical graph. It merges the one in `AGENTS.md`, the one
formerly in `docs/tasks/README.md`, and the board's `depends` fields. Built work
is collapsed; the board's `depends` field stays authoritative for any single
ticket.

```
T000 Bootstrap
 └─ T010 Core protocol ............................................ Gate A
     └─ T020 Workspace
         ├─ T030 File I/O + document/buffer .......................... Gate B
         │   ├─ T040 Monaco
         │   └─ T050 Markdown
         ├─ T070 File watcher
         └─ T080 Search ─ T080-U search panel
T060 Persistence · T090 Terminal · T100 Git · T110 Diagnostics

T100 Git
 ├─ T101-R commit walk + blob at revision ─ T101-D diff as an editor tab
 ├─ T102-R/T102-U staging and commit ────── HUNK-STAGE (needs T101-D)
 └─ T400-R worktree create and remove
      └─ T400 a workstream owns a worktree and a branch (also needs T200)
           ├─ WORKTREE-SETUP setup command on create
           ├─ AGENT-TERMINAL agent CLI in the task's terminal (also needs T090)
           └─ DIFF-BASE diff against the base (also needs T101-R)
                └─ REVIEW-COMMENTS (also needs T101-D, AGENT-TERMINAL)

T200 Workstream model ........................................ Gate C
 └─ T210 Workstream UI
     ├─ T220 Observed agent sessions (does not gate on T300)
     │    ├─ T230 Timeline ─┬─ T221 Hook bridge (ADR-018)
     │    │                 │    ├─ T223 Notifications + workstream link
     │    │                 │    └─ T335 Permission verdicts (ADR-020)
     │    │                 └─ T231/T232/T233 roster, timeline, console
     │    ├─ T222-SEAM provider registry ─ T222-CODEX
     │    └─ T224 Reveal the terminal
     ├─ T300 Agent run contract + fake agent .................... Gate D
     │    ├─ T305 Capability model            (deferred)
     │    ├─ T310-A/B/U Claude Code adapter (did not wait for T305)
     │    │    ├─ T310-ANSWER host permission verdicts ─ RUN-NOTIFY (also T223)
     │    │    ├─ RUN-LAUNCHER start a run from a release build
     │    │    └─ T310-RESUME                  (deferred)
     │    └─ T320 Context manifest            (deferred)
     └─ T325 Workspace ownership, observed ......................... Gate O
          ├─ T326 Observed violations
          ├─ T330-MEASURE → T330-R → T330-U write interception (deferred)
          ├─ T405 Ownership-aware scheduler (also T300)      (deferred)
          └─ T410 Attention queue (also T210, T335)
```

T315 (live activity) is superseded: the observed-sessions tree above built it.
`T330` means write interception, and decision interruption is `T335`; older
completion reports keep the numbers they were written with.

## 10. Forward sequence

**ADR-035 owns the sequence**, and this section only points to it: M0
demoable, M1 a task is a worktree, M2 agents in terminals, M3 the review loop,
M4 ship code and polish, M5 release. Its *Deferred until M3 exits* list and its
lighter process rules (§5 of the ADR) apply. Read the ADR for the ticket list
of each milestone. The board holds each ticket's status.

What this plan adds around it:

- **M0 includes this plan's hygiene stage.**
  - Commit work that sits at `review` uncommitted (`review` is not
    committed; check `git status` for a ticket's files).
  - Drain the review queue.
  - Close board rows that no longer tell the truth.
  - Make `README.md` describe what exists.
- **The review cap is adopted.** From M0's exit on, no new `take` while more
  than ten tickets sit at review (ADR-035 §5).
- **The owned-run specs already written** fold into M2 where they serve a run
  launched from a task, and are otherwise dropped at M2's start (ADR-035
  *Consequences*). These are T310-VERDICT-EVENT (record a permission verdict in
  a run's ring), T310-U-CANCEL (stop a run from the console it is showing in),
  T310-RUN-PATHS (a run's file paths, made workspace-relative) and
  T310-U-FILES (files touched, on an owned run's console).
- **Run persistence is not on the deferred list.** ADR-021 is accepted and has
  no implementing ticket. A run launched from a task (M2) that vanishes on
  restart fails the recoverability principle, so ADR-021's implementation
  belongs in M2. That needs a ticket (Follow-up).
- **Two amendments come before M1's code**, as ADR-035 records: ADR-029's
  subcommand allowlist admits `worktree` and `branch`, and the workspace model
  admits a second root per repository.
- **Linux, from 2026-10-01.** When the Actions allowance resets:
  - run CI;
  - evaluate ADR-001's third kill criterion;
  - confirm the PTY-FLAKE process-group fix and the inotify watcher path on
    Linux.

  This is groundwork for M5's Linux packages, not a milestone of its own.
- **Numbering.** The next free ADR is 036 (`docs/merge-queue.md`). The
  capabilities decision, when it is taken after M3, takes whatever number is
  free then.

## 11. Founding open questions, resolved or not

| Question (Plan §21, addendum §14) | Resolution |
|---|---|
| Desktop runtime | ADR-001: Tauri 2 (narrowed acceptance) |
| Platform order | ADR-002 |
| PTY crate and abstraction | ADR-015: `portable-pty`, process-group kill |
| Git: shell out or a library | ADR-023: `gix` for reads. ADR-029: the user's `git` for writes. Networked git: ADR-030, unwritten (M4) |
| Search: ripgrep process, library or hybrid | ADR-022: in-process library, no index. ADR-034: "ignored" takes Git's meaning |
| Markdown stack and link syntax | ADR-013: markdown-it plus DOMPurify, `path:line:col` |
| SQLite crate and migrations | ADR-014: `rusqlite`, hand-rolled forward-only runner |
| Frontend state library | ADR-032: none; a reducer plus a hook per feature |
| First agent backend | Claude Code, in practice (T310). No ADR |
| Agent event protocol | An internal translation layer over the provider's own stream (`agent-runs.md`). ADR-027 is reserved in case the run feed needs changing |
| Symlinks, case, normalization, files outside the root | ADR-010 |
| Encodings and large-file thresholds | ADR-011 |
| Repository-local `.gritty/` | ADR-031: not introduced |
| Shell and environment inheritance, login shell, PATH, cwd, process-tree termination | ADR-015 (and ADR-029 for `git`'s environment) |
| Proxy variables | Open |
| Linux distro and WebKitGTK policy | Open (M5) |
| Workspace trust and permission UX | Open. A prerequisite for workspace-scope settings (ADR-031) |
| Signing, notarization, update channels, rollback | Open (M5) |
| Telemetry | Open. Default stays local diagnostics only |
| When LSP enters the critical path | Open (Phase 6) |
| Accessibility and input: remappable keys, screen readers, IME, dead keys, zoom, contrast | Partly open. Keybinding registry in M4. IME and dead keys unevaluated (ADR-001) |

## 12. Open decisions

| Decision | Next step | When |
|---|---|---|
| Capabilities: keep addendum §8's scoped model, or formally replace it with verdicts, leases and the fence | An ADR, then T305 built or superseded | After M3 (ADR-035) |
| ChangeSet beyond a task's diff against its base | Spec, informed by DIFF-BASE | After M3 |
| Context manifest structure | Spec for T320 | After M3 |
| Networked git: crate boundary, allowlist, credentials | ADR-030 | M4 |
| Agent run feed: event ring or transcript | ADR-027, only if the coarse feed proves insufficient | When it does |
| Run persistence implementation | Ticket for ADR-021 (migration `0005`, schema 5) | M2 (§10) |
| Workspace trust | ADR before any workspace-scope setting | Before workspace scope |
| Distribution: signing, notarization, updater, rollback, migration compatibility | Tickets | M5 |
| Linux support policy | Decide after CI returns | M5 |
| Proxy environment variables for shells and agents | Decide with AGENT-TERMINAL, which is where it bites | M2 |
| Keybinding remapping and the shortcut registry ADR-026 calls owed | Ticket | M4 |
| `operation_record` grows without bound (ADR-021) | Retention rule | Next persistence ticket |
| `MAX_CONCURRENT_RUNS = 4` was never justified | Decide with parallel tasks | M2 |
| Telemetry | Keep local-only unless a concrete need appears | — |
| LSP | Trigger: a navigation or diagnostics need the regex outline cannot meet | Phase 6 |

## 13. Success criteria

Gritty is not judged by feature count. After sustained use it succeeds if:

- editor failures are rare and never interrupt unrelated work;
- basic code and Markdown editing is close enough to a conventional IDE that
  switching tools is not needed;
- the state of several coding tasks is understandable without reading
  transcripts;
- agent questions, blockers and review-ready changes surface clearly;
- three concurrent tasks are materially easier in Gritty than in three windows;
- decisions and task state survive provider changes and restarts;
- resource use stays acceptable with agents and terminals running;
- **the core loop of §1 works end to end in a release build** (ADR-035);
- the improvement is large enough to justify a Windows port.

## 14. Keeping this plan true

- Update this plan in the same change as:
  - a phase changing status;
  - a gate or freeze changing;
  - an ADR that amends a section here;
  - a milestone of ADR-035 exiting.

  Bump the *as of* date when you do.
- **Do not copy** the graph, the phase table or the sequence into another
  document; point to the section instead. The founding plan, its addendum and
  the roadmap drifted precisely because the same material lived in four places.
- **Never write a bare ticket id where a person reads it** (`AGENTS.md`).

## Appendix A. The three source documents

| Document | Contributed | Now |
|---|---|---|
| `docs/Gritty IDE Project Plan.md` | Thesis, goals, platform choice, principles, runtime boundaries, the first data model, the workstream state machine, phases 0–8, T000–T410, the agent contract, definition of done, open decisions, success criteria | Frozen, history |
| `docs/Gritty IDE Architecture Refinements.md` | Contract gates A–D, the operation model, the document and buffer model, actors, event vocabulary, ChangeSet semantics, capabilities, dependency policy, fixtures, Phase 0 failure tests, fake agent first, phases 1A/1B/4A/4B, freeze points, go/no-go questions, the eight questions | Frozen, history |
| `docs/Gritty Architecture Refinements.md` | An untracked, shortened copy of the addendum (14 Sep). A word-level diff found rewording only, and a few dropped lines | Deleted by PLAN-OF-RECORD |

Contradictions between the founding documents, and how each was settled:

| Contradiction | Settled by |
|---|---|
| Windows "Phase 2 platform work" (Plan §4.1) against Phase 8 (Plan §15) | ADR-002: Phase 8 |
| "macOS + Linux first" (Plan header) against "ship macOS, keep Linux buildable" (§4.1) | ADR-002: macOS first |
| File watching in the Phase 0 checklist (Plan §16) against T070 after ADR-001 (addendum §18) | The watcher was measured in the spike and built as T070. The roadmap amendment of 2026-09-06 started it before ADR-001 |
| T060 persistence and T090 terminal in Phase 0 (addendum §18) against Phases 1 and 2 (Plan §17) | Both were built as Phase 0 spikes, then hardened (T095, T096, T098 for the terminal) |
| T300 under T210, T400/T410 under decision interruption (addendum §2) | ADR-019 and the Coordination plan §36: T325 is a sibling of T300, and T400/T410 hang from T325 |
| Command palette in Plan Phase 1, absent from the addendum | Built as quick-open (T130-*). A real command palette is in M4 |
| Agent runs listed as operations (addendum §3) | `agent-runs.md`: a run has its own lifecycle and is not an operation |
| Workstream states (Plan §10) | T200 chose a different set (§5), and the run carries the execution states |

## Appendix B. Documentation drift, recorded for follow-up

These were found while writing this plan and deliberately not fixed in it:

- **`docs/architecture/overview.md`.** Its event vocabulary is the addendum's,
  not the code's. It presents ChangeSets and capabilities as current. It still
  says the planning documents are the authority.
- **`docs/architecture/security.md`.** No section on hook writes into other
  tools' configuration, on `git` hooks running repository code, or on
  `settings.json` (ADR-031 says security.md records it). Its `data:` rule
  disagrees with ADR-013. "A shell starts inside the workspace" has been stale
  since T095 (shells with no workspace). Capabilities are promised as if built.
- **`docs/architecture/terminal.md`.** Calls ADR-017 a draft. Describes the old
  two-group kill. Places the shell choice in the layout blob instead of
  `settings.json`.
- **`docs/architecture/ownership.md` and `agent-sessions.md`.** "The rail"
  (retired by ADR-026). No mention of ADR-018A. Notifications "off by default"
  (superseded by ADR-031).
- **`docs/architecture/agent-runs.md`.** "One variant, `FAKE`". "No
  `runs.start`". Persistence "decided before T310".
- **`docs/architecture/persistence.md`.** "Protocol 1.3.0" heading.
- **`docs/architecture/documents.md`.** The cause list omits `RECOVERED`.
- **`docs/decisions/README.md`.** The amendment index misses ADR-009, 015, 016,
  018's amendments, and ADR-020→018, ADR-028→011, ADR-032→001.
- **ADR text.** ADR-021 §8 says "Proposed". ADR-032 says ADR-001 is still open.
  ADR-024's body keeps an argument ADR-033 withdrew. ADR-008's CI facts predate
  CI-COST and the Linux outage.
- **`docs/tasks/README.md`.** Several rows link to `docs/tasks/docs/tasks/…`,
  which is broken.
- **`docs/testing/platform-matrix.md`.** Not updated since T045 (drag and drop).
- **`docs/plans/Gritty Agent Runtime Coordination.md`.** No date or status line.
  Its §36 order is overridden by ADR-035's milestones wherever the two differ.
