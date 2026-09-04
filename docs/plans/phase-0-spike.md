# Phase 0 — Architecture and runtime spike

Executable checklist. Fill the result columns with what was actually observed
(date, machine, numbers). Nothing here is "done" until it was exercised in the
running application. Exit: ADR-001 go/no-go.

## A. Functional checklist (plan §16)

| # | Item | Task | Result |
|---|---|---|---|
| A1 | Minimal Tauri 2 + React + TypeScript application launches | T000 | |
| A2 | Monaco embedded; TypeScript, Rust, Markdown and JSON fixtures open | T040 | 2026-09-03, macOS 26 arm64: Monaco 0.56.0 embedded (`packages/editor`, ADR-012); the bundle loads in WKWebView 21624.4.5 and the health round trip completes (`pnpm dev`); opening the `small-repo` fixtures in the running app pending reviewer (the agent cannot see the WebView) |
| A3 | Keyboard shortcuts, multi-cursor, selection, find, clipboard, drag/drop, font rendering verified on WKWebView | T040 | pending reviewer; rows and steps in `docs/testing/platform-matrix.md` (T040 completion report lists the exact checks) |
| A4 | Repository with 10k+ files opens without eager content loading | T020 | |
| A5 | Rust commands for directory listing, safe read, atomic write | T020/T030 | 2026-09-03, macOS 26 arm64: `workspace.list` (T020) and `documents.open/save` (T030) through the real adapters in `cargo test -p gritty-desktop`; atomic write with injected failure/crash in `cargo test -p gritty-workspace`; UI pass pending reviewer |
| A6 | Filesystem watching; rapid changes do not lock the UI | T070 (after ADR-001) or spike | |
| A7 | Markdown preview with sanitized HTML and fenced-code highlighting | T050 | 2026-09-03, macOS 26 arm64: markdown-it + DOMPurify allowlist (ADR-013), fenced code colorized by Monaco's tokenizers; XSS fixture asserted clean and GFM fixture snapshotted under jsdom; `long.md` (134 KB) renders in 366 ms under jsdom (markdown-it alone 15–20 ms); in-app rendering, scroll sync and link navigation pending reviewer (the agent cannot see the WebView) |
| A8 | PTY: spawn a shell, resize, stream output, terminate cleanly | T090 | 2026-09-04, macOS 26 arm64: `portable-pty` 0.9 behind `gritty-terminal` (ADR-015); `cargo test -p gritty-terminal` drives a real PTY through echo, exit codes, `TERM`, `stty size` before and after a resize, a `yes` flood, an unexpected `SIGKILL` and a process-tree kill verified by pid (15 tests, 1.6 s); `pnpm dev` boots on protocol 1.4.0 with the xterm bundle loaded in WKWebView 21624.4.5; driving a shell in the window pending reviewer (the agent cannot see the WebView) |
| A9 | Persist and restore workspace + tabs | T060 | 2026-09-04, macOS 26 arm64: SQLite session database (ADR-014); `session.restore` reopens the last root and returns tabs, view modes, cursors, preferences and layout; automated through the adapters (`cargo test -p gritty-desktop`) and the WebView restore flow (Vitest); `pnpm dev` twice restores the window geometry; a quit/relaunch with open tabs in the app pending reviewer |
| A10 | Cold/warm startup, idle RAM/CPU, editor RAM growth, interaction latency recorded | below | |
| A11 | Linux build via CI; platform-specific defects logged | T000 CI + platform matrix | |
| A12 | ADR-001 written after measurements | end | |

## B. Failure and recovery tests (addendum §11)

