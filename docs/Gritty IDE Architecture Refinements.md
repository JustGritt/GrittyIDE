# Gritty IDE --- Architecture Refinements & Agent Execution Addendum

> **Superseded on 2026-09-15 by `docs/plans/project-plan.md`** (PLAN-OF-RECORD,
> one project plan replaces the founding plan, its addendum and the roadmap).
> Kept unchanged below because code, ADRs and task files cite its section
> numbers. It is history, not instructions: the "agents should follow this
> document" note below no longer applies. Do not edit.

**Status:** Pre-implementation architecture addendum\
**Date:** 2 September 2026\
**Applies to:** `Gritty IDE Project Plan.md`\
**Purpose:** Refine the existing project plan before Phase 0 and provide
coding agents with explicit architectural contracts, task dependencies,
concurrency semantics, failure expectations, and implementation
constraints.

> This document supplements the main Gritty IDE project plan. It does
> not replace it. Where this addendum is more specific than the original
> plan, agents should follow this document unless an ADR explicitly
> supersedes it.

------------------------------------------------------------------------

## 1. Why This Addendum Exists

The existing project plan establishes the correct product direction:

-   Gritty is a dependable editor first and an agent orchestration
    environment second.
-   Tauri + Rust forms the desktop/core boundary, with React + Monaco in
    the UI.
-   Heavy and failure-prone work must remain outside the renderer.
-   Workstreams, decisions, context, changes and results are durable
    domain state.
-   Agents are replaceable supervised execution backends.
-   Markdown and repository files remain portable project artifacts.
-   Reliability, recoverability and inspectability take priority over
    feature count.

Before implementation begins, several cross-cutting contracts need to be
made more explicit so independent coding agents do not invent
incompatible abstractions.

The most important refinements are:

1.  Task dependencies and architectural contract gates.
2.  A standard asynchronous operation/concurrency model.
3.  A first-class document/buffer model.
4.  Stronger Git and ChangeSet semantics.
5.  Explicit capability-based permissions.
6.  Actor/ownership attribution.
7.  A stable internal event vocabulary.
8.  Dependency/version policy.
9.  Deterministic fixture repositories.
10. Failure/crash testing during Phase 0.
11. A fake-agent milestone before any real provider integration.

------------------------------------------------------------------------

# 2. Implementation Dependency Graph

Coding tasks are **not independent**. Agents must respect the following
dependency order.

``` text
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

Agents may prepare isolated research or tests for later tasks, but they
must not implement a downstream domain abstraction before its upstream
contract exists.

## 2.1 Contract gates

Some tasks establish contracts that downstream work must consume rather
than reinterpret.

### Gate A --- T010 Core Protocol

Must establish:

-   command conventions;
-   event conventions;
-   shared identifiers;
-   error envelope;
-   asynchronous operation semantics;
-   cancellation semantics;
-   serialization/versioning conventions.

Do not allow later frontend features to call raw Tauri
filesystem/process primitives directly.

### Gate B --- T030 Document/Buffer Contract

Must establish:

-   relationship between files and open documents;
-   dirty-state semantics;
-   safe-save semantics;
-   external modification detection;
-   encoding/binary behavior;
-   crash recovery inputs.

Monaco integration must consume this contract rather than becoming the
source of truth for documents.

### Gate C --- T200 Workstream Domain

Must establish:

-   workstream state machine;
-   valid transitions;
-   persistence;
-   relationships to decisions, runs and changes.

Agent adapters must not define their own task/workstream state model.

### Gate D --- T300 Agent Protocol

Must establish a provider-neutral run/event model using a deterministic
fake agent before a real backend is introduced.

Real provider behavior must be translated into the Gritty protocol, not
leak into the core domain.

------------------------------------------------------------------------

# 3. Standard Asynchronous Operation Model

Long-running work must use one common lifecycle.

This applies to:

-   repository enumeration;
-   search;
-   Git operations that may block;
-   indexing;
-   process execution;
-   agent runs;
-   selected filesystem operations;
-   future language-server operations.

Conceptual flow:

``` text
UI command
    ↓
