# Gritty Agent Runtime & Workspace Coordination

**Architecture & Implementation Specification**

Agent observation • Workspace ownership • File conflict prevention • Markdown context • Multi-agent orchestration

---

# 1. Purpose

Gritty is designed to supervise multiple coding agents working on a software project simultaneously.

The IDE must therefore understand more than whether an agent process is running. It should know, whenever the underlying agent provides sufficient information:

- what agent is running;
- which workstream it belongs to;
- what it is currently doing;
- which files it is reading;
- which files or repository areas it intends to modify;
- which commands it is executing;
- whether it is waiting, blocked, or requesting a decision;
- what project documentation and instructions it received;
- which files or paths it currently owns for modification.

The goal is to transform multiple independent coding-agent processes into **coordinated workers supervised by Gritty**.

Gritty owns orchestration state.

Agents remain replaceable execution backends.

---

# 2. Design Inspiration

Two existing projects provide useful architectural patterns.

## Herdr

Herdr demonstrates a persistent runtime model around coding-agent terminals.

Relevant concepts:

- persistent agent/session processes;
- semantic agent states such as working, blocked, and idle;
- structured state reporting;
- CLI/socket APIs;
- process/session recovery;
- multiple agents represented through one runtime;
- agent state separated from the UI observing it.

Gritty should adopt the principle that agent execution belongs to a durable runtime rather than to individual UI components.

Gritty should **not** become terminal-centric as a consequence.

---

## Open Island

Open Island demonstrates how heterogeneous coding agents can be observed through provider-specific integrations.

Different agents expose different mechanisms:

- hooks;
- plugins;
- JSONL transcripts;
- JSON-RPC;
- process discovery;
- permission callbacks;
- tool lifecycle events.

The important architectural lesson is:

> Do not attempt to understand every coding agent through one generic stdout parser.

Gritty should implement provider-specific adapters that normalize their capabilities and events into one internal protocol.

---

# 3. Core Architecture

The proposed architecture is:

```text
┌─────────────────────────────────────────────┐
│                  Gritty UI                  │
│                                             │
│ Editor • Workstreams • Agents • Attention  │
└─────────────────────┬───────────────────────┘
                      │
                      │ typed commands/events
                      │
┌─────────────────────▼───────────────────────┐
│                 Gritty Core                 │
│                                             │
│ workspace • persistence • orchestration    │
└──────────────┬─────────────────┬────────────┘
               │                 │
               │                 │
     ┌─────────▼────────┐ ┌──────▼──────────┐
     │   Agent Runtime │ │ Ownership Engine│
     │                  │◄►│                │
     │ sessions         │ │ leases          │
     │ adapters         │ │ conflicts       │
     │ processes        │ │ reservations    │
     │ event normalize  │ │ permissions     │
     └─────────┬────────┘ └─────────────────┘
               │
       ┌───────┼────────┬───────────┐
       │       │        │           │
    Claude   Codex   OpenCode   Generic CLI
```

The Agent Runtime and Ownership Engine are backend services owned by Gritty Core.

Neither belongs inside React/UI state.

Their durable state should be persisted where appropriate.

---

# 4. Architecture Principles

## 4.1 Gritty owns coordination

Individual agents do not decide global workspace coordination.

Gritty owns:

- workstreams;
- agent session identity;
- agent state;
- context manifests;
- workspace ownership;
- conflicts;
- permissions;
- decisions;
- review state.

Agents execute bounded work inside those constraints.

---

## 4.2 Agents are replaceable

No core workstream or ownership concept may depend directly on:

- Claude Code;
- Codex;
- OpenCode;
- Cursor;
- Gemini;
- or any other specific provider.

Provider-specific behavior belongs behind adapters.

---

## 4.3 Observation is capability-dependent

Gritty must never pretend to know more than an agent integration exposes.

An agent may provide:

```text
process only
```

while another provides:

```text
process
session
tool calls
file reads
file writes
commands
permissions
questions
```

