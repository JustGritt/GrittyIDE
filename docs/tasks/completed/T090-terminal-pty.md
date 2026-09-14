# Task T090 — Terminal (PTY) spike

- **Phase:** 0 / 2
- **Status:** done
- **Depends on:** T010

## Objective

Prove PTY viability inside the Tauri/Rust architecture: spawn a shell through a
PTY in the core, stream output to an xterm frontend with backpressure, resize,
and terminate process trees predictably. Produce the Phase 0 evidence for the
terminal go/no-go questions.

## Dependencies

T010 (operations, events, error envelope). Independent of documents.

## Required reading

- Plan §6.1 (PTY never in the renderer), §16 (PTY checklist), §21 (PTY crate decision)
- Addendum §3.3 (backpressure), §11.2 PTY failure test, §14 environment/processes
  (shell inheritance, login vs non-login, PATH, cwd, process-tree termination)
- ADR-003; `docs/architecture/security.md`

## Allowed scope

- new `crates/terminal/**` (PTY adapter around `portable-pty`, session lifecycle, output batching)
- `crates/gritty-core` (terminal session registry, events)
- `packages/protocol` (`terminal.spawn/write/resize/kill/list`, `TerminalOutput`, `TerminalExited` events)
- `apps/desktop/src/features/terminal/**` with `@xterm/xterm` (+ fit addon)
- `apps/desktop/src-tauri` adapters; CSP only if xterm needs a directive (record why)
- dependency additions with justification (`portable-pty`, `@xterm/xterm`, `@xterm/addon-fit`)

## Explicit non-goals

- Terminal buffer persistence, profiles UI, shell integration/OSC features, split
  terminals, agent process supervision (T300), Windows/ConPTY.

## Required behavior

1. `terminal.spawn({ cwd?, shell?, env? })`: default shell is the user's login shell
   (`$SHELL`, fallback `/bin/zsh` on macOS, `/bin/bash` on Linux) started as an
   interactive login shell; cwd defaults to the workspace root; environment is inherited
   plus `TERM=xterm-256color`, `COLORTERM=truecolor`; document the inheritance policy.
2. Output streaming: reader thread per session → bounded channel → batched
   `TerminalOutput` events (flush at most ~60 times per second, coalescing chunks;
   drop-oldest with a visible "output truncated" marker if the consumer falls behind by
   more than a bound).
3. `terminal.write(id, data)` for input; `terminal.resize(id, cols, rows)` propagates to the PTY.
4. `terminal.kill(id)`: terminate the whole process group (SIGHUP → SIGTERM → SIGKILL
   with a short timeout); `TerminalExited { code, signal }` emitted exactly once.
5. Multiple concurrent sessions; sessions are listed with cwd, pid and status.
6. On application exit every session is killed and reaped.
7. xterm frontend: renders output, forwards keystrokes, resizes with its container,
   shows exit status when the shell ends.

## Failure behavior

- Addendum §11.2: killing the child unexpectedly leaves the renderer responsive and the
  session reports termination with the exit reason.
- Spawn failure (missing shell, permission) → `Io`/`PermissionDenied` error envelope.
- Output storm (`yes`, `cat` of a huge file) does not freeze the UI; batching/truncation engages.
- Session ids that do not exist → `NotFound`; double kill is idempotent.

## Acceptance criteria

- Interactive programs (`vim`, `less`, `top`, `htop` if installed) work with correct
  resizing; Ctrl+C interrupts; closing the tab kills the process tree (verified with `pgrep`).
- Output storm test keeps the UI responsive (record frame timing or a stopwatch method).
- Phase 0 terminal go/no-go questions answered with evidence in `docs/plans/phase-0-spike.md`.

## Tests required

- Rust unit/integration tests: spawn/echo/exit, resize, kill of a process tree, batching
  under a synthetic flood, events after kill are dropped.
- Vitest: terminal store reducer (sessions, exit states).
- Manual: interactive checklist recorded in the platform matrix.

## Commands to run

`pnpm check`, `pnpm dev`.

## Completion report

### Summary

PTY sessions work inside the Tauri/Rust architecture. A new `gritty-terminal`
crate is the only place that opens a pseudo-terminal; `gritty_core::terminal`
owns the registry, the ids, the workspace-relative path policy and the events;
five adapters expose `terminal.*`; an xterm dock renders sessions. Protocol
1.4.0 (additive), ADR-015, `docs/architecture/terminal.md`.

