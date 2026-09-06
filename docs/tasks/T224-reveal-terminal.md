# Task T224 — Reveal the terminal a session is running in

- **Phase:** 3
- **Status:** todo
- **Depends on:** T220 (agent session discovery)

## Objective

T220 put a pid and a tty on every roster row and wrote down what they are for:

> The pid and tty are shown so the user can go and find the terminal
> themselves; that is the whole interaction model.

That is honest, and it is also a chore. This task turns it into one action:
**copy the identifiers always, and, where the platform can do it reliably,
focus the terminal window that owns the session.**

This is the smallest useful step across the line from *showing* to *acting*,
and it is deliberately the only one in this plan: focusing a window changes
nothing about the agent, grants no ability to send it input, and cannot corrupt
anything if it fails.

## Dependencies

- **T220** for `AgentSession.pid` and `AgentSession.tty`, already on the wire.
- Nothing else. This task is independent of T221–T223 and can land in any order
  relative to them.

## Required reading

- `docs/architecture/agent-sessions.md` — what the roster already knows
- `docs/architecture/security.md` — the boundary this must not widen
- `docs/decisions/ADR-002-platform-order.md` — the platform policy
- `crates/terminal/src/system.rs` — how a platform adapter is isolated behind a
  trait, and the shape to copy
- `crates/gritty-core/src/agent_session/wire.rs` — `pid`, `tty`
- `AGENTS.md` hard rules 1, 11 and 12

## Allowed scope

- `crates/platform/**` — new crate, `gritty-platform`
- `crates/gritty-core/src/agent_session/**` — the command's domain side
- `crates/gritty-core/src/protocol/{mod.rs,export.rs}`
- root `Cargo.toml`, `crates/gritty-core/Cargo.toml`
- `apps/desktop/src-tauri/src/{commands.rs,core.rs,lib.rs}`
- `packages/protocol/src/{client.ts,commands.ts,types.ts,index.ts}` + tests
- `apps/desktop/src/features/agents/**`
- `docs/architecture/agent-sessions.md`, `docs/tasks/README.md`, this file

## Explicit non-goals

- **No input.** Focusing a window is the entire capability. No keystrokes, no
  paste, no `write` to a tty, no signals. `AgentSession.pid`'s doc comment —
  "Gritty never signals it" — remains true after this task.
- **No arbitrary command execution from the renderer.** The renderer sends a
  session id. It does not send a program, a script, an emulator name or a path.
  Hard rule 1 is the reason, and it is the main design pressure here.
- **No claim of Linux or Windows support.** Implement what can be run and
  tested; everything else returns "unsupported on this platform" and the UI
  offers copy instead (rule 11, rule 12).
- **No emulator configuration, no user-supplied focus command.** A settable
  "command to run to focus my terminal" is exactly the arbitrary-execution hole
  this task must not open. If the built-in support misses an emulator, the
  outcome is copy-to-clipboard, and the gap is a follow-up.
- **No window management beyond focus.** No moving, resizing, arranging or
  raising Gritty's own window as a side effect.
- **No tmux/screen attach.** Recognising that a session is inside a multiplexer
  and reporting it is in scope; attaching to it is not.

## Required behavior

1. `agents.reveal { sessionId }` asks the platform to focus the terminal window
   that owns that session's controlling tty. It returns what actually happened:
   `FOCUSED | UNSUPPORTED | NOT_FOUND | FAILED`, with a reason. It never claims
   success it did not verify.
2. **The renderer names a session, never a command.** The core resolves the
   session to a tty and pid from the roster it already holds, and the platform
   crate turns that into a focus attempt. There is no path from the WebView to
   an arbitrary program, which is what keeps hard rule 1 intact.
3. **Copy always works**, on every platform and for every emulator, including
   when focus is unsupported: the row offers the tty and the pid as text. This
   is the guaranteed floor, and the feature is worth landing even if focus never
   works anywhere.
4. **Resolution is best-effort and must degrade honestly.** The chain is
   tty → owning process → emulator application → window. Each link can fail:
   a tty with no recoverable owner, an owner Gritty does not recognise, an
   emulator with no scriptable window model. Each failure returns `NOT_FOUND`
   or `UNSUPPORTED` with the link that broke named in the reason, so the
   completion report can say which emulators were verified rather than which
   were hoped for.
5. **A multiplexer is reported, not entered.** If the tty's owner is `tmux` or
   `screen`, focusing the emulator window is still correct and useful; say in
   the response that the session is inside a multiplexer so the UI can add
   "…in tmux" rather than implying the pane will be on screen.
