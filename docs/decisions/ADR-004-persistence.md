# ADR-004: SQLite for application metadata; repository files stay authoritative

- **Status:** Accepted
- **Date:** 2026-09-02

## Context

Workstreams, decisions, runs, sessions and dirty-buffer recovery need durable,
queryable state that survives crashes. Repository files and Markdown must remain
portable project artifacts.

## Decision

Use SQLite (introduced in T060) with explicit, forward-only schema migrations for
application metadata: workspaces, sessions, tabs, cursor/layout state,
workstreams, decisions, run metadata and recovery snapshots. Repository files
remain the source of truth for project content. Gritty is not event-sourced:
runtime events notify, SQLite/domain state is authoritative for app metadata.
File writes are atomic (temp file, fsync, rename).

## Consequences

- Every schema change ships a migration.
- Recovery snapshots never silently replace repository files.
- Startup reconciles persisted process state with reality (RUNNING → interrupted).

## Revisit when

An ADR demonstrates a concrete need for event sourcing or a different store.