The three Phase 0 terminal questions are answered with evidence in
`docs/plans/phase-0-spike.md` §D: interactive use is reliable (with one caveat
about shell readiness, below), process trees terminate predictably, and
unexpected child death stays contained.

### Files changed

- `crates/terminal/**` (new): `host.rs` (the `PtyHost`/`PtySession`/
  `TerminalSink` seam), `system.rs` (portable-pty, reader and flusher threads,
  escalating `killpg`), `batch.rs` (coalescing, the 1 MiB cap, UTF-8 decoding),
  `shell.rs` (shell and environment policy), `error.rs`, `testing.rs`
  (`FakePtyHost`), `tests/pty.rs`.
- `crates/gritty-core/src/terminal/{mod,wire}.rs` (new); `TerminalId` in
  `protocol/ids.rs`; `TerminalOutput`/`TerminalExited` in `event/mod.rs`;
  `session/tap.rs` ignores both (terminal output is never persisted);
  `protocol/export.rs`; `PROTOCOL_VERSION` 1.3.0 → 1.4.0.
- `packages/protocol`: `commands.ts`, `client.ts`, `types.ts`, generated types,
  `terminal-client.test.ts` (new).
- `apps/desktop/src-tauri`: five adapters in `commands.rs`, registration in
  `lib.rs`, `Core.terminals` and `CoreOptions.pty_host` in `core.rs`,
  terminals killed first in `Core::shutdown`, seven tests in `tests/commands.rs`.
- `apps/desktop/src/features/terminal/**` (new): `output-bus.ts`,
  `terminal-reducer.ts`, `use-terminals.ts`, `terminal-view.tsx`,
  `terminal-panel.tsx` and two test files; the dock in `App.tsx`; styles in
  `app.css`.
- Docs: ADR-015 and its index row, `docs/architecture/terminal.md`, terminal
  sections in `security.md` and `overview.md`, Phase 0 rows A8/B2 and the
  terminal go/no-go answers, nine platform-matrix rows.

### Tests actually executed

| Command | Result |
|---|---|
| `cargo test -p gritty-terminal` | 16 unit + 16 real-PTY integration, all pass (1.6 s) |
| `cargo test -p gritty-core` | 106 pass (17 new terminal tests) |
| `cargo test -p gritty-desktop` | 26 pass (7 new terminal adapter tests) |
| `pnpm exec vitest run` | 220 pass across 39 files (22 new) |
| `pnpm --filter @gritty/desktop build` | clean; xterm's CSS lands in the app's own stylesheet |
| `GRITTY_LOG=debug pnpm dev` | boots on protocol 1.4.0; WebView health round trip completes in WKWebView 21624.4.5; no panic or error in the log |
| `pnpm check` | green |
| Linux CI | PR #8 and PR #9 both failed on the output-storm test; PR #10 carries the fix that makes it machine-independent (see the third finding) |

The real-PTY tests cover: echo and clean exit, non-zero exit codes, the
requested cwd, `TERM`/`COLORTERM`, `stty size` before and after a resize, size
clamping, process-tree kill verified by the background job's pid, an output
storm (`yes`) with truncation reported, delivery rate limiting with a resize
completing mid-flood,
`SIGKILL` behind Gritty's back, idempotent kill with writes refused afterwards,
a dropped handle killing the shell, concurrent sessions not crossing, and spawn
failures for a missing shell and an unusable cwd.

### Architectural decisions

Recorded in ADR-015. The ones that shaped the code most:

- **Two threads per session, not async.** A PTY read blocks. The reader never
  blocks on the consumer, so a flooding child cannot freeze the UI; the flusher
  waits on a condition variable, so an idle terminal costs no wakeups (the
  Phase 0 idle-CPU target) while still capping delivery at ~60 Hz.
- **The flusher owns the ending**, not the reader. That is what makes
  `TerminalExited` land exactly once, after the last byte, with nothing
  following it.
