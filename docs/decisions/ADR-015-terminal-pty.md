# ADR-015: Terminal sessions over a PTY

- **Status:** Accepted
- **Date:** 2026-09-04

## Context

Plan §6.1 puts the PTY outside the renderer and §21 left the crate choice open.
T090 has to answer the Phase 0 terminal go/no-go questions with evidence: can a
shell be driven reliably, can its process tree be terminated predictably, and
does an unexpected child death stay contained. Addendum §3.3 forbids flooding
the renderer with high-frequency output, and §14 asks for an explicit shell,
environment and process-termination policy. The command/event envelopes freeze
after Phase 0, so the shape recorded here is the shape that has to last.

## Decision

- **Library and layout.** `portable-pty` 0.9 in a new crate, `gritty-terminal`,
  the only place in the tree that opens a pseudo-terminal. It exposes a
  `PtyHost` trait; `gritty-core` owns the session registry, the ids, the path
  policy and the events, and never sees `portable-pty`. A `FakePtyHost` ships
  with the crate so the core's tests and the desktop adapter tests never spawn
  a shell. `portable-pty` was chosen over calling `openpty`/`forkpty` directly
  because it already handles `setsid`, the controlling-terminal `ioctl` and the
  fd hygiene that a hand-rolled version gets wrong quietly; ConPTY support it
  also has is not exercised (Windows is a Phase 8 target).
- **Threads, not async.** A PTY read is a blocking file descriptor read. Each
  session runs a reader thread and a flusher thread rather than occupying a
  tokio worker: the reader never blocks on the consumer, so a child that writes
  faster than the UI consumes is never throttled into freezing it.
- **Backpressure.** The reader appends into a per-session buffer capped at
  1 MiB. Past the cap the *oldest* bytes are dropped, not the newest — a
  terminal is a tail — and the dropped byte count travels with the next batch
  as `truncatedBytes`, which the UI draws as a visible marker. Delivery is rate
  limited to ~60 Hz by sleeping after each flush; the flusher waits on a
  condition variable rather than a timer, so an idle terminal costs no wakeups
  at all (the Phase 0 idle-CPU target). The first byte after a quiet period is
  delivered immediately, so interactive echo keeps its latency.
- **Bytes and text.** Output is decoded to UTF-8 in the core, holding back an
  incomplete sequence split across a batch boundary; genuinely invalid bytes
  become U+FFFD, which is what a terminal shows for them anyway. Base64 with a
  `Uint8Array` in the renderer was rejected: it costs a third more payload and
  a dependency to preserve bytes that xterm would render identically.
- **The ending belongs to the flusher.** The reader reaps the child and hands
  the ending over; the flusher delivers the last batch and only then calls
  `exited`. That is what makes `TerminalExited` land exactly once, after every
  byte the program wrote, with nothing following it.
- **Termination.** `killpg` through `nix` 0.31 (a safe wrapper: two signal
  calls did not justify an `unsafe_code` allow), escalating SIGHUP → SIGTERM →
  SIGKILL with 250 ms between steps, over *two* process groups: the shell's own
  (it is a session leader, so its pgid is its pid) and whatever group currently
  owns the terminal, re-read between signals because a shell starts and
  finishes jobs. SIGHUP is first because an interactive shell forwards it to
  its jobs, which is how closing a terminal window has always ended what was
  running in it. Dropping a session handle terminates it, so losing a handle
  cannot leak a shell.
- **Shell and environment.** The user's login shell, `argv[0]` prefixed with
  `-`, exactly as a terminal emulator runs it: an app launched from Finder
  inherits a minimal `PATH`, and a non-login shell would leave the terminal
  unable to find tools the user has in their own terminal. Resolution order is
  an explicit `shell`, then `$SHELL`, then the platform default, then
  `/bin/sh`; a candidate that is not an executable file is an error, never a
  silent fall-through. The child inherits Gritty's environment plus
  `TERM=xterm-256color` and `COLORTERM=truecolor`.
- **Working directory.** A workspace-relative `WorkspacePath` resolved through
  the workspace root (ADR-010), defaulting to the root. The WebView cannot ask
  for a shell rooted anywhere it likes. The user may `cd` afterwards; this
  bounds where Gritty *starts* a shell, not what the user may do in one.
- **Protocol** (1.4.0, additive): `terminal.spawn/write/resize/kill/list`,
  events `TerminalOutput` and `TerminalExited`, a new `TerminalId`. `kill` is
  idempotent. An ended session stays listed with its exit status until the
  workspace closes, so the user can still read why it ended. Sessions are
  bounded at 16 and a single write at 64 KiB.
- **Frontend.** `@xterm/xterm` 6.0 with the fit addon. Output does not pass
  through React state: an output bus hands each batch straight to
  `term.write`, and React holds only which sessions exist and which ended.
  xterm reports the grid it settled on and that is what the PTY is told, so the
  child's `stty size` matches what is drawn.

## Consequences

- Terminal buffers are not persisted (a T090 non-goal) and terminal output is
  never logged: it is user data and routinely contains secrets. Only session
  ids, pids and sizes reach the logs.
- A process that puts itself in a new session (`setsid`, a daemon) survives
  termination, exactly as it survives closing any terminal. This is documented
  rather than worked around.
- Two threads per session. At 16 sessions that is 32 threads, idle-blocked on
  a read or a condition variable.
- The renderer cannot show output it was not given: after truncation the marker
  is the only record of what was dropped. That is deliberate — the alternative
  is an unbounded buffer or a frozen UI.
- Windows has no process-tree termination behind this interface; only the
  direct child can be reached until ConPTY work happens in Phase 8.

## Revisit when

Terminal buffer persistence, split terminals, shell integration (OSC 133) or
profiles are wanted; when agent process supervision (T300) needs the same
mechanics, which should reuse this crate rather than copy it; or when Windows
becomes a target.