The UI and orchestration engine must respect this distinction.

---

## 4.4 Coordination should fail safely

Failure of Gritty's observation layer must not unexpectedly terminate an agent unless execution would otherwise violate an explicitly enforced safety or ownership rule.

Observation hooks should generally fail open.

Ownership enforcement is different and must have explicit behavior.

---

## 4.5 No renderer-owned orchestration

React must never become the source of truth for:

- active agent processes;
- leases;
- ownership;
- conflicts;
- permissions;
- agent lifecycle.

The UI renders state supplied by Gritty Core.

---

# 5. Agent Runtime

The Agent Runtime supervises agent processes and translates provider-specific behavior into Gritty's internal model.

Conceptual entity:

```ts
interface AgentSession {
  id: string;

  workstreamId: string;
  workspaceId: string;

  provider: string;
  backend: string;

  processId?: number;
  nativeSessionId?: string;

  state: AgentState;

  currentOperation?: AgentOperation;

  startedAt: number;
  lastActivityAt: number;

  capabilities: AgentCapabilities;
}
```

---

# 6. Agent State Model

Initial normalized states:

```ts
type AgentState =
  | "starting"
  | "running"
  | "waiting"
  | "needs_decision"
  | "blocked"
  | "review"
  | "finished"
  | "failed"
  | "cancelled"
  | "interrupted";
```

`running` describes the overall execution state.

Current activity provides more detail.

For example:

```text
RUNNING
Editing src/auth/session.ts
```

or:

```text
RUNNING
Running cargo test
```

or:

```text
BLOCKED
Waiting for ownership of src/api/router.ts
```

Agent state and agent activity must remain separate concepts.

---

# 7. Agent Activity

Normalized activity types should initially include:

```ts
type AgentActivity =
  | "thinking"
  | "reading"
  | "editing"
  | "searching"
  | "executing"
  | "testing"
  | "waiting"
  | "requesting_permission"
  | "requesting_decision";
```

An activity record may contain:

```ts
interface AgentOperation {
  type: AgentActivity;

  path?: string;
  command?: string;

  startedAt: number;

  metadata?: Record<string, unknown>;
}
```

Examples:

```text
reading
docs/architecture/security.md
```

```text
editing
crates/workspace/src/watcher.rs
```

```text
executing
cargo test -p workspace
```

---

# 8. Normalized Agent Event Protocol

Provider adapters emit normalized events.

Initial conceptual protocol:

```ts
type AgentEvent =
  | SessionStarted
  | SessionStopped
  | PromptStarted
  | PromptFinished
  | FileRead
  | FileWriteRequested
  | FileWritten
  | CommandStarted
  | CommandFinished
  | PermissionRequested
  | DecisionRequested
  | AgentBlocked
  | AgentResumed
  | AgentFailed;
```

Example:

```ts
interface FileRead {
  type: "file.read";

  sessionId: string;
  path: string;
  timestamp: number;
}
```

Example:

```ts
interface FileWriteRequested {
  type: "file.write.requested";

  sessionId: string;
  path: string;
  timestamp: number;
}
```

Example:

```ts
interface CommandStarted {
  type: "command.started";

  sessionId: string;
  command: string;
  cwd: string;
  timestamp: number;
}
```

Events must contain enough information to correlate them with:

```text
Workspace
→ Workstream
→ Agent Run
→ Agent Session
```

---

# 9. Agent Capability Model

Not every adapter can emit every event.

Each adapter therefore declares capabilities.

```ts
interface AgentCapabilities {
  lifecycleEvents: boolean;

  toolEvents: boolean;

  fileReadEvents: boolean;
  fileWriteEvents: boolean;

  commandEvents: boolean;

  permissionInterception: boolean;

  structuredQuestions: boolean;

  sessionRecovery: boolean;
}
```

Example:

```text
ClaudeAdapter

lifecycleEvents        ✓
toolEvents             ✓
fileReadEvents         ✓
fileWriteEvents        ✓
commandEvents          ✓
permissionInterception ✓
structuredQuestions    ✓
```

