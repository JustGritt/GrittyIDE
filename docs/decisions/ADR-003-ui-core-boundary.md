# ADR-003: The UI talks to the core only through a typed protocol

- **Status:** Accepted
- **Date:** 2026-09-02

## Context

Tauri can expose filesystem, shell and process plugins directly to the WebView.
Doing so would leak business logic into UI components, weaken the security
model and tie the UI to the shell.

## Decision

React/Monaco code calls domain commands and subscribes to domain events through
a typed client (`packages/protocol`, T010). The client is the only module that
imports `@tauri-apps/api` (during T000: `apps/desktop/src/core/bridge.ts`).
Tauri capabilities stay at the core app/event/window/webview defaults. Heavy or
failure-prone work runs in the core or in supervised processes, never in the
renderer.

## Consequences

- Every new UI capability needs a protocol command/event first.
- Shared types are generated from Rust (`ts-rs`), so Rust owns the schema.
- Swapping the desktop shell (see ADR-001) does not touch UI feature code.

## Revisit when

A concrete need cannot be expressed as a command/event pair.
