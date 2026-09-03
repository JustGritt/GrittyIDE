# Architecture decision records

One file per durable decision, numbered in order. Statuses: Proposed, Accepted,
Superseded by ADR-XXX. Changing a frozen contract (see `AGENTS.md`) normally
requires updating the relevant ADR.

| ADR | Title | Status |
|---|---|---|
| [ADR-001](ADR-001-desktop-runtime.md) | Desktop runtime: Tauri 2 | Proposed (decided by Phase 0) |
| [ADR-002](ADR-002-platform-order.md) | Platform order: macOS, Linux, Windows deferred | Accepted |
| [ADR-003](ADR-003-ui-core-boundary.md) | UI/core boundary through a typed protocol | Accepted |
| [ADR-004](ADR-004-persistence.md) | SQLite for app metadata; repository files stay authoritative | Accepted |
| [ADR-005](ADR-005-agent-isolation.md) | Agents are supervised external processes behind adapters | Accepted |
| [ADR-006](ADR-006-markdown.md) | Markdown is a first-class portable planning format | Accepted |
| [ADR-007](ADR-007-git-workstreams.md) | Branch/worktree per workstream for parallel work | Accepted |
| [ADR-008](ADR-008-repository-tooling.md) | Repository tooling and type sharing | Accepted |
| [ADR-009](ADR-009-core-protocol-conventions.md) | Core protocol conventions: ids, envelopes, events, versioning | Accepted |
| [ADR-010](ADR-010-workspace-paths.md) | Workspace root, path representation and lazy listing | Accepted |
| [ADR-011](ADR-011-document-buffer-contract.md) | Document/buffer contract: identity, versions, dirty state, atomic saves, external changes | Accepted |

Template:

```markdown
# ADR-NNN: Title

- **Status:** Proposed | Accepted | Superseded by ADR-XXX
- **Date:** YYYY-MM-DD

## Context
## Decision
## Consequences
## Revisit when
```
