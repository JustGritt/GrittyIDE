GRITTY IDE

Project Definition, Architecture & Execution Plan

Agent-native development environment • macOS + Linux first • Windows
later

Planning baseline --- 2 September 2026

North star

A dependable code editor that becomes an orchestration cockpit when
agents are available --- and remains fully useful when they are not.

# 1. Executive Summary

Gritty is an agent-native desktop development environment designed
around the shift from manually producing every line of code to
supervising multiple streams of software work. The product must not
become an AI chat application wrapped around an editor. Code reading,
editing, search, terminal access, Git inspection and Markdown are the
dependable base; agent orchestration is an additional layer.

The initial platform strategy is Tauri with a Rust core and a
TypeScript/React frontend using Monaco. The first supported targets are
macOS and Linux. Windows is intentionally deferred until the workflow
and product model have proven themselves through sustained daily use.

The plan is deliberately reliability-first. Heavy or failure-prone
work---agents, language servers, terminals, repository indexing and
external tools---must execute outside the UI process. Workspace state
must be recoverable after crashes. Markdown is a first-class operational
format because plans, specifications, decisions and agent instructions
naturally live there.

# 2. Product Thesis

Traditional IDEs optimize for a developer manipulating files. Agentic
development increasingly requires the developer to manipulate intent,
tasks, decisions, reviews and concurrent workstreams.

-   Primary unit of work: a workstream/task, not a chat session.

-   Files and diffs remain first-class, but are supporting views of
    work.

-   Agent activity is represented structurally: plan, status, files
    touched, commands, findings, blockers, tests and decisions.

-   Human attention is scarce; the UI should surface only decisions,
    failures, conflicts and review-ready work that require intervention.

-   Project knowledge must persist independently from any individual
    model conversation.

-   Models and agent CLIs are replaceable execution backends; Gritty
    owns project state and orchestration.

-   The application must remain a competent code editor if every AI
    provider is unavailable.

# 3. Goals and Non-Goals

  -----------------------------------------------------------------------
  Goals for v1                        Explicit non-goals
  ----------------------------------- -----------------------------------
  Reliable daily-use editor for macOS VS Code extension compatibility
  and Linux                           

  Fast code reading/editing with      Building a new text editor engine
  Monaco                              

  Excellent Markdown editing and      Supporting every programming
  preview                             language semantically

  Terminal, project explorer, search  Replacing Git or inventing a VCS
  and Git diff/status                 

  Crash-resistant process             Autonomous agents with unlimited
  architecture                        permissions

  Persistent workstreams and          Multi-user collaboration
  recoverable sessions                

  Agent execution behind a            Windows support before product
  provider-neutral protocol           validation

  Dogfoodable as early as practical   Feature parity with VS Code/Cursor
  -----------------------------------------------------------------------

# 4. Platform Decision

## 4.1 Initial target

Use Tauri for the desktop shell. Ship and dogfood macOS first, keep
Linux continuously buildable, and treat Windows as Phase 2 platform work
after the core workflow is validated.

## 4.2 Why Tauri

-   Lower baseline memory and distribution overhead than bundling a full
    Chromium runtime.

-   Rust is a strong fit for filesystem traversal, process supervision,
    indexing, Git integration and persistence.

-   A strict frontend/core boundary discourages business logic from
    leaking into UI components.

-   The long-running IDE plus multiple agents/LSPs/terminals makes
    memory efficiency increasingly valuable.

-   Tauri supports spawning and supervising external binaries/sidecars,
    which fits agent CLIs and language servers.

## 4.3 Known risks

-   macOS uses WKWebView while Linux webview behavior depends on the
    platform stack; Monaco and keyboard/input behavior require real
    testing.

-   Rust + TypeScript increases build-system and contributor complexity.

-   Native dependencies, PTY support, signing and packaging can create
    platform-specific maintenance.

-   A later Windows port will require explicit WebView2, path, shell,
    PTY, filesystem-watcher and signing validation.

Mitigation: keep the Gritty Core API independent from UI details, build
automated smoke tests around the editor and workspace primitives, and
avoid platform-specific behavior unless isolated behind adapters.