| # | Test | Steps | Pass condition | Result |
|---|---|---|---|---|
| B1 | Dirty-buffer crash | open workspace → edit without saving → `kill -9` Gritty → restart | unsaved content is recoverable; no silent loss | 2026-09-04, macOS 26 arm64: automated in `crates/gritty-core/tests/crash_recovery.rs` (snapshot written within the debounce, process state dropped without close/flush/checkpoint, relaunch offers `SAME_AS_DISK` and recovers the text; changed and deleted files recover as `CONFLICTED` / `DELETED_EXTERNALLY`) and through the adapters; the real `kill -9` of the app while typing pending reviewer |
| B2 | PTY failure | spawn PTY → produce output → kill the child unexpectedly | renderer stays responsive; terminal reports termination | 2026-09-04, macOS 26 arm64: `killing_the_child_unexpectedly_reports_termination` (`crates/terminal/tests/pty.rs`) SIGKILLs the shell behind Gritty's back; the session reports `signal: SIGKILL`, `code: None`, exactly one `TerminalExited`, and no output after it. Renderer containment is structural (output never enters React state, delivery capped at ~60 Hz) and is asserted in `an_output_storm_stays_bounded_and_reports_what_it_dropped`: a resize still completes while `yes` floods. The renderer staying responsive on screen is pending reviewer |
| B3 | Operation cancellation | start a long operation (search or fixture op) → receive progress → cancel | prompt cancellation; no stale successful completion | 2026-09-03, demo operation (T010): automated in `cargo test -p gritty-core` (registry) and `cargo test -p gritty-desktop` (mock runtime through the real adapters): progress observed → cancel → `CANCELLED` in < 1 s, no `OperationCompleted`, `seq` contiguous. Manual UI pass pending reviewer; re-run with search (T080) |
| B4 | Renderer reload | reload/recreate the WebView while core/workspace exist | durable state rehydrates without corruption | 2026-09-04: `documents.list` + `documents.open` rehydration kept (`useDocuments.rehydrate`, always run by the session flow); a second `session.restore` in the same process reports the open workspace without reopening (core and adapter tests); the WebView flow is unit-tested for the reload path; a Cmd+R in `pnpm dev` with dirty tabs pending reviewer |
| B5 | Filesystem storm | generate rapid file changes in `fixtures/large-repo` | UI responsive; events batched/bounded; final state converges | |

## C. Performance baseline (plan §13 targets)

Measure on the primary development Mac with `fixtures/large-repo` and a real
repository. Record the command/method used.

| Metric | Target | Measured | Method |
|---|---|---|---|
| Cold launch to usable shell | ≤ 2 s | | |
| Warm launch | ≤ 1 s | | |
| Idle CPU (no watchers/tasks) | ~0 % | | Activity Monitor / `top` |
| Typing responsiveness | no perceptible frame drops | pending | T040: type for 30 s in `weird-files/generated/large-text.txt` after "Enable editing" with the Safari Web Inspector timeline recording; report frames over 16 ms; repeat in `small-repo/src/*.ts` |
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

*2026-09-04, macOS 26 arm64 (T090, ADR-015).* **Reliable enough to keep going,
with one caveat.** A real shell runs, echoes, reports its exit code, and
`stty size` follows a resize; `TERM=xterm-256color` reaches the child;
concurrent sessions do not cross. The caveat is not the PTY but the shell: an
interactive line editor is not ready the instant the PTY opens, and input
written before the first prompt is silently mangled or lost. A user types after
seeing a prompt, so this does not affect interactive use — but anything driving
a terminal programmatically (an agent, T300) must wait for readiness, and the
integration tests had to pin a shell and generate its rc files to be
deterministic at all. Running a developer's own login shell, oh-my-zsh asked
about updates and autosuggestions rewrote the input as it arrived.

**Process trees: yes, with a documented limit.** `killpg` over the shell's own
process group *and* the terminal's current foreground group, escalating
SIGHUP → SIGTERM → SIGKILL, re-reading the foreground group between signals.
`kill_terminates_the_whole_process_tree` starts a background job (`sleep 300 &`,
its own process group under job control), records its pid, kills the session
and asserts the pid is gone — so the SIGHUP-first design that relies on the
shell forwarding to its jobs is verified, not assumed. Dropping a session
handle kills it, and `Core::shutdown` kills every session before the session
writer flushes. A process that puts itself in a new session (`setsid`, a
daemon) survives, exactly as when any terminal closes; that is recorded rather
than worked around.

**Unexpected death is contained.** SIGKILL behind Gritty's back produces one
`TerminalExited` carrying `SIGKILL`, after the last output and with nothing
following it, and the registry marks the session `EXITED` and drops the pid.
The exit signal is normalized: `strsignal` says "Killed: 9" on macOS and
"Killed" on Linux, which is platform text, not a protocol value.

**Flooding does not reach the renderer.** `yes` overflows the 1 MiB cap; the
oldest bytes go, the count is reported as `truncatedBytes` and drawn as a
visible marker, batches stay under the cap and delivery stays rate limited
(asserted against a measured ceiling of ~70/s). A resize completes during the
flood, so the control path is not starved. Output never passes through React
state at all.

Remaining for the reviewer: the same checks with a human at the window (`vim`,
`less`, `top`, Ctrl+C, tab close verified with `pgrep`, the flood on screen).

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