fast validation
    ↓
operation created
    ↓
operationId returned
    ↓
background/supervised execution
    ↓
progress/events
    ↓
COMPLETED | FAILED | CANCELLED
```

## 3.1 Required operation properties

Every cancellable long-running operation should have:

``` text
Operation
- id
- kind
- actor
- workspaceId
- status
- createdAt
- startedAt
- finishedAt
- cancellationRequested
- correlationId
```

Not every operation needs durable SQLite persistence. Persist only
operations whose recovery/history matters.

## 3.2 Required lifecycle semantics

Agents must define behavior for:

-   cancellation before execution starts;
-   cancellation during execution;
-   child-process termination;
-   events arriving after cancellation;
-   duplicate completion signals;
-   UI reload/disconnection;
-   application shutdown;
-   unexpected worker/process death.

Cancellation is cooperative where possible and forceful where necessary.

A cancelled operation must never later appear as successfully completed
because of a stale event.

## 3.3 Backpressure

High-frequency sources such as filesystem watchers, terminal output,
indexing and agent logs must not flood the renderer.

Use batching, bounded queues, throttling or domain-level aggregation
where appropriate.

Do not send one UI event per low-level filesystem event during large
repository changes if a compact batch can represent the same state.

------------------------------------------------------------------------

# 4. Document and Buffer Model

A repository file and an open editable document are not the same thing.

Introduce an explicit document/buffer domain abstraction before Monaco
becomes deeply integrated.

Suggested conceptual model:

``` text
Document
- id
- workspaceId
- path / URI
- encoding
- languageId
- savedRevision
- savedHash
- currentVersion
- dirty
- readOnly
- binary
- largeFileMode
- externalModificationState
```

The exact schema may evolve, but the distinction must remain.

## 4.1 Source of truth

-   Repository files remain the durable source artifacts.
-   Gritty owns unsaved buffer state while a document is
    open/recoverable.
-   Monaco is a rendering/editing component, not the durable source of
    truth.
-   SQLite may persist recovery metadata/content as needed, but must not
    silently replace repository files as the canonical project state.

## 4.2 External modification conflict

Critical scenario:

``` text
1. User opens file A.
2. User edits A without saving.
3. External process or agent modifies A on disk.
4. File watcher detects the change.
5. Gritty must preserve both states.
```

Never silently overwrite the dirty buffer or external version.

The initial UI may be simple, but the domain must be able to represent:

``` text
UNCHANGED
MODIFIED_EXTERNALLY
CONFLICTED
DELETED_EXTERNALLY
```

A future resolution flow can offer compare, reload, keep buffer, or
merge.

## 4.3 Safe writes

Normal saves should use safe/atomic replacement where supported:

``` text
write temporary file
→ flush/fsync where appropriate
→ replace destination
→ update saved revision/hash
```

Platform-specific behavior belongs behind the platform/filesystem
abstraction.

------------------------------------------------------------------------

# 5. Actor and Ownership Model

Concurrent human, agent and external activity needs attribution.

Introduce a lightweight actor model:

``` text
Actor
- USER
- SYSTEM
- AGENT(runId)
- EXTERNAL
```

Actor attribution should be available where useful for:

-   operations;
-   filesystem changes;
-   command execution;
-   decisions;
-   logs;
-   ChangeSets;
-   agent-generated edits.

This does **not** require full event sourcing.

The purpose is explainability.

The UI should eventually be able to distinguish:

``` text
Modified by you
Modified by Agent Run #18
Modified by a Gritty system operation
Modified externally
```

------------------------------------------------------------------------

# 6. Internal Event Vocabulary

Define a stable domain event vocabulary early.

Suggested initial families:

``` text
WorkspaceOpened
WorkspaceClosed
WorkspaceChanged

DocumentOpened
DocumentChanged
DocumentSaved
DocumentExternalChangeDetected
DocumentClosed

FileSystemBatchChanged

OperationStarted
OperationProgress
OperationCompleted
OperationFailed
OperationCancelled

GitStatusChanged
ChangeSetUpdated