# 5. Architecture Principles

  -----------------------------------------------------------------------
  Principle                           Meaning
  ----------------------------------- -----------------------------------
  Editor before agent                 Every core editing workflow must
                                      work without an AI account or
                                      network connection.

  Failure containment                 Agent, LSP, PTY, Git and indexing
                                      failures must not take down or
                                      freeze the UI.

  Persist intent, not transcripts     Store goals, plans, decisions,
                                      state and results; raw
                                      conversational logs are secondary.

  Portable project artifacts          Prefer Markdown and normal
                                      repository files over proprietary
                                      formats when practical.

  Measure before optimizing           Use Rust naturally for the core,
                                      but do not create bespoke
                                      high-performance subsystems without
                                      profiling.

  Explicit permissions                Agents must operate under visible
                                      workspace/process permissions and
                                      auditable actions.

  No hidden magic                     Users should be able to inspect
                                      context, changes, commands and
                                      decisions behind agent work.

  Recoverability                      Unexpected termination should lose
                                      as little state as reasonably
                                      possible.
  -----------------------------------------------------------------------

# 6. Proposed Technical Architecture

Recommended repository shape:

``` text
apps/
  desktop/                 # Tauri application + React shell
packages/
  ui/                      # reusable UI primitives
  editor/                  # Monaco integration
  markdown/                # Markdown editor/preview/link handling
  protocol/                # generated/shared API types and event contracts
  workstreams/             # frontend domain layer
crates/
  gritty-core/             # application orchestration
  workspace/               # filesystem/project model
  process-supervisor/      # agents, LSPs, external commands
  git/                     # Git abstraction
  search-index/            # repository search/indexing
  persistence/             # SQLite + migrations
  terminal/                # PTY abstraction
  platform/                # macOS/Linux/Windows adapters
docs/
  architecture/
  decisions/               # ADRs
  plans/
  agent/
```

## 6.1 Runtime boundaries

  -----------------------------------------------------------------------------------
  Component               Responsibility                      Must not
  ----------------------- ----------------------------------- -----------------------
  Tauri/WebView UI        Render React, Monaco, Markdown,     Scan repositories, run
                          workstream/review UI                agents, block on
                                                              Git/LSP/PTY work

  Gritty Core (Rust)      Own workspace lifecycle,            Contain
                          commands/events, persistence and    provider-specific agent
                          supervision                         logic in the domain
                                                              model

  Agent process           Plan/execute bounded task using     Directly mutate Gritty
                          granted tools                       UI state

  LSP process             Language                            Become required for
                          diagnostics/navigation/completion   plain text editing

  PTY/shell               Interactive terminal sessions       Run inside renderer

  Indexer/search worker   Search/index repository             Block opening/editing
                          incrementally                       files

  SQLite                  Durable app/workstream/session      Become source of truth
                          metadata                            for repository files
  -----------------------------------------------------------------------------------

## 6.2 Frontend/core protocol

Define typed commands and events early. UI code calls domain APIs rather
than Tauri primitives directly. Example conceptual surface:

``` text
workspace.open(path)
workspace.list(path)
workspace.read(path)
workspace.write(path, content)
workspace.watch()

search.query(text, options)
git.status()
git.diff(target)
terminal.spawn(profile)
terminal.write(id, data)

workstreams.create(goal)
workstreams.update(id, patch)
agents.start(workstreamId, config)
agents.cancel(runId)
agents.respond(runId, decision)
```

All long-running operations should return quickly and stream
progress/events. Cancellation must be designed into the protocol rather
than added later.

# 7. Core Data Model

  -----------------------------------------------------------------------
  Entity                              Key fields / purpose
  ----------------------------------- -----------------------------------
  Workspace                           id, root path, name, Git metadata,
                                      settings, last opened

  Workstream                          id, workspace, goal, status, plan,
                                      branch/worktree, created/updated
                                      timestamps

  Agent Run                           provider/backend, model, status,
                                      permissions, context manifest,
                                      process id, result

  Step                                plan item, status, timestamps,
                                      evidence/output

  Decision                            question, alternatives, selected
                                      option, rationale, related
                                      files/workstream

  Change Set                          files changed, base revision, diff
                                      summary, test status, review status

  Context Manifest                    explicit
                                      files/docs/symbols/instructions
                                      provided to an agent

  Terminal Session                    shell/profile, cwd, lifecycle
                                      metadata; terminal buffer
                                      persistence optional

  Project Decision (ADR)              durable architectural/product
                                      decision stored preferably in
                                      repository Markdown
  -----------------------------------------------------------------------

# 8. Editor & Markdown Experience

## 8.1 Code editor baseline

-   Monaco editor with syntax highlighting, line numbers, minimap
    toggle, folding, multi-cursor, find/replace, bracket matching and
    standard keybindings.

