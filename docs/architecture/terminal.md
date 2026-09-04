# Terminal sessions (T090, T095)

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

## Where a shell starts (T095)

A terminal does not need a workspace. `terminal.spawn` without a `workspaceId`
starts the shell in the user's home directory (`$HOME`, then the account's
password-database entry); with one, it starts at the workspace root or at a
workspace-relative `cwd`. A `cwd` without a `workspaceId` is `InvalidInput` —
there is nothing to resolve it against.

This keeps ADR-010's property rather than weakening it: the renderer names a
*workspace*, or names nothing, and the core turns that into a directory. There
is still no way to root a shell at an arbitrary path from the WebView. Both
`workspaceId` and `cwd` are absent on a home session, which is why they are
optional on `Terminal` — the protocol does not put absolute paths on the wire,
so a home directory has nothing workspace-relative to report.

Sessions are not partitioned by workspace. Closing or replacing a workspace
leaves its shells running, and the panel keeps showing them: the core does not
kill them, so a UI that hid them would leave the user with processes they could
neither see nor terminate.

## Naming a session (T047)

`terminal.rename` gives a session a label, and the tab shows it instead of the
shell's name. The shell is the fallback, not the answer: once sessions hold an
agent, a dev server and a build, three tabs reading "zsh" identify none of
them.

The label is **core state**, not renderer state. `terminal.list` is what
rehydrates the panel after a renderer reload, so a name kept only in the
renderer would vanish while the session it described kept running. It returns
the updated snapshot like `resize` and `kill` do, and emits no event — there is
nothing another subscriber needs to react to.

A blank name clears the label rather than setting an empty one; a tab with no
text is unclickable. Control characters are refused and the length is bounded
in characters, not bytes, because the bound exists so a tab stays a tab and a
tab's width is measured in what is drawn. An ended session can still be
renamed: "this is the build that failed" is exactly when a label earns its
keep, and that is only known afterwards.

Renaming is manual. Detecting what a session is *running* — so a tab could
label itself `claude` — is shell integration (OSC 133 and the process title),
which is a T090 non-goal and a task of its own.

## Choosing a shell (T095)

`terminal.profiles` reports the shells the picker should offer. Discovery lives
in `gritty-terminal` behind `PtyHost::shells`, next to the PTY, because "which
shells exist" is the same operating-system knowledge as "how a PTY opens" — and
putting it on the host is what lets the core test the command against a fake
instead of the developer's own `/etc/shells`.

**Two sources, both authoritative.** `$SHELL`, resolved exactly the way a spawn
resolves it, so the entry marked `isDefault` is the one the user actually gets;
and `/etc/shells`, which *is* the operating system's register of valid login
shells — the list `chsh` validates against. Candidates go through the same
executable check a spawn uses and are deduplicated by canonical path, so a
merged-`/usr` system does not offer `/bin/bash` and `/usr/bin/bash` as two
shells.

**Deliberately nothing else.** An earlier version also scanned the usual `bin`
directories for a fixed set of shell *names*. On the first machine it ran
against, that offered `/usr/local/bin/nu` — a symlink to an npm package
manager's helper, not nushell. A file name is not evidence that a program is a
shell. A picker that lists things the user cannot use is worse than one that
misses something they can add by hand, so the guessing was removed and the
adding was built.

**Custom shells** come in through the request: `terminal.profiles({ custom })`
takes absolute paths the user configured and checks each one like a discovered
shell. An entry that cannot be started is *reported* — `available: false` with
a reason — rather than dropped, because the user typed it and is owed the
explanation. The list is bounded (`MAX_CUSTOM_SHELLS`). This is the escape
hatch for a Homebrew `fish` that was never added to `/etc/shells`.

Discovery starts no process. It stats a handful of paths, and it is re-run on
every call rather than cached — a user who installs a shell expects to find it
without restarting Gritty.

The UI remembers the choice as a **path**, in the persisted layout blob, not as
an index or a name: the list can change between runs. A remembered path the
machine no longer has, or one that is listed but unavailable, falls back to the
default rather than failing a spawn the user cannot fix. Choosing the default
profile sends *no* `shell` to `terminal.spawn`, so the core applies its own
resolution order instead of the UI pinning a path that was correct when the
list was fetched. Unavailable profiles appear in the settings, where they can
be explained or removed, and never in the picker.

**Fonts.** A configured prompt (powerlevel10k, starship, oh-my-posh) draws its
separators from the Private Use Area, and a font without those code points
renders every glyph as an empty box — which reads as a broken terminal rather
than a missing font. The default font stack therefore names the widely
installed patched families before the ordinary monospace ones; a family that is
not installed is skipped, so this costs nothing on a machine without one. The
terminal settings override the family and the size, and a change is applied to
a live terminal in place — the grid is recomputed and the PTY told, so the
shell stops wrapping at the old width, and the scrollback survives.

## What is deliberately not here

- **No terminal buffer persistence.** Nothing a shell prints is written to the
  session database, and nothing is logged. Terminal content is user data and
  routinely contains secrets.
- **No shell integration or splits.** T090 is a spike; these are product
  features with their own tasks. A tab that names itself after the program it
  is running needs OSC 133 and lives there.
- **No named profiles.** T095 lets the user pick a shell and add a path. A
  *named* profile with its own arguments, environment and working directory
  needs a document to persist it in, not a list of paths.
- **No escape from a new session.** A process that calls `setsid` itself, or a
  daemon, survives termination — as it survives closing any terminal.
- **No Windows process-tree termination.** ConPTY is a Phase 8 concern; outside
  unix only the direct child is reachable.

## Bounds

| Bound | Value | Why |
|---|---|---|
| Pending output per session | 1 MiB | more scrollback than fits between two frames (`SystemPtyHost::with_max_pending_bytes` lowers it for tests) |
| Delivery rate | ~60 Hz | one batch per frame, not one per read |
| Read size | 64 KiB | one `read` per wakeup under a flood |
| Concurrent sessions | 16 | each costs a shell, two threads and a buffer |
| Single write | 64 KiB | a paste is legitimate; a megabyte from the renderer is not |
| Buffered chunks before the view mounts | 256 | keeps the first prompt, bounded |
| Custom shells | 16 | the picker is a menu, not a database |
| Session name | 64 characters | a tab has to stay a tab; counted in what is drawn |

A session started without a workspace lives in `$HOME` and is bound to no
workspace lifetime; see ADR-015's T095 amendment.

## Tests

- `crates/terminal/tests/pty.rs` — a real PTY: echo, exit codes, cwd, `TERM`,
  resize through `stty size`, process-tree kill verified by pid, output storm
  (against an injected 16 KiB cap, so overflow does not depend on how fast the
  machine is), delivery rate limiting, unexpected `SIGKILL`, idempotent kill,
  concurrent sessions, spawn failures.
  These pin a shell and generate its rc files: run against a developer's own
  login shell, an interactive line editor rewrites input as it arrives and the
  assertions depend on whose machine they run on.
- `gritty_core::terminal::tests` — registry bookkeeping against the fake host.
- `apps/desktop/src-tauri/tests/commands.rs` — the six adapters through the
  real command wiring on Tauri's mock runtime.
- `apps/desktop/src/features/terminal/*.test.ts` — the reducer, the output bus,
  the profile fallback and adding a custom shell.
- `apps/desktop/src/features/session/persisted-layout.test.ts` — the settings
  blob, including one written before the terminal section existed.
