# ADR-002: Platform order

- **Status:** Accepted
- **Date:** 2026-09-02

## Context

Daily dogfooding happens on macOS. Linux shares most of the Rust stack but uses
WebKitGTK. Windows needs WebView2, ConPTY, different path/shell semantics,
installers and signing.

## Decision

macOS is the primary target and is dogfooded first. Linux stays continuously
buildable through CI from T000 and gets a focused hardening pass in Phase 7.
Windows is deferred until the workflow has proven its value in sustained daily
use (Phase 8).

## Consequences

- No Windows-specific code paths before Phase 8; do not claim Windows support.
- Every Phase 0 platform finding is logged in `docs/testing/platform-matrix.md`.

## Revisit when

Phase 5 exit criteria are met and the product thesis is validated.