6. **The platform crate is small and isolated** (rule 11), mirroring
   `crates/terminal/src/system.rs`: a trait in the core's view, one
   implementation per platform, a no-op implementation elsewhere, and no
   platform API visible outside the crate. `gritty-core` never learns what an
   emulator is.
7. On macOS the implementation may drive the system's own scripting interface.
   The script is a **compile-time constant with no interpolation of any value
   derived from the roster** — no tty, path, title or process name may be
   concatenated into it. Anything variable is passed as an argument. Roster
   fields are untrusted input (rule 13), and a script built by string
   concatenation from them is an injection.
8. Any subprocess the implementation runs is bounded by a timeout, has its
   output drained while it runs — T220 already lost a scan to a deadlocked
   `ps` that filled a pipe buffer — and never blocks a command handler or the
   renderer (rules 8 and 10).
9. Protocol version bumps MINOR: the command and its result type are additive.
10. The UI shows reveal as an action on a roster row, with copy always present
    beside it. An `UNSUPPORTED` result hides the action for the rest of the
    session rather than offering a button that never works.

## Failure behavior

| Situation | Behavior |
|---|---|
| Session not in the roster | `NOT_FOUND`. The roster is a snapshot; a row can vanish between render and click |
| Session has no tty (transcript-only evidence) | `NOT_FOUND`, reason "no controlling terminal". Copy still offers the pid if there is one |
| tty exists but its owner cannot be resolved | `NOT_FOUND`, naming the broken link |
| Owner is an emulator with no window model, or one not recognised | `UNSUPPORTED`. Copy is the answer |
| Emulator is running but the window was closed | `NOT_FOUND` |
| Scripting permission denied by the OS | `FAILED` with the reason. Gritty does not prompt repeatedly, and does not tell the user to grant a permission it cannot verify |
| The focus subprocess hangs | Killed at the timeout; `FAILED`. Output is drained throughout so the timeout is real and not a deadlock |
| Non-macOS platform | `UNSUPPORTED` from the no-op implementation. Nothing claims otherwise |
| Session is inside tmux/screen | Focus the emulator, report the multiplexer (§5) |
| Called twice quickly | Idempotent; the second call is another focus attempt, not an error |

## Security

- The renderer sends a session id and nothing else. No program, path, emulator
  name or script reaches the core from the WebView (hard rule 1).
- Roster-derived values are untrusted input (rule 13). Nothing from a tty,
  title, path or process name is interpolated into a script or a shell command;
  variable data is passed as arguments to a constant program.
- The capability is focus only. No signal, no write, no input injection.
- Tracing records the outcome and the emulator class, never a path or a title.

## Acceptance criteria

- With Claude Code running in a terminal on the development machine, revealing
  its roster row focuses that terminal window.
- With the same session inside tmux, the emulator window is focused and the
  response reports the multiplexer.
- Copy places the tty and pid on the clipboard for every row that has them,
  including rows where focus is unsupported.
- A row with no tty offers copy of what exists and no reveal action.
- Nothing in the WebView can cause any program other than the constant the
  platform crate names to be executed — verified by reading the command
  construction, and asserted by a test that a hostile tty string is passed as an
  argument rather than concatenated.
- The completion report names exactly which emulators were tested and which
  were not, per rule 12.
- `pnpm check` green; generated protocol types fresh and committed.

## Tests required

- **Unit (Rust):** resolution against captured process-table output through an
  injected command runner, as T220's probe tests do — no test spawns a
  subprocess. Cases: resolved, tty absent, owner unresolvable, owner
  unrecognised, owner is tmux.
- **Unit (Rust):** a tty or title containing quotes, semicolons, newlines and
  shell metacharacters is passed as an argument and never concatenated; assert
  on the constructed argv.
- **Unit (Rust):** the subprocess timeout fires and the call returns `FAILED`
  rather than hanging.
- **Unit (Rust):** the no-op platform implementation returns `UNSUPPORTED` for
  every input.
- **Unit (Rust):** `agents.reveal` for an id absent from the roster is
  `NOT_FOUND`.
- **Unit (TS):** the client method; the row hides reveal after `UNSUPPORTED`.
- **Manual:** at minimum the development machine's own emulator, plus one other
  if available, and a tmux session. Record which.

## Commands to run

```
cargo test -p gritty-platform
cargo test -p gritty-core
cargo clippy --workspace --all-targets -- -D warnings
pnpm check
```

## Completion report

_(filled in by the implementing session — including, per rule 12, the explicit
list of emulators and platforms actually exercised, and those left unclaimed)_