A generic CLI adapter may expose:

```text
lifecycleEvents        partial
toolEvents             ✗
fileReadEvents         ✗
fileWriteEvents        ✗
commandEvents          partial
permissionInterception ✗
structuredQuestions    ✗
```

The UI must not fabricate unavailable activity.

---

# 10. Agent Adapter Interface

Provider integrations should implement a shared conceptual interface.

```ts
interface AgentAdapter {
  id: string;

  detect(): Promise<DetectionResult>;

  capabilities(): AgentCapabilities;

  start(config: AgentStartConfig): Promise<AgentSession>;

  attach?(session: NativeSessionReference): Promise<AgentSession>;

  cancel(sessionId: string): Promise<void>;

  events(sessionId: string): AsyncIterable<AgentEvent>;
}
```

Provider-specific implementation details remain inside the adapter.

Examples:

```text
ClaudeAdapter
CodexAdapter
OpenCodeAdapter
GenericCliAdapter
```

Future integrations should not require modifications to the core workstream model.

---

# 11. Observation Strategies

Adapters may obtain information through several mechanisms.

Preferred order:

```text
structured native API
        ↓
agent hooks/plugins
        ↓
structured transcript/events
        ↓
PTY/process observation
        ↓
filesystem inference
```

Structured events should always be preferred over heuristic inference.

The source of an event should optionally be recorded:

```ts
type ObservationSource =
  | "native"
  | "hook"
  | "plugin"
  | "transcript"
  | "process"
  | "filesystem"
  | "inferred";
```

This will later allow diagnostics and confidence-aware UI.

---

# 12. Workspace Ownership Engine

Parallel agents introduce a fundamental coordination problem:

> Two agents must not unknowingly modify the same logical area simultaneously.

Gritty therefore needs a first-class Workspace Ownership Engine.

This is separate from Git.

This is separate from OS-level file locking.

This is an orchestration primitive.

---

# 13. Logical Ownership, Not OS Locks

Do not use operating-system file locks as the primary coordination mechanism.

Instead, Gritty maintains logical ownership leases.

Reasons:

- agents may not respect OS locks;
- lock behavior varies across platforms;
- workstream ownership is semantic, not merely filesystem-level;
- directories and globs may need ownership;
- ownership must be visible in the UI;
- ownership needs workstream metadata;
- conflicts need explicit resolution;
- worktrees may later supersede or alter ownership requirements.

---

# 14. Ownership Lease

Conceptual model:

```ts
interface OwnershipLease {
  id: string;

  workspaceId: string;
  workstreamId: string;
  agentRunId: string;

  target: OwnershipTarget;

  mode: "write";

  acquiredAt: number;
  expiresAt?: number;

  state: "active" | "released" | "expired";
}
```

Ownership targets:

```ts
type OwnershipTarget =
  | FileTarget
  | DirectoryTarget
  | GlobTarget;
```

Initial implementation should prioritize:

```text
file
directory
glob
```

Symbol-level ownership may be investigated later but is not required initially.

---

# 15. Ownership Examples

File:

```text
src/auth/session.ts
```

Directory:

```text
src/auth/**
```

Glob:

```text
tests/auth/**/*.ts
```

Example:

```yaml
workstream: authentication-refactor

ownership:
  - src/auth/**
  - tests/auth/**
```

Another workstream requesting:

```text
src/payments/**
```

does not conflict.

Another requesting:

```text
src/auth/session.ts
```

does conflict.

---

# 16. Ownership Lifecycle

Typical lifecycle:

```text
Workstream planned
       ↓
required paths identified
       ↓
ownership requested
       ↓
conflict check
       ↓
ownership granted
       ↓
agent execution
       ↓
work completed/reviewed
       ↓
ownership released
```

Ownership should normally be acquired before modification begins.

---

# 17. Dynamic Ownership

Agents will not always know every file before execution.

Therefore ownership must also support dynamic acquisition.