-   Tabs, dirty-state indication, autosave policy, safe file writes and
    external-change detection.

-   Large-file guardrails: disable expensive features progressively
    rather than hanging.

-   Diff editor for Git and agent changes.

-   Command palette and keyboard-first navigation.

-   LSP integration only after the editing substrate is stable.

## 8.2 Markdown as an operational surface

-   Side-by-side source/preview and preview-only modes.

-   GitHub-flavored Markdown, task lists, tables, fenced code, syntax
    highlighting and Mermaid later if justified.

-   Clickable repository paths and optional :line/:column references.

-   Fast navigation between Markdown plans/specs and referenced source
    files.

-   Recognize common project instruction files such as AGENTS.md and
    README.md without hard-coding behavior to one vendor.

-   Allow agent plans/results to be exportable or materialized as normal
    Markdown.

-   Later: promote Markdown checklist items into workstreams and render
    structured workstream state back into Markdown-compatible
    representations.

# 9. Agent System

Agent integration begins only after the editor foundation is usable.
Gritty should treat agents as supervised external workers, not as the
application runtime.

  -----------------------------------------------------------------------
  Concern                             Design
  ----------------------------------- -----------------------------------
  Backend neutrality                  Adapter interface for CLI/API
                                      agents; no provider concepts in
                                      Workstream core

  Process model                       One supervised process/run;
                                      stdout/stderr/event parsing
                                      isolated from UI

  Permissions                         Workspace roots, command execution,
                                      network/provider capability and
                                      write permissions explicit

  Context                             Persist a manifest of what was
                                      supplied; expose it to the user

  Output                              Normalize progress, file changes,
                                      tests, findings, questions and
                                      final result

  Intervention                        Agent can enter NEEDS_DECISION /
                                      BLOCKED instead of burying
                                      questions in text

  Cancellation                        Hard requirement from first
                                      implementation

  Recovery                            Detect orphaned/terminated runs and
                                      present restart/reconcile options

  Git isolation                       Prefer
                                      branch/worktree-per-workstream once
                                      parallel execution is introduced
  -----------------------------------------------------------------------

# 10. Workstream State Machine

Suggested initial states: DRAFT → PLANNING → READY → RUNNING →
NEEDS_DECISION / BLOCKED → RUNNING → REVIEW → DONE. CANCELLED and FAILED
are terminal alternatives. State transitions must be explicit and
persisted.

-   DRAFT: goal is being defined.

-   PLANNING: agent/user decomposes the task.

-   READY: plan/context is accepted and execution can begin.

-   RUNNING: one or more supervised operations are active.

-   NEEDS_DECISION: progress is intentionally paused for human choice.

-   BLOCKED: dependency/failure prevents progress.

-   REVIEW: implementation is complete enough for human diff/test
    review.

-   DONE: accepted/merged/closed.

# 11. Persistence & Recovery

-   SQLite for application metadata with explicit schema migrations.

-   Atomic file writes: write temporary file, fsync where appropriate,
    then replace.

-   Persist open workspace, tabs, cursor/selection state, layout,
    workstreams, decisions and run metadata.

-   Do not depend on persisting Monaco internals; persist stable domain
    state.

-   On startup, reconcile persisted process state with reality:
    previously RUNNING processes that no longer exist become
    interrupted.

-   Keep structured logs with bounded retention and a one-click
    diagnostics bundle later.

-   Never silently discard unsaved buffers after a crash.

# 12. Security Model

-   Treat repository content and agent output as untrusted input.

-   Expose a minimal Tauri command surface; never expose arbitrary
    filesystem/process APIs to the webview.

-   Canonicalize and validate paths against allowed workspace roots.

-   Use explicit command/process allowlists or permission grants for
    agent execution.

-   Do not render raw agent HTML in Markdown preview.

-   Keep secrets out of logs and SQLite where possible; use OS
    credential storage for provider tokens when direct API credentials
    are introduced.

-   Display destructive or out-of-scope agent actions before execution
    unless a user has deliberately granted an appropriate persistent
    permission.

# 13. Performance Budgets

These are initial engineering targets, not promises. Benchmark on
representative repositories and adjust based on measured user
experience.

  -----------------------------------------------------------------------
  Metric                              Initial target
  ----------------------------------- -----------------------------------
  Cold launch to usable shell         ≤ 2 s on primary development Mac

  Warm launch                         ≤ 1 s

  Idle CPU                            \~0% absent active watchers/tasks

  Typing responsiveness               No perceptible frame drops during
                                      normal editing

  Open normal source file             ≤ 100 ms perceived for cached/local
                                      files

  Workspace restore                   UI appears immediately; expensive
                                      restoration is incremental

  Repository enumeration              Never blocks editor; progressive
                                      results

  Agent/LSP/indexer crash             No renderer crash or UI freeze

  Memory                              Track baseline and growth
                                      continuously; establish hard budget
                                      after prototype measurements
  -----------------------------------------------------------------------