WorkstreamCreated
WorkstreamChanged

AgentRunStarted
AgentRunChanged
AgentRunFinished

DecisionRequested
DecisionResolved
```

Names are illustrative; T010 owns the final naming convention.

## 6.1 Important architectural rule

**Gritty is not event-sourced.**

Runtime events notify consumers about changes and progress.

SQLite/domain state remains authoritative for durable application
metadata. Repository files remain authoritative project artifacts.

Do not introduce an event-sourcing framework unless an ADR later
demonstrates a concrete need.

------------------------------------------------------------------------

# 7. Git and ChangeSet Semantics

Git is introduced as a user-facing feature in Phase 2, but its domain
concepts affect later orchestration and should be kept clean from the
beginning.

Core concepts:

``` text
Repository
Revision
ChangeSet
```

A `ChangeSet` must not merely mean "the current output of git diff."

It represents a coherent set of changes associated with work.

Conceptually:

``` text
ChangeSet
- id
- workspaceId
- workstreamId?
- agentRunId?
- actor
- baseRevision
- resultingRevision?
- files
- diffSummary
- testStatus
- reviewStatus
- createdAt
- updatedAt
```

Later orchestration should support:

``` text
Workstream
  → Agent Run
      → ChangeSet
          → files
          → base revision
          → tests
          → review state
```

The implementation may initially derive much of this from Git, but the
domain model should not be coupled to raw `git diff` output.

------------------------------------------------------------------------

# 8. Capability-Based Permission Model

Agent permissions must be explicit and inspectable.

Prefer capabilities over scattered provider-specific booleans.

Initial conceptual capabilities:

``` text
filesystem.read
filesystem.write
process.spawn
network.access
git.read
git.write
workspace.outsideRoot
secrets.access
```

Capabilities may be scoped.

Example:

``` yaml
filesystem.write:
  roots:
    - /workspace/src
    - /workspace/tests
```

A command permission may eventually include command/profile
restrictions.

## 8.1 Rules

-   Default permissions should be narrow.
-   Provider adapters translate Gritty capabilities into
    provider-specific mechanisms.
-   Provider permission concepts must not become Workstream domain
    fields.
-   Permission grants should be inspectable.
-   Destructive or out-of-scope actions require explicit approval unless
    an appropriate persistent permission was deliberately granted.
-   Secrets should not appear in normal logs, events or persisted run
    output.

The initial UI may expose only a small subset of this model, but the
core representation should leave room for scoped capabilities.

------------------------------------------------------------------------

# 9. Dependency and Version Policy

Reliability requires dependency discipline.

## 9.1 General policy

-   Prefer mature and actively maintained dependencies.
-   Prefer existing project dependencies before adding overlapping
    libraries.
-   Avoid speculative framework dependencies.
-   Every new dependency introduced by an agent must include a short
    justification.
-   Security-sensitive dependencies require deliberate review.
-   Major dependency upgrades should normally be explicit tasks rather
    than incidental changes.
-   Lockfiles are committed.

## 9.2 JavaScript tooling

Unless superseded by ADR:

-   use one package manager repository-wide;
-   prefer `pnpm`;
-   commit the lockfile;
-   pin/coordinate workspace tooling versions;
-   do not introduce npm/yarn/bun lockfiles alongside it.

## 9.3 Rust

-   Commit `Cargo.lock` for the application workspace.
-   Keep platform-specific crates isolated where practical.
-   Avoid unsafe Rust unless justified and reviewed.
-   Prefer small adapters around PTY/Git/platform libraries so
    replacements remain possible.

------------------------------------------------------------------------

# 10. Fixture Repository Strategy

Testing must use deterministic fixture repositories rather than
depending only on developers' real projects.

Suggested structure:

``` text
fixtures/
  small-repo/
  large-repo/
  weird-files/
  markdown/
  git-history/
  git-conflicts/