Example:

```text
Agent wants to edit:

src/api/router.ts
        ↓
file.write.requested
        ↓
Ownership Engine
        ↓
available?
```

If yes:

```text
lease acquired
↓
write allowed
```

If no:

```text
write blocked
↓
agent enters BLOCKED
↓
conflict surfaced
```

---

# 18. Ownership Conflict

Example:

```text
Agent A

Workstream:
Authentication

owns:
src/api/router.ts
```

Agent B requests the same file.

Gritty creates:

```ts
interface OwnershipConflict {
  id: string;

  requestedBy: string;
  ownedBy: string;

  target: OwnershipTarget;

  createdAt: number;

  state:
    | "waiting"
    | "resolved"
    | "overridden"
    | "cancelled";
}
```

The UI should show:

```text
Ownership conflict

Payment API
wants to modify:

src/api/router.ts

Currently owned by:

Authentication
Agent A

Actions:

Wait
Request release
Override
Move workstream to isolated worktree
Cancel task
```

Not every resolution action must exist in the first implementation.

---

# 19. Write Interception

For adapters supporting pre-write or pre-tool hooks:

```text
Agent
  ↓
write requested
  ↓
adapter hook
  ↓
Gritty Ownership Engine
  ↓
allow / deny / wait
```

This is the preferred enforcement mechanism.

---

# 20. Agents Without Write Interception

Some agents may not expose a mechanism allowing Gritty to stop writes before they occur.

Gritty must clearly distinguish:

```text
ENFORCED OWNERSHIP
```

from:

```text
OBSERVED OWNERSHIP
```

For an agent without interception support:

```text
filesystem watcher
      ↓
unauthorized modification detected
      ↓
ownership violation
      ↓
pause agent when possible
      ↓
surface conflict
      ↓
reconcile changes
```

This is weaker than pre-write interception and must not be presented as equivalent.

---

# 21. Lease Recovery

Ownership must not become permanently stuck after crashes.

Each lease should be associated with:

```text
Agent Run
Workstream
Process/session
```

On startup Gritty reconciles leases.

For example:

```text
lease active
+
agent process gone
+
run interrupted
```

becomes:

```text
lease stale
```

Stale leases should be recoverable/releasable.

Never silently retain ownership forever because an agent crashed.

---

# 22. Ownership and Git Worktrees

Ownership and worktrees solve related but different problems.

Ownership:

```text
same workspace
+
multiple agents
+
prevent overlapping modifications
```

Worktrees:

```text
separate filesystem trees
+
parallel Git changes
+
independent branches
```

Gritty should support both.

Example:

```text
Workstream A
main workspace
owns src/auth/**
```

```text
Workstream B
isolated worktree
owns its own copy of src/auth/**
```

There is no filesystem-level ownership collision because the workspaces are isolated.

Merge conflicts may still occur later and should be handled as Git/review conflicts rather than live ownership conflicts.

---

# 23. Ownership-Aware Scheduling

Eventually Gritty should inspect workstream scope before starting an agent.

Example:

```text
Workstream A

expected modifications:
src/auth/**
```

and:

```text
Workstream B

expected modifications:
src/auth/session.ts
```

Gritty detects overlap before execution.

Possible response:

```text
Workstream B cannot safely start in the current workspace.

Conflict:
src/auth/session.ts

Owned by:
Workstream A

Options:

Wait
Use worktree
Change scope
Override
```

This should eventually become part of the transition:

```text
READY → RUNNING
```

---

# 24. Markdown as Operational Context

Markdown is a first-class project artifact in Gritty.

Agents should be able to receive repository Markdown as explicit context.

Examples:

```text
AGENTS.md
README.md

docs/architecture/*.md
docs/plans/*.md
docs/decisions/*.md
```

Markdown remains stored as normal repository files.

Gritty does not replace these documents with proprietary conversation state.

---

# 25. Context Manifest

Every Agent Run should have an explicit Context Manifest.

Example:

