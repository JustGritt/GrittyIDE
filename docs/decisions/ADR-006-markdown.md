# ADR-006: Markdown is a first-class, portable planning and instruction format

- **Status:** Accepted
- **Date:** 2026-09-02

## Context

Plans, specifications, decisions and agent instructions naturally live in
Markdown inside the repository and must outlive any model conversation.

## Decision

Gritty treats Markdown as an operational surface: source/preview/split editing,
GitHub-flavored Markdown, sanitized rendering (no raw HTML), clickable
repository paths with optional line/column references, recognition of common
instruction files (README.md, AGENTS.md) without vendor-specific behavior, and
export of agent plans/results as plain Markdown (T050 and later).

## Consequences

- Markdown rendering is part of Phase 1, not an afterthought.
- Proprietary formats are avoided whenever Markdown or normal files suffice.

## Revisit when

Structured workstream state needs a representation Markdown cannot round-trip.