# 14. Observability & Quality

-   Structured logging across UI/core/process boundaries with
    correlation IDs for workstreams and runs.

-   Developer diagnostics panel: running child processes, watchers,
    IPC/event throughput, database status and recent errors.

-   Unit tests for core domain/state transitions and path/security
    rules.

-   Integration tests for workspace open/read/write/watch, Git
    status/diff, process spawn/cancel and persistence recovery.

-   End-to-end smoke test that opens a fixture repository, edits a file,
    previews Markdown, runs a terminal command and restores state.

-   Performance regression fixture repository containing many files,
    large files and rapid filesystem changes.

-   Manual platform matrix for keyboard shortcuts, clipboard, IME,
    fonts, high-DPI, multi-monitor, sleep/wake and file-system edge
    cases.

# 15. Delivery Plan

  -------------------------------------------------------------------------------------
  Phase             Indicative effort Scope                    Exit criterion
  ----------------- ----------------- ------------------------ ------------------------
  Phase 0 ---       1--2 weeks        Validate Tauri +         Go/no-go on Tauri;
  Architecture                        Monaco + Markdown +      measured baseline.
  spike                               filesystem + PTY on      
                                      macOS; Linux build smoke 
                                      test.                    

  Phase 1 ---       2--4 weeks        Workspace open,          Can use Gritty for
  Dependable editor                   explorer, Monaco, tabs,  simple repository
  shell                               read/write, dirty        reading/editing.
                                      buffers, Markdown        
                                      preview, command         
                                      palette, restoration.    

  Phase 2 ---       2--4 weeks        Search, terminal, Git    Can dogfood for
  Developer                           status/diff, file        meaningful daily tasks.
  essentials                          watching, settings,      
                                      diagnostics, robust      
                                      recovery.                

  Phase 3 ---       2--3 weeks        Persistent workstream    Orchestration exists
  Workstreams                         model, plans, decisions, without AI dependency.
                                      status UI, change-set    
                                      association.             

  Phase 4 --- First 2--4 weeks        One CLI agent,           One bounded task can be
  agent adapter                       supervised runs, context delegated and reviewed.
                                      manifest, progress,      
                                      cancel, questions,       
                                      result/review.           

  Phase 5 ---       3--5 weeks        Worktrees/branches,      Core product thesis
  Parallel                            multiple concurrent      demonstrably useful.
  orchestration                       runs, attention queue,   
                                      conflict/review flow.    

  Phase 6 ---       As needed         LSP lifecycle and        Navigation/diagnostics
  Language                            selected language        improve without
  intelligence                        servers based on actual  extension system.
                                      projects.                

  Phase 7 --- Linux Parallel +        Packaging,               Supported Linux release.
  hardening         focused pass      webview/PTY/filesystem   
                                      validation, distro       
                                      policy.                  

  Phase 8 ---       After validation  WebView2, ConPTY,        Windows only if Gritty
  Windows                             path/shell semantics,    has proven its workflow
  evaluation/port                     installer/signing, QA.   value.
  -------------------------------------------------------------------------------------

# 16. Phase 0 Spike --- Exact Checklist

-   Create minimal Tauri 2 + React + TypeScript application.

-   Embed Monaco; open TypeScript, Rust, Markdown and JSON fixtures.

-   Verify keyboard shortcuts, multi-cursor, selection, find, clipboard,
    drag/drop and font rendering.

-   Open a repository with at least 10k files without eagerly loading
    file contents.

-   Implement Rust commands for directory listing, safe read and atomic
    write.

-   Add filesystem watching and verify rapid changes do not lock UI.

-   Render Markdown preview with sanitized HTML and fenced-code
    highlighting.

-   Spawn a shell through PTY, resize it, stream output and terminate it
    cleanly.

-   Persist and restore workspace + tabs.

-   Record cold/warm startup, idle RAM/CPU, editor RAM growth and basic
    interaction latency.

-   Build on Linux CI or a Linux machine; log all platform-specific
    defects.

-   Write ADR-001 with final Tauri decision after measurements, not
    before.

