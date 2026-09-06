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
| [ADR-012](ADR-012-monaco-editor-binding.md) | Monaco editor binding: one model per document, lazy reads, line-ending policy, workers | Accepted |
| [ADR-013](ADR-013-markdown-preview.md) | Markdown preview: markdown-it + DOMPurify allowlist, link resolution, `shell.openExternal` | Accepted |
| [ADR-014](ADR-014-session-persistence.md) | Session persistence and crash recovery: SQLite schema, snapshot writer, restore protocol, failure policy | Accepted |
| [ADR-015](ADR-015-terminal-pty.md) | Terminal sessions over a PTY: portable-pty, output batching, process-tree termination, shell and environment policy | Accepted |
| [ADR-016](ADR-016-design-tokens.md) | Design tokens: CSS custom properties are the single source of truth; Monaco and xterm read them back | Accepted |
| [ADR-017](ADR-017-terminal-command-blocks.md) | Terminal command blocks are a renderer-side view of xterm's scrollback | Proposed |
| [ADR-018](ADR-018-agent-integration-bridge.md) | Gritty may install a hook into another tool's configuration, additively and with consent | Accepted |
| [ADR-019](ADR-019-workspace-ownership.md) | Write ownership is a logical lease held by a workstream, and it is observed rather than enforced | Accepted |
| [ADR-020](ADR-020-agent-permission-verdicts.md) | A hook may wait for Gritty's verdict on a permission prompt, bounded and fail-open | Accepted |
| [ADR-022](ADR-022-search-engine.md) | Search runs in-process on ripgrep's own crates, and "ignored" keeps its current, narrower meaning | Proposed |
| [ADR-023](ADR-023-git-library.md) | Git is read in-process through `gix`, with no `git` process and no configuration that can run one | Proposed |
| [ADR-024](ADR-024-target-dir-retention.md) | One `target/` per checkout, swept by budget and age — not a shared `CARGO_TARGET_DIR` | Proposed |
| [ADR-025](ADR-025-filesystem-watching.md) | Filesystem watching is one coalesced, bounded, directory-grained signal | Proposed |

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
