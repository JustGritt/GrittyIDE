# ADR-001: Desktop runtime is Tauri 2 (subject to the Phase 0 spike)

- **Status:** Proposed. Becomes Accepted or Rejected at the end of Phase 0, after measurements, not before.
- **Date:** 2026-09-02

## Context

Gritty is a long-running IDE that will host agents, terminals, language servers
and indexers. Baseline memory, process supervision and a strict UI/core boundary
matter more than web-platform parity. The alternatives are Electron (full
Chromium, larger footprint, uniform WebView) and Tauri (system WebView, Rust
core, smaller footprint, WebView divergence risk).

## Decision (provisional)

Use Tauri 2 with a Rust core and a React + Monaco frontend. Prove it in Phase 0
(`docs/plans/phase-0-spike.md`): WebView/Monaco behavior, filesystem boundary,
typed protocol, PTY, Markdown, persistence, failure containment, Linux build,
performance baseline.

The frontend half of that sentence is not decided here, and on review was
never argued here either: ADR-032 records why React, and why Monaco being
framework-agnostic means the two choices were always separable. The
go/no-go below is about the runtime alone.

Kill criteria (any one triggers an Electron evaluation while keeping the core
protocol):

- recurring Monaco or text-input defects on the primary platform (macOS WKWebView);
- unstable PTY/process behavior needing invasive workarounds;
- unacceptable maintenance caused by WebView divergence between macOS and Linux.

## Consequences

- Rust + TypeScript toolchain; contributors need both.
- Platform-specific behavior must sit behind adapters and tests.
- The core protocol (T010) is designed to survive a shell swap.

## Revisit when

Phase 0 exit. Record the go/no-go answers and measurements here and flip the status.
