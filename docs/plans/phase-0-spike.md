# Phase 0 — Architecture and runtime spike

Executable checklist. Fill the result columns with what was actually observed
(date, machine, numbers). Nothing here is "done" until it was exercised in the
running application. Exit: ADR-001 go/no-go.

## A. Functional checklist (plan §16)

| # | Item | Task | Result |
|---|---|---|---|
| A1 | Minimal Tauri 2 + React + TypeScript application launches | T000 | |
| A2 | Monaco embedded; TypeScript, Rust, Markdown and JSON fixtures open | T040 | |
| A3 | Keyboard shortcuts, multi-cursor, selection, find, clipboard, drag/drop, font rendering verified on WKWebView | T040 | |
| A4 | Repository with 10k+ files opens without eager content loading | T020 | |
| A5 | Rust commands for directory listing, safe read, atomic write | T020/T030 | |
| A6 | Filesystem watching; rapid changes do not lock the UI | T070 (after ADR-001) or spike | |
| A7 | Markdown preview with sanitized HTML and fenced-code highlighting | T050 | |
| A8 | PTY: spawn a shell, resize, stream output, terminate cleanly | T090 | |
| A9 | Persist and restore workspace + tabs | T060 | |
| A10 | Cold/warm startup, idle RAM/CPU, editor RAM growth, interaction latency recorded | below | |
| A11 | Linux build via CI; platform-specific defects logged | T000 CI + platform matrix | |
| A12 | ADR-001 written after measurements | end | |

## B. Failure and recovery tests (addendum §11)

| # | Test | Steps | Pass condition | Result |
|---|---|---|---|---|
| B1 | Dirty-buffer crash | open workspace → edit without saving → `kill -9` Gritty → restart | unsaved content is recoverable; no silent loss | |
| B2 | PTY failure | spawn PTY → produce output → kill the child unexpectedly | renderer stays responsive; terminal reports termination | |
| B3 | Operation cancellation | start a long operation (search or fixture op) → receive progress → cancel | prompt cancellation; no stale successful completion | |
| B4 | Renderer reload | reload/recreate the WebView while core/workspace exist | durable state rehydrates without corruption | |
| B5 | Filesystem storm | generate rapid file changes in `fixtures/large-repo` | UI responsive; events batched/bounded; final state converges | |

## C. Performance baseline (plan §13 targets)

Measure on the primary development Mac with `fixtures/large-repo` and a real
repository. Record the command/method used.

| Metric | Target | Measured | Method |
|---|---|---|---|
| Cold launch to usable shell | ≤ 2 s | | |
| Warm launch | ≤ 1 s | | |
| Idle CPU (no watchers/tasks) | ~0 % | | Activity Monitor / `top` |
| Typing responsiveness | no perceptible frame drops | | |
| Open a normal source file | ≤ 100 ms perceived | | |
| Workspace restore | UI immediately, expensive work incremental | | |
| Repository enumeration | never blocks the editor; progressive | | |
| Baseline memory (shell only) | track; compare with an Electron baseline | | |
| Memory growth after 30 min editing | track | | |
| Agent/LSP/indexer crash | no renderer crash or freeze | | |

## D. Go/no-go questions (addendum §19)

Answer each explicitly with evidence.

**Editor.** Does Monaco behave correctly in WKWebView? Are shortcuts, clipboard,
multi-cursor and text input dependable? Any unresolved IME/input defects?

**Runtime.** Do filesystem operations stay outside the renderer cleanly? Can
high-frequency events be delivered without UI degradation? Does the typed
protocol stay understandable rather than becoming Tauri glue?

**Terminal.** Is PTY behavior reliable for daily interactive use? Can child
process trees be terminated predictably? Does unexpected child death stay contained?

**Recovery.** Does workspace/session state survive force termination? Can unsaved
buffers be recovered? Do interrupted operations reconcile correctly?

**Performance.** Startup acceptable? Idle CPU effectively zero? Baseline memory
materially better than Electron? Does repository size degrade progressively?

**Platform.** Does Linux remain buildable? Are WebView differences manageable
behind adapters/tests?

**Maintenance.** Are Tauri/WebView workarounds isolated? Does the Rust/TypeScript
split still produce a clean architecture? Are shell-specific hacks accumulating
faster than product functionality?

## E. Kill criteria (ADR-001)

Recurring Monaco/input defects on macOS; unstable PTY/process behavior requiring
invasive workarounds; unacceptable maintenance from WebView divergence. If
triggered: keep the core protocol, evaluate Electron as the shell.