Kill criteria for Tauri: recurring Monaco/input defects on the primary
platform, unstable PTY/process behavior that requires invasive
workarounds, or unacceptable maintenance caused by webview divergence.
If triggered, preserve the core protocol and evaluate Electron as the
shell.

# 17. Agent-Ready Work Breakdown

Each coding-agent task should be small enough to review independently,
define allowed files, specify acceptance criteria, and avoid silently
expanding scope.

  -------------------------------------------------------------------------------------
  ID                Task              Deliverable                 Acceptance
  ----------------- ----------------- --------------------------- ---------------------
  T000              Bootstrap         Create repo structure,      App launches; one
                    workspace         Tauri desktop app, React/TS command runs checks.
                                      frontend, Rust workspace,   
                                      formatting/lint/test        
                                      commands.                   

  T010              Core protocol     Define typed command/event  Frontend invokes a
                    skeleton          conventions and error       health command;
                                      envelope.                   errors serialize
                                                                  consistently.

  T020              Workspace         Native directory picker +   Large fixture opens
                    open/list         validated root + lazy       without recursive
                                      directory listing.          eager scan.

  T030              File read/write   Safe UTF-8/binary           Edit/save/reload
                                      detection, dirty buffers,   works; external
                                      atomic save.                modification is
                                                                  detected.

  T040              Monaco shell      Tabs, editor models,        Common source
                                      language detection, basic   fixtures edit
                                      preferences.                correctly.

  T050              Markdown          Source/preview/split view,  Markdown fixture
                                      sanitized GFM rendering,    renders safely and
                                      clickable local references. navigation works.

  T060              Session           SQLite schema/migrations +  Force-kill/relaunch
                    persistence       workspace/tab/cursor        restores useful
                                      restore.                    state.

  T070              File watcher      Incremental filesystem      Rapid fixture changes
                                      events with                 do not freeze UI.
                                      batching/debouncing.        

  T080              Search            Filename/text search with   Large search can be
                                      cancellation and streamed   cancelled
                                      results.                    immediately.

  T090              Terminal          PTY                         Interactive shell
                                      spawn/write/resize/kill +   survives normal usage
                                      xterm frontend.             and closes cleanly.

  T100              Git baseline      Repo detection, status and  Modified files and
                                      diff viewer.                diffs are reviewable.

  T110              Diagnostics       Structured logs + developer Core/process errors
                                      diagnostics surface.        are inspectable
                                                                  without console
                                                                  spelunking.

  T200              Workstream model  SQLite/domain state machine Workstreams survive
                                      and CRUD.                   restart and invalid
                                                                  transitions are
                                                                  rejected.

  T210              Workstream UI     Goal, plan, status,         A task can be managed
                                      decisions and related       manually end-to-end.
                                      changes.                    

  T300              Agent protocol    Backend-neutral adapter/run Fake agent can stream
                                      event types.                events
                                                                  deterministically.

  T310              First real agent  Integrate one selected CLI  Bounded task executes
                                      backend with cancellation.  without UI blocking.

  T320              Context manifest  Explicit                    User can inspect
                                      files/docs/instructions     exactly what context
                                      attached to runs.           was supplied.

  T330              Decision          Agent run can request       Question is visible
                    interruption      structured user choice and  outside transcript
                                      resume.                     and persisted.

  T400              Worktree          Create/manage Git worktree  Two workstreams
                    isolation         per workstream.             modify code
                                                                  independently.

  T410              Attention queue   Aggregate                   User can identify
                                      blocked/decision/review     required intervention
                                      states.                     at a glance.
  -------------------------------------------------------------------------------------

# 18. Coding Agent Contract

Recommended repository-level instructions for every coding agent:

-   Read the relevant architecture/ADR/plan files before modifying code.

-   Do not broaden scope beyond the assigned task. Record newly
    discovered work as follow-up items.

-   Prefer simple, testable abstractions over speculative frameworks.

-   Never introduce a new dependency without stating why the standard
    library/current dependencies are insufficient.

-   Keep frontend code free of direct filesystem/process access; use the
    defined core protocol.

-   Long-running core operations must support cancellation or document
    why cancellation is impossible.

-   Do not block the renderer or Rust command handler with long CPU/I/O
    operations.

-   Add or update tests for domain logic and regression-prone behavior.

-   Run formatting, linting, unit/integration tests relevant to changed
    areas before reporting completion.

-   Report: summary, files changed, architectural decisions, tests
    run/results, known limitations and follow-up tasks.