```

## 10.1 `small-repo`

Used for fast integration and E2E tests.

Include representative:

-   TypeScript;
-   Rust;
-   JSON;
-   Markdown;
-   nested directories.

## 10.2 `large-repo`

Used for performance and cancellation testing.

Should eventually exercise:

-   10k+ files;
-   deep trees;
-   large directories;
-   ignored/generated directories;
-   rapid filesystem changes.

Do not necessarily commit thousands of large files if a deterministic
generator is more practical.

## 10.3 `weird-files`

Cover filesystem edge cases where supported:

-   Unicode filenames;
-   spaces;
-   very long lines;
-   large text files;
-   binary files;
-   invalid UTF-8;
-   hidden files;
-   symlinks;
-   broken symlinks;
-   deeply nested paths;
-   permission errors.

Platform-specific expectations must be documented rather than forcing
identical behavior where OS semantics differ.

## 10.4 Git fixtures

Create deterministic history for:

-   clean repository;
-   modified tracked files;
-   untracked files;
-   staged changes;
-   branches;
-   merge conflicts;
-   worktrees when T400 begins.

------------------------------------------------------------------------

# 11. Phase 0 Failure and Recovery Tests

Phase 0 must validate failure containment, not only happy paths.

Add the following tests to the existing Phase 0 checklist.

## 11.1 Dirty-buffer crash test

``` text
1. Open workspace.
2. Edit a file without saving.
3. Force-kill Gritty.
4. Restart.
5. Verify the user can recover the unsaved content.
```

A primitive recovery implementation is acceptable during the spike.
Silent data loss is not.

## 11.2 PTY failure test

``` text
1. Spawn PTY.
2. Produce output.
3. Kill the child unexpectedly.
4. Verify renderer remains responsive.
5. Verify terminal state reports termination.
```

## 11.3 Worker/operation cancellation test

Use search or another long-running fixture operation:

``` text
start
→ receive progress
→ cancel
→ verify prompt cancellation
→ verify no stale successful completion
```

## 11.4 Renderer reload test

Reload/recreate the renderer while the core/workspace exists where
technically practical.

Verify that durable domain state can be rehydrated without corrupting
the workspace.

## 11.5 Filesystem storm test

Generate rapid file changes and verify:

-   UI remains responsive;
-   watcher events are bounded/batched;
-   final visible state converges to filesystem reality.

------------------------------------------------------------------------

# 12. Fake Agent Before Real Agent

T300 must include a deterministic fake agent backend and is a mandatory
milestone before T310.

The fake backend exists to test Gritty orchestration independently of
external model/provider behavior.

It must be able to simulate at least:

``` text
START
PROGRESS
COMMAND/TOOL ACTIVITY
FILE CHANGE
DECISION REQUEST
RESUME
BLOCKED
FAILURE
CANCELLATION
SUCCESS
```

Suggested deterministic scenarios:

``` text
fake-agent/success
fake-agent/needs-decision
fake-agent/fails
fake-agent/slow-cancellable
fake-agent/changes-files
```

Acceptance criterion:

> The workstream/run/review UI and process lifecycle can be exercised
> end-to-end without network access, API credentials or a real AI
> provider.

Only after this passes should T310 integrate the first real CLI/API
agent.

------------------------------------------------------------------------

# 13. Refined Delivery Sequence

The existing roadmap remains valid, with the following refinement.

## Phase 0 --- Architecture and Runtime Spike

Prove:

-   Tauri/WebView viability;
-   Monaco behavior;
-   filesystem boundary;
-   typed protocol;
-   PTY;
-   Markdown;
-   persistence fundamentals;
-   failure containment;
-   Linux build viability;
-   measurable performance baseline.

Exit with ADR-001 go/no-go.

## Phase 1A --- Workspace + Document Foundation

Implement:

-   workspace lifecycle;
-   file explorer;
-   document/buffer model;
-   safe read/write;
-   dirty buffers;
-   Monaco;
-   tabs;
-   external modification handling.

Exit when Gritty is safe for basic code editing.

## Phase 1B --- Markdown + Persistence + Recovery

Implement:

-   Markdown source/preview;
-   session restoration;
-   dirty-buffer recovery;
-   cursor/layout state where appropriate.

Exit when force-killing/restarting Gritty does not make routine editing
feel unsafe.

## Phase 2 --- Developer Essentials

Implement:

-   search;
-   terminal;
-   Git status/diff;
-   watcher hardening;
-   diagnostics;
-   settings.

Exit when Gritty is usable for meaningful daily repository work.

## Phase 3 --- Workstreams

Implement the persistent workstream/decision/change association model
without any AI dependency.

Exit when a workstream can be manually managed from goal to review/done.

## Phase 4A --- Agent Protocol + Fake Agent

Implement:

-   provider-neutral protocol;
-   run lifecycle;
-   normalized events;
-   capabilities;
-   context manifest;
-   cancellation;
-   deterministic fake backend.

Exit when orchestration can be tested completely offline.

## Phase 4B --- First Real Agent

Integrate one selected backend through an adapter.

Exit when one bounded real task can be delegated, interrupted, resumed,
cancelled and reviewed without blocking the UI.

## Phase 5 --- Parallel Orchestration

Introduce:

-   branches/worktrees;
-   concurrent runs;
-   attention queue;
-   conflicts;
-   review workflow.

Exit when multiple workstreams are materially easier to supervise in
Gritty than in separate editor/agent windows.

------------------------------------------------------------------------

# 14. Additional Research Decisions

Add the following items to the architecture research backlog.

## Filesystem

-   Symlink traversal policy.
-   Broken symlink behavior.
-   Case-sensitive vs case-insensitive path identity.
-   Path normalization rules.
-   File encoding policy beyond UTF-8.
-   Very large file thresholds.
-   Files outside the workspace root.

## Workspace configuration

Decide whether Gritty uses repository-local configuration such as:

``` text
.gritty/
```

Potential uses:

-   project settings;
-   agent instructions;
-   permission defaults;
-   workstream exports;
-   local metadata that belongs in the repository.

Do not introduce this directory until its ownership and Git behavior are
defined.

## Environment/processes

Define:

-   shell inheritance;
-   environment variable inheritance;
-   login vs non-login shell behavior;
-   proxy variables;
-   PATH resolution;
-   working-directory semantics;
-   process-tree termination.

## Distribution

Research:

-   signing;
-   notarization;
-   update channels;
-   rollback after a bad update;
-   migration compatibility across app versions.

## Accessibility/input

Track explicitly:

-   keyboard-only navigation;
-   remappable keybindings;
-   screen-reader implications;
-   IME;
-   dead keys;
-   international layouts;
-   zoom/scaling;
-   high contrast.

------------------------------------------------------------------------

# 15. Agent Task Template

Every implementation task sent to a coding agent should include the
following structure.

``` markdown
# Task <ID> — <Name>

