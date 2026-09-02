# ADR-007: Branch or worktree per workstream for parallel implementation

- **Status:** Accepted
- **Date:** 2026-09-02

## Context

Concurrent agent runs must not modify the same working tree. Git already
provides isolation primitives; Gritty must not invent a VCS.

## Decision

When parallel orchestration is introduced (T400), each workstream gets its own
Git branch and, by default, its own worktree. Gritty derives ChangeSets from Git
but models them as domain objects (base revision, files, diff summary, test and
review status) rather than raw `git diff` output.

## Consequences

- Workspace and document models must support multiple roots/worktrees later.
- Git integration starts read-only (T100) with clean domain concepts.

## Revisit when

Worktrees prove impractical for a supported workflow (recorded evidence required).