-   Never claim a test or command passed unless it was actually
    executed.

-   Preserve portability: platform-specific code belongs behind the
    platform abstraction.

# 19. Definition of Done

-   Scope and acceptance criteria are satisfied.

-   No unexplained warnings/errors are introduced.

-   Relevant automated tests pass.

-   Failure/cancellation behavior has been considered for
    asynchronous/process work.

-   No renderer-blocking filesystem/process work was added.

-   Persistence schema changes include migrations.

-   Security/path boundaries remain enforced.

-   Documentation/ADR is updated when a durable architectural decision
    changed.

-   The feature has been manually exercised in the running application
    where applicable.

-   Agent completion report identifies remaining risks rather than
    hiding them.

# 20. ADRs to Create Immediately

  -----------------------------------------------------------------------
  ADR                                 Decision
  ----------------------------------- -----------------------------------
  ADR-001 Desktop Runtime             Tauri 2, subject to Phase 0 spike
                                      validation

  ADR-002 Platform Order              macOS primary, Linux secondary,
                                      Windows deferred

  ADR-003 UI/Core Boundary            React/Monaco UI communicates only
                                      through typed core protocol

  ADR-004 Persistence                 SQLite for app metadata; repository
                                      files remain source artifacts

  ADR-005 Agent Isolation             Agents are supervised external
                                      processes/adapters

  ADR-006 Markdown                    Markdown is a first-class portable
                                      planning/instruction format

  ADR-007 Git Workstreams             Use branches/worktrees for isolated
                                      parallel implementation when
                                      introduced
  -----------------------------------------------------------------------

# 21. Open Decisions / Research Backlog

-   Exact Linux distribution support policy and minimum
    WebKitGTK/runtime requirements.

-   PTY Rust crate and cross-platform abstraction choice.

-   Git implementation: shell out to Git initially vs libgit2/gix later.

-   Search/index implementation: ripgrep process, Rust library, or
    hybrid.

-   Markdown stack and safe custom link/reference syntax.

-   SQLite access/migration crate.

-   Frontend state library; keep choice lightweight until state
    complexity is observed.

-   First agent backend to integrate for dogfooding.

-   Whether agent event protocol is JSON Lines, structured stdout
    adapter, RPC, or internal translation layer.

-   Workspace trust/permission UX.

-   Auto-update/signing/release channel strategy.

-   Telemetry policy; default should favor local diagnostics and
    privacy.

-   When LSP support becomes valuable enough to enter the critical path.

# 22. Success Criteria for the Product Thesis

Do not judge Gritty primarily by feature count. After sustained
dogfooding, the project is succeeding if:

-   Gritty is stable enough that editor failures are rare and do not
    interrupt unrelated project work.

-   Normal code/Markdown reading and editing feels sufficiently close to
    a conventional IDE that switching tools is not required for basic
    work.

-   The user can understand the state of multiple coding tasks without
    reading several long chat transcripts.

-   Agent questions, blockers and review-ready changes are surfaced
    clearly.

-   Three concurrent workstreams are materially easier to manage than
    three separate agent/editor windows.

-   Project decisions and task state survive model/provider changes and
    application restarts.

-   Resource usage remains acceptable while agents, terminals and
    language tooling are running.

-   The workflow improvement is large enough to justify a Windows port.

# Appendix A --- Suggested First Repository Docs

-   README.md --- what Gritty is, current maturity, build/run commands.

-   AGENTS.md --- coding-agent contract, repository map, commands,
    constraints.

-   docs/architecture/overview.md --- runtime boundaries and core
    protocol.

-   docs/architecture/security.md --- path/process/agent permission
    model.

-   docs/architecture/persistence.md --- SQLite entities and recovery
    semantics.

-   docs/plans/phase-0-spike.md --- executable spike checklist and
    measurements.

-   docs/plans/roadmap.md --- phases and exit criteria.

-   docs/decisions/ADR-001-desktop-runtime.md etc. --- durable
    decisions.

-   docs/testing/platform-matrix.md --- macOS/Linux behavior and
    regression checklist.

# Appendix B --- Recommended Next Action

Start with Phase 0 only. Do not ask agents to implement the final
architecture in parallel yet. Bootstrap the shell, prove Monaco and PTY
behavior, establish the typed core boundary, measure resource usage and
write the first ADRs. Once the spike passes, freeze the foundational
contracts long enough to build Phase 1 sequentially. Parallel agent work
becomes safer after repository boundaries, tests and acceptance
conventions exist.