## Objective
What this task must accomplish.

## Dependencies
Which previous tasks/contracts must already exist.

## Required reading
- relevant ADRs
- architecture docs
- related implementation files

## Allowed scope
Directories/files the agent may modify.

## Explicit non-goals
Things the agent must not implement.

## Required behavior
Concrete functional requirements.

## Failure behavior
Expected behavior under cancellation, invalid input, process death,
filesystem errors, etc.

## Acceptance criteria
Observable pass/fail conditions.

## Tests required
Unit/integration/E2E/manual checks.

## Commands to run
Formatting, linting, tests and build commands.

## Completion report
- summary
- files changed
- tests actually executed
- results
- architectural decisions
- known limitations
- follow-up tasks
```

Tasks should be small enough to review independently.

Agents must record newly discovered work rather than silently expanding
scope.

------------------------------------------------------------------------

# 16. Additional Coding-Agent Rules

In addition to the contract in the main project plan:

1.  Do not create a new architectural abstraction when an existing
    contract owns that responsibility.
2.  Do not bypass the typed core protocol from frontend code.
3.  Do not make Monaco the source of truth for persisted document state.
4.  Do not treat raw provider output as Gritty domain state.
5.  Do not treat raw `git diff` as the complete ChangeSet domain model.
6.  Do not introduce provider-specific fields into Workstream.
7.  Do not silently resolve dirty-buffer/external-file conflicts.
8.  Do not perform unbounded high-frequency event forwarding to the
    renderer.
9.  Do not introduce event sourcing, plugin systems, distributed
    infrastructure or speculative framework layers without an ADR and
    concrete requirement.
10. Do not claim cross-platform support based only on compilation.
11. Do not claim recovery behavior without force-kill/failure testing.
12. Do not claim a command/test passed unless it was executed.

------------------------------------------------------------------------

# 17. Architecture Freeze Points

Avoid continuous redesign while implementation is underway.

After Phase 0 passes, temporarily freeze:

-   frontend/core command conventions;
-   event envelope;
-   error envelope;
-   operation/cancellation model;
-   workspace path representation;
-   basic document identity;
-   persistence migration conventions.

After T200 passes, temporarily freeze:

-   Workstream identity;
-   state transitions;
-   Decision relationship;
-   ChangeSet relationship.

After T300 passes, temporarily freeze:

-   Agent Run lifecycle;
-   normalized agent event vocabulary;
-   context manifest structure;
-   capability representation.

Changes remain possible, but crossing these boundaries should require
deliberate review and usually an ADR update.

------------------------------------------------------------------------

# 18. Immediate Execution Order

The recommended next implementation sequence is:

``` text
1. T000 — Bootstrap workspace
2. T010 — Core protocol + operation/event/error conventions
3. T020 — Workspace open/list
4. T030 — File I/O + initial document/buffer contract
5. T040 — Monaco shell
6. T050 — Markdown
7. T060 — Persistence/recovery spike
8. T090 — PTY spike
9. Phase 0 failure/performance/platform tests
10. ADR-001 — Confirm or reject Tauri
```

T070/T080 and the rest of Phase 1/2 should proceed only after the Phase
0 runtime decision is recorded.

------------------------------------------------------------------------

# 19. Phase 0 Go/No-Go Questions

At the end of Phase 0, answer these explicitly.

### Editor

-   Does Monaco behave correctly in the target macOS WebView?
-   Are keyboard shortcuts, clipboard, multi-cursor and text input
    dependable?
-   Are there unresolved IME/input defects that threaten daily use?

### Runtime

-   Can filesystem operations remain outside the renderer cleanly?
-   Can high-frequency events be delivered without UI degradation?
-   Does the typed protocol remain understandable rather than becoming
    Tauri glue?

### Terminal

-   Is PTY behavior reliable enough for daily interactive use?
-   Can child process trees be terminated predictably?
-   Does unexpected child death remain contained?

### Recovery

-   Can workspace/session state survive force termination?
-   Can unsaved buffers be recovered?
-   Do interrupted operations reconcile correctly?

### Performance

-   Is startup within an acceptable range?
-   Is idle CPU effectively zero?
-   Is baseline memory materially better than the Electron alternative?
-   Does repository size degrade progressively rather than
    catastrophically?

### Platform

-   Does Linux remain buildable?
-   Are observed WebView differences manageable behind adapters/tests?

### Maintenance

-   Are Tauri/WebView-specific workarounds isolated?
-   Does the Rust/TypeScript split still produce a clean architecture?
-   Are we accumulating shell-specific hacks faster than product
    functionality?

If these answers are satisfactory, ADR-001 confirms Tauri.

If recurring editor/input, PTY, WebView divergence or maintenance
problems violate the original kill criteria, preserve the core protocol
and evaluate Electron before building further product layers.

------------------------------------------------------------------------

# 20. Final Implementation Principle

The main risk to Gritty is not insufficient functionality.

It is building sophisticated orchestration on top of an editor/runtime
that cannot be trusted.

Therefore the implementation priority remains:

``` text
correctness
→ recoverability
→ inspectability
→ responsiveness
→ developer ergonomics
→ orchestration power
```

Agent features should increase leverage without increasing uncertainty.

At every stage, Gritty should make it possible to answer:

``` text
What is happening?
Who initiated it?
What can it modify?
Can I cancel it?
What changed?
What failed?
Can I recover?
What requires my attention?
```

If the architecture makes those questions easy to answer, it is aligned
with the product thesis.
