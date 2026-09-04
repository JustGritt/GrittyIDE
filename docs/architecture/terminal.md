# Terminal sessions (T090)

How a keystroke reaches a shell and how its output comes back. Decisions and
their rationale are in [ADR-015](../decisions/ADR-015-terminal-pty.md); this
document is the map.

## Layers

```
apps/desktop/src/features/terminal   xterm view, tabs, output bus
        │  terminal.spawn/write/resize/kill/list, TerminalOutput, TerminalExited
apps/desktop/src-tauri/commands.rs   adapters (parse, delegate, correlate)
        │
gritty_core::terminal                registry, ids, path policy, events
        │  PtyHost / PtySession / TerminalSink
crates/terminal (gritty-terminal)    portable-pty, reader + flusher threads, killpg
```

`gritty-core` never sees `portable-pty`, and `gritty-terminal` never sees a
`TerminalId`, a workspace or an event. The seam is three small traits, which is
what lets the core's tests and the desktop adapter tests run against
`gritty_terminal::testing::FakePtyHost` without spawning a shell.

## A session's life

**Spawn.** `terminal.spawn` takes a `workspaceId` and an optional
workspace-relative `cwd`. The core resolves the cwd through the workspace root,
so a path outside it is refused before anything is allocated; it resolves the
shell (explicit → `$SHELL` → platform default → `/bin/sh`, each required to be
an executable file) and asks the host for a session. The host opens a PTY,
starts the shell as a login shell, drops the slave fd — without that the master
never reaches EOF and the session would look alive forever after the shell
exits — and starts two threads.

**Output.** The reader thread blocks on the master and appends into a
`PendingOutput` buffer. It never blocks on the consumer: a child writing faster
than the UI consumes is discarded from the *oldest* end at a 1 MiB cap, and the
dropped byte count rides along with the next batch. The flusher thread waits on
a condition variable, so an idle terminal costs nothing; on waking it delivers
one coalesced batch, then sleeps 16 ms, which caps delivery at ~60 Hz and
coalesces whatever arrives meanwhile. Batches are decoded to UTF-8 with an
incomplete trailing sequence held back for the next flush.

**Input.** `terminal.write` sends the string verbatim, control characters
included: Ctrl+C is `U+0003` and the shell, not Gritty, decides what it means.
Writes are bounded at 64 KiB.

**Resize.** xterm's fit addon settles on a grid and reports it; that grid is
what the PTY is told, so the child's `stty size` matches what is drawn. Absurd
sizes are clamped rather than rejected — a container mid-layout can legitimately
report zero height.

**Ending.** The reader reaps the child, records the exit and marks the buffer
finished; the *flusher* delivers the last batch and only then `exited`. So
`TerminalExited` lands exactly once, after every byte the program wrote, and
nothing follows it. The core marks the session `EXITED`, clears the pid and
keeps it listed: the user can still read why it ended. The UI drops it when the
tab is closed, and the core drops everything when the workspace changes.

**Termination.** `terminal.kill` starts an escalation on its own thread and
returns immediately: SIGHUP, SIGTERM, SIGKILL, 250 ms apart, sent to the
shell's process group *and* to whatever group currently owns the terminal,
re-read between signals. SIGHUP first is what reaches a job the shell started
in its own process group, because an interactive shell forwards it. Killing an
ended session is a no-op, so closing a tab twice is never an error. Dropping a
session handle kills it; `Core::shutdown` kills every session before the
session writer is flushed.

## What is deliberately not here

- **No terminal buffer persistence.** Nothing a shell prints is written to the
  session database, and nothing is logged. Terminal content is user data and
  routinely contains secrets.
- **No shell integration, profiles or splits.** T090 is a spike; these are
  product features with their own tasks.
- **No escape from a new session.** A process that calls `setsid` itself, or a
  daemon, survives termination — as it survives closing any terminal.
- **No Windows process-tree termination.** ConPTY is a Phase 8 concern; outside
  unix only the direct child is reachable.

## Bounds

| Bound | Value | Why |
|---|---|---|
| Pending output per session | 1 MiB | more scrollback than fits between two frames |
| Delivery rate | ~60 Hz | one batch per frame, not one per read |
| Read size | 64 KiB | one `read` per wakeup under a flood |
| Concurrent sessions | 16 | each costs a shell, two threads and a buffer |
| Single write | 64 KiB | a paste is legitimate; a megabyte from the renderer is not |
| Buffered chunks before the view mounts | 256 | keeps the first prompt, bounded |

## Tests

- `crates/terminal/tests/pty.rs` — a real PTY: echo, exit codes, cwd, `TERM`,
  resize through `stty size`, process-tree kill verified by pid, output storm,
  unexpected `SIGKILL`, idempotent kill, concurrent sessions, spawn failures.
  These pin a shell and generate its rc files: run against a developer's own
  login shell, an interactive line editor rewrites input as it arrives and the
  assertions depend on whose machine they run on.
- `gritty_core::terminal::tests` — registry bookkeeping against the fake host.
- `apps/desktop/src-tauri/tests/commands.rs` — the five adapters through the
  real command wiring on Tauri's mock runtime.
- `apps/desktop/src/features/terminal/*.test.ts` — the reducer and the output
  bus.