```ts
interface ContextManifest {
  workspaceInstructions: ContextEntry[];

  taskDocuments: ContextEntry[];

  architectureDocuments: ContextEntry[];

  sourceFiles: ContextEntry[];
}
```

Example representation:

```json
{
  "workspaceInstructions": [
    "AGENTS.md"
  ],

  "taskDocuments": [
    "docs/plans/auth-refactor.md"
  ],

  "architectureDocuments": [
    "docs/architecture/overview.md",
    "docs/decisions/ADR-004.md"
  ],

  "sourceFiles": [
    "src/auth/session.ts",
    "src/auth/types.ts"
  ]
}
```

---

# 26. Context Visibility

The user must be able to inspect exactly what Gritty supplied to an agent.

Example UI:

```text
Agent Context

Workspace Instructions
✓ AGENTS.md

Task
✓ docs/plans/auth-refactor.md

Architecture
✓ docs/architecture/overview.md
✓ ADR-004-auth-boundaries.md

Source
✓ src/auth/session.ts
✓ src/auth/types.ts
```

This is important for debugging agent behavior.

Questions such as:

> Did the agent read our architecture rules?

should have deterministic answers.

---

# 27. Required Markdown Reading

Workspaces may define required agent documentation.

Initial convention:

```text
AGENTS.md
```

may define repository-wide instructions.

Gritty should also allow explicit required documents through workspace configuration.

Example:

```yaml
agentContext:
  required:
    - AGENTS.md
    - docs/architecture/overview.md
```

Before starting execution, Gritty can ensure these documents appear in the Context Manifest.

Do not hard-code assumptions that every repository uses identical documentation conventions.

---

# 28. Agent Activity UI

Gritty should expose live agent activity without requiring users to read terminal output.

Example:

```text
┌───────────────────────────────────────┐
│ Authentication                       │
│ Claude Code               ● Working │
│                                       │
│ Editing src/auth/session.ts          │
│ Owns 4 files                         │
└───────────────────────────────────────┘
```

```text
┌───────────────────────────────────────┐
│ Payment API                          │
│ Codex                     ⚠ Blocked │
│                                       │
│ Waiting for src/api/router.ts        │
│ Owned by Authentication              │
└───────────────────────────────────────┘
```

```text
┌───────────────────────────────────────┐
│ Documentation                        │
│ OpenCode                  ● Working │
│                                       │
│ Reading docs/architecture.md         │
│ Writing docs/api.md                  │
└───────────────────────────────────────┘
```

---

# 29. Agent Detail View

Selecting an agent should expose structured state.

Example:

```text
Authentication Refactor
Claude Code

RUNNING

Current activity

Editing
src/auth/session.ts

Recent activity

✓ Read AGENTS.md
✓ Read ADR-004.md
✓ Read src/auth/types.ts
✓ Modified src/auth/session.ts
● Running cargo test

Context

AGENTS.md
auth-refactor.md
ADR-004.md

Owned paths

src/auth/**
tests/auth/**

Process

PID 38241
Running 12m 43s
```

Raw terminal/session output may remain available but should not be the primary status representation.

---

# 30. Attention Queue Integration

Agent state should feed the Gritty Attention Queue.

Important states:

```text
NEEDS_DECISION
BLOCKED
OWNERSHIP_CONFLICT
PERMISSION_REQUIRED
FAILED
REVIEW_READY
```

The user should not need to inspect every running agent.

The product should surface only work requiring human attention.

---

# 31. Persistence

Persist:

- Agent Run metadata;
- native session references where useful;
- last known state;
- context manifests;
- ownership leases;
- ownership conflicts;
- decisions;
- timestamps;
- adapter identity;
- adapter capabilities.

Do not rely on persistence of:

- provider process internals;
- renderer state;
- transient hook connections.

Startup reconciliation must compare persisted state against actual runtime state.

---

# 32. Core API

Initial conceptual core API:

