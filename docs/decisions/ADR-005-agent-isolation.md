# ADR-005: Agents are supervised external processes behind provider-neutral adapters

- **Status:** Accepted
- **Date:** 2026-09-02

## Context

Models and agent CLIs change quickly. Gritty must own project state and
orchestration, remain a competent editor with no provider available, and keep
agent failures from taking down the UI.

## Decision

Each agent run is a supervised process (or API session) driven through an
adapter that translates provider behavior into the Gritty run/event protocol
(T300). A deterministic fake agent is mandatory before any real provider
(T310). Permissions are explicit, scoped capabilities. Context supplied to an
agent is persisted as a manifest. Cancellation and orphan detection are hard
requirements from the first implementation.

## Consequences

- No provider concepts in the Workstream domain model.
- Orchestration can be tested offline without credentials.
- Raw transcripts are secondary; intent, decisions, state and results are persisted.

## Revisit when

An adapter cannot express a provider capability without leaking into the domain.
