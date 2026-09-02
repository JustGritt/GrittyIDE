# Task T090 — Terminal (PTY) spike

- **Phase:** 0 / 2
- **Status:** todo
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

(fill in)