```text
agents.list()
agents.get(sessionId)

agents.start(workstreamId, config)
agents.cancel(sessionId)

agents.capabilities(adapterId)

agents.subscribe(sessionId)
agents.subscribeAll()
```

Ownership:

```text
ownership.acquire(request)
ownership.release(leaseId)

ownership.list(workspaceId)
ownership.forWorkstream(workstreamId)

ownership.check(target)

ownership.conflicts()
ownership.resolve(conflictId, action)
```

Context:

```text
context.build(workstreamId)
context.get(runId)
context.validate(runId)
```

All long-running operations must remain asynchronous and cancellable where applicable.

---

# 33. Security Requirements

Agent events and agent-provided paths are untrusted input.

Always:

- canonicalize paths;
- validate paths against workspace roots;
- reject traversal outside allowed roots;
- validate hook payloads;
- bound event/message sizes;
- avoid rendering raw agent HTML;
- sanitize Markdown preview;
- avoid logging secrets;
- validate process identity where practical.

An agent must never gain filesystem access merely because it reports a path in an event.

---

# 34. Failure Behavior

The following failures must not crash or freeze the IDE:

```text
agent crash
adapter crash
hook disconnect
malformed event
ownership service error
filesystem watcher overflow
provider protocol change
agent producing unexpected output
```

Failures should become inspectable diagnostics.

Example:

```text
Codex adapter

DEGRADED

Reason:
Unexpected tool event schema

Agent process:
Still running

File observation:
Unavailable
```

---

# 35. Diagnostics

Developer diagnostics should expose:

```text
Agent Runtime

sessions: 3
adapters: 3

Claude
connected

Codex
connected

OpenCode
degraded
```

Ownership diagnostics:

```text
Ownership

active leases: 7
waiting requests: 1
conflicts: 1
stale leases: 0
```

Event diagnostics:

```text
Events

received: 1,482
normalized: 1,476
rejected: 6
```

Provider integration failures should be debuggable without inspecting arbitrary frontend console logs.

---

# 36. Implementation Order

Do not implement every integration simultaneously.

Recommended order:

## T300 — Agent Event Protocol

Implement:

- normalized session model;
- normalized events;
- state transitions;
- event correlation;
- tests.

Acceptance:

A fake adapter can deterministically simulate an agent lifecycle.

---

## T305 — Agent Capability Model

Implement:

- adapter capability declaration;
- capability persistence;
- capability-aware core behavior.

Acceptance:

Gritty can distinguish a fully observable agent from a generic CLI agent.

---

## T310 — First Real Adapter

Choose one agent used regularly during Gritty development.

Implement:

- detection;
- lifecycle;
- event ingestion;
- cancellation;
- recovery where supported.

Acceptance:

One real coding-agent session appears in Gritty and updates state without blocking the UI.

---

## T315 — Live Agent Activity

Implement:

- current activity;
- recent events;
- file activity;
- command activity;
- basic agent status UI.

Acceptance:

The user can understand what the agent is doing without reading its terminal transcript.

---

## T320 — Context Manifest

Implement:

- Markdown instruction discovery;
- explicit context entries;
- manifest persistence;
- manifest inspection UI.

Acceptance:

The user can determine exactly which project documents/files were supplied to an Agent Run.

---

## T325 — Workspace Ownership Registry

Implement:

- file leases;
- directory leases;
- glob leases;
- conflict detection;
- release;
- crash/stale recovery.

Acceptance:

Two simulated Agent Runs cannot obtain conflicting write ownership without an explicit conflict.

---

## T330 — Write Interception

Implement ownership enforcement for the first adapter supporting pre-write/tool interception.

Acceptance:

An agent attempting to modify a path owned by another workstream is blocked before modification.

---

## T335 — Decision Interruption

Implement structured:

```text
NEEDS_DECISION
BLOCKED
PERMISSION_REQUIRED
```

events.

Acceptance:

Questions and blockers appear outside raw agent transcripts and survive restart.

---

## T400 — Worktree Isolation