- **Drop-oldest with a reported count.** `truncatedBytes` is on every batch
  (not `skip_serializing_if`, which would let an absent field read as "no
  truncation") and the UI draws a visible marker. A gap the user cannot see is
  worse than an ugly line.
- **Output bypasses React state entirely.** An output bus hands each batch to
  `term.write`; React holds only which sessions exist and which ended.
- **The exit signal is normalized** to `SIGKILL` rather than `strsignal`'s
  platform text ("Killed: 9" on macOS, "Killed" on Linux). Raw platform output
  is not domain state.
- **`nix` over an `unsafe_code` allow** for two `killpg` calls.
- **A workspace-relative `cwd`**, resolved through the root like every other
  path (ADR-010).

### Three findings worth keeping

- **A fake host earns its place.** `FakePtyHost` calls the sink from inside
  `kill()`; the sink re-enters the manager to record the exit. The first
  version held the registry lock across that call and deadlocked. Every call
  into a session now happens with the lock released — a real bug that a
  shell-based test would have hidden behind timing.
- **An interactive shell is not ready when the PTY opens.** Input written
  before the first prompt is mangled or lost, and against a developer's own
  login shell oh-my-zsh asked about updates while autosuggestions rewrote the
  input. The integration tests now pin a shell and generate its rc files. This
  does not affect a human (who types after seeing a prompt), but anything
  driving a terminal programmatically — an agent, T300 — must wait for
  readiness.
- **A timing assertion that passes locally can be an assertion about the
  machine**, and it took two attempts to stop being one. The first output-storm
  test required the flood to overflow the production 1 MiB buffer faster than a
  60 Hz drain could empty it: true on the development Mac, false on a Linux CI
  runner, where it failed. The second attempt slowed the *consumer*, which only
  widened the drain window from 16 ms to 66 ms — `take()` empties the buffer
  completely, so a slower consumer does not change the throughput the test
  demands — and it failed on CI as well. The cap is now injectable
  (`SystemPtyHost::with_max_pending_bytes`, the same kind of test hook as
  `Database::inject_write_failures`), the integration test uses 16 KiB, which
  is smaller than one read, and the production cap is pinned by the
  deterministic unit tests in `batch.rs`. The Phase 0 performance baseline (§C)
  is all thresholds and will fail the same way if calibrated on one machine.

### Dependencies added

- `portable-pty` 0.9.0 — cross-platform PTY. The only crate in the tree that
  opens one, kept behind `gritty-terminal`.
- `nix` 0.31.3 (`signal`, `process`, unix targets only) — safe `killpg`, so no
  `unsafe_code` allow is needed. Already in the tree via portable-pty (0.28).
- `@xterm/xterm` 6.0.0 and `@xterm/addon-fit` 0.11.0 — the terminal frontend.
  No install scripts, so pnpm's `allowBuilds` stays empty. No CSP change: the
  stylesheet is bundled into the app's own, which `style-src 'self'` allows.

### Known limitations

- **Manual checks are owed.** The agent cannot see or click the WebView.
  Driving `vim`/`less`/`top`, Ctrl+C on a real program, verifying a closed tab
  with `pgrep`, and watching a flood on screen are reviewer checks; rows exist
  in `docs/testing/platform-matrix.md`.
- **Linux is untested** for this task beyond compilation; CI runs the suite but
  no human has used a terminal there. `strsignal` differences are already
  normalized away, which was the most likely divergence.
- **A process that calls `setsid` itself survives** termination, as it does
  when any terminal closes. Documented, not worked around.
- **Windows has no process-tree termination** behind this interface (ConPTY is
  a Phase 8 concern); only the direct child is reachable.
- **Ended sessions stay in the registry** until the workspace changes, so a
  long session accumulates a bounded number of dead entries. There is no
  `terminal.close` to forget one without killing it.
- **No shell integration, profiles, splits or buffer persistence** — all
  explicit non-goals.

### Follow-up tasks

- A `terminal.close` command to forget an ended session without terminating,
  and pruning of ended sessions, when the terminal UI gets its own task.
- Terminal buffer persistence, if wanted, follows the T060 recipe (a numbered
  migration plus a repository module) — but see the security section first:
  terminal content is user data.
- Shell readiness detection (OSC 133 shell integration) before any agent is
  allowed to drive a terminal (T300).
- Font size and theme for the terminal should follow the editor preferences
  once there is a settings surface; they are fixed today.
- `gritty-terminal`'s threads and escalation are what T300 process supervision
  needs; it should reuse this crate rather than copy it.

### Remaining risks

- Interactive behaviour on WebKitGTK is unverified by a human; xterm is
  well-travelled there, but Gritty's fit-and-resize loop is not.
- The ~60 Hz delivery cap is measured against a synthetic flood, not against a
  real workload with the editor also running. The Phase 0 performance baseline
  (§C) is still empty and should be measured with a terminal open.
- `portable-pty`'s `Child` is downcast to `std::process::Child` to read the
  numeric exit signal. It degrades to the string form if that ever stops
  working, but it is an implementation detail of a dependency.