Implement branch/worktree-per-workstream isolation.

Acceptance:

Two workstreams can modify otherwise conflicting repository paths independently.

---

## T405 — Ownership-Aware Scheduler

Before execution:

- inspect requested scope;
- compare ownership;
- identify conflicts;
- recommend waiting or worktree isolation.

Acceptance:

Known ownership conflicts are detected before an Agent Run starts.

---

## T410 — Attention Queue

Aggregate:

- decisions;
- blockers;
- ownership conflicts;
- permission requests;
- failures;
- review-ready work.

Acceptance:

The user can determine where intervention is required without opening each workstream.

---

# 37. Testing Requirements

Agent Runtime tests must cover:

- valid event normalization;
- malformed events;
- duplicate events;
- out-of-order events;
- agent termination;
- adapter disconnect;
- cancellation;
- restart/reconciliation.

Ownership tests must cover:

- independent files;
- identical files;
- directory/file overlap;
- glob/file overlap;
- multiple leases;
- release;
- stale leases;
- crashed agents;
- canonical path equivalence;
- path traversal attempts.

Concurrency tests are mandatory.

The ownership registry must not allow two concurrent acquisition requests to both receive incompatible ownership.

---

# 38. Initial Non-Goals

Do not initially build:

- distributed ownership across multiple computers;
- multi-user collaborative editing;
- symbol-level semantic locking;
- autonomous agent-to-agent negotiation;
- custom version control;
- universal provider behavior;
- perfect inference from arbitrary terminal output.

Prefer deterministic capabilities over fragile magic.

---

# 39. Definition of Done for Agent Integration

An adapter is not considered complete merely because an agent process can launch.

It should document:

- supported lifecycle events;
- supported activity events;
- file read visibility;
- file write visibility;
- write interception capability;
- permission capability;
- structured decision capability;
- cancellation behavior;
- crash behavior;
- session recovery;
- known limitations.

Capabilities must match actual behavior.

---

# 40. Coding Agent Rules

Agents implementing this subsystem must follow these rules.

1. Do not introduce provider-specific concepts into Workstream or Ownership domain models.

2. Do not place orchestration state inside React components.

3. Do not treat filesystem observation as equivalent to pre-write interception.

4. Do not use OS-level file locks as the primary ownership mechanism.

5. Canonicalize every filesystem path before ownership comparison.

6. Ownership acquisition must be atomic.

7. Never silently override another workstream's ownership.

8. Agent events are untrusted input.

9. Unknown provider events must degrade gracefully rather than crash the runtime.

10. Do not claim an adapter supports a capability unless it is tested against the actual provider behavior.

11. Keep raw provider payloads behind adapter boundaries.

12. Add tests for every new normalized event/state transition.

13. Persist durable orchestration state, not arbitrary provider internals.

14. Keep long-running work outside the renderer.

15. Record architectural discoveries as follow-up tasks rather than silently expanding implementation scope.

---

# 41. Product Success Criterion

This subsystem succeeds when a developer can run several coding agents and understand the system at a glance:

```text
Authentication
Claude
WORKING
Editing src/auth/session.ts

Payments
Codex
BLOCKED
Waiting for src/api/router.ts

Documentation
OpenCode
WORKING
Reading architecture.md
```

The developer should be able to answer:

- What is every agent doing?
- What is blocked?
- Which files are being modified?
- Who owns this file?
- Why can't another agent modify it?
- What documentation did this agent receive?
- Which agent needs my attention?
- Which changes are ready for review?

without reading multiple terminal transcripts or switching between several agent applications.

---

# 42. Architectural North Star

Gritty should not attempt to become the coding agent.

It should become the **coordination layer around coding agents**.

The relationship is:

```text
Agents execute.

Gritty observes.
Gritty coordinates.
Gritty protects shared workspace state.
Gritty persists intent.
Gritty surfaces decisions.
The developer remains in control.
```

This distinction should guide implementation decisions throughout the Agent Runtime and multi-agent orchestration system.