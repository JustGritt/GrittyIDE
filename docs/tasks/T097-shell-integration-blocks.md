# Task T097 — Shell integration and command blocks

- **Phase:** 2
- **Status:** todo
- **Depends on:** T090, T095, T096

## Objective

The other half of design artboard 8a: the terminal's output grouped **per
command**, each block carrying the command, how long it took and how it ended,
with a coloured bar down its left edge — "so a failure is scannable without
reading scrollback". That requires knowing where a command starts and stops,
which a stream of bytes does not say. This task adds OSC 133 shell integration
and renders the blocks over xterm.

## Motivation

Everything the dock says today is about the *session*: which shell, where, and
whether it is still alive. Nothing is about the *work* — and the work is what
the user came for. A `pnpm check` that failed 400 lines ago is currently found
by scrolling.

It is also the missing input for two things already drawn elsewhere: 8b's
session rail subtitles (`cargo test · exit 101`) and T047's deferred automatic
titles (a tab naming itself `claude` after what it is running).

## Dependencies

T090 (`SpawnRequest.env`, which `TerminalManager::spawn` currently always passes
empty — `crates/gritty-core/src/terminal/mod.rs:194`), T095 (the settings
surface an opt-out belongs in), T096 (the bar the blocks sit under).

## Required reading

- `docs/decisions/ADR-017-terminal-command-blocks.md` (draft — decide it first)
- ADR-015, `docs/architecture/terminal.md`, `docs/architecture/security.md`
- `crates/terminal/src/{system.rs,shell.rs,host.rs}` — spawn, env and the
  `SpawnRequest` shape
- Design artboard 8a, the body of the frame

## Allowed scope

- `crates/terminal/**` (a new `integration` module, the scripts it ships,
  `SpawnRequest.args`)
- `crates/gritty-core/src/terminal/**` (`SpawnTerminalRequest.shellIntegration`)
- `packages/protocol` (the field, the version bump)
- `apps/desktop/src-tauri` (nothing beyond the existing adapter, if the field is
  additive)
- `apps/desktop/src/features/terminal/**`, `features/session/persisted-layout.ts`
- `docs/decisions/ADR-017-…`, `docs/architecture/terminal.md`

## Explicit non-goals

- **Writing to the user's dotfiles.** Gritty ships scripts and points a shell at
  them through the environment of the session it starts. It never edits
  `~/.zshrc`, and a session started without integration must behave exactly as
  it does today.
- **Automatic session titles.** That is T047's deferred follow-up and can be
  built on this, but it is a separate decision about what a tab is called.
- **Persisting blocks.** Terminal content is user data and routinely contains
  secrets (ADR-015). Nothing here reaches the session database.
- **A block model in the core.** See ADR-017 — decide it before starting.
- **Shells beyond zsh and bash.** fish, nushell and the rest get no integration
  and no blocks; they must not get a broken prompt either.
- **Windows.** No ConPTY behind this interface yet.

## Required behavior

1. `terminal.spawn` accepts `shellIntegration` (default on), and the core asks
   the host to build the environment for the resolved shell:
   - **zsh** — `ZDOTDIR` pointed at Gritty's directory, with the user's own
     value preserved in `USER_ZDOTDIR`; Gritty's `.zshrc` sources theirs first
     and installs `precmd`/`preexec` hooks after it.
   - **bash** — `--rcfile`, which needs an `args` field on `SpawnRequest` that
     does not exist yet; the script sources `~/.bashrc` then installs
     `PROMPT_COMMAND` and a `DEBUG` trap.
   - **anything else** — no integration, no error, no blocks.
2. The scripts emit OSC 133 `A` (prompt start), `B` (command start), `C`
   (output start) and `D;<exit>` (finished), and nothing else. They are the only
   thing Gritty adds to a shell's startup and must be readable in one sitting.
3. A shell whose integration fails to load still starts and still works. The
   test for this is deleting the script directory between resolve and spawn.
4. The renderer parses the markers, groups the scrollback into blocks, and draws
   each one's accent bar and its right-aligned duration and exit code, from the
   artboard: green for success, red for a non-zero exit with a wash over the
   block, blue while it runs.
5. The failed block's file paths are links: `crates/terminal/src/pty.rs:214`
   opens that document at that line through the ordinary command. Refuse
   anything that does not resolve inside the workspace.
6. A terminal settings toggle turns integration off, and the change applies to
   the *next* session — a running shell is not restarted underneath the user.
7. Blocks cost nothing when there are none: a shell without integration renders
   exactly as it does today.

## Failure behavior

- **powerlevel10k's instant prompt.** It requires being first in `.zshrc` and
  prints a warning when anything writes to stdout before it. This is the named
  risk of the task — T095 already found one real defect on this exact surface.
  The zsh script must load *after* the user's `.zshrc` and must not write before
  it. If it cannot, the integration is off for that machine, loudly.
- A shell that emits `D` with no matching `C` (a `^C` at an empty prompt) closes
  no block and creates no phantom one.
- A marker split across two output batches is still one marker.
- An `exit` mid-command leaves the last block open; it is closed by the session
  ending, marked with the session's own exit, not the command's.
- Output dropped by the 1 MiB bound (`OutputBatch.truncated_bytes`) can remove
  the rows a block spanned. The block reports that it is incomplete rather than
  drawing a bar over rows that are not its own.
- An OSC 133 sequence in a *file* the user `cat`s is untrusted input (hard rule
  13). It can only lie about block boundaries in the user's own terminal — no
  path here may act on it.

## Acceptance criteria

- `pnpm check:ts` in a zsh session draws one block, its duration, and a green ✓.
- A failing `cargo test` draws a red block with `exit 101` and the wash, and the
  path in its output opens the file at the line.
- A shell with integration off is byte-for-byte what ships today.
- `vim`, `htop` and `less` are unaffected — a full-screen program is not a block.
- powerlevel10k starts with no warning and its prompt is intact.
- Deleting the integration directory before a spawn still gets a working shell.

## Tests required

- Rust: environment building for zsh, bash and an unknown shell; `USER_ZDOTDIR`
  preserved; integration off producing exactly today's environment.
- Rust: `SpawnRequest.args` reaches the child (extend the existing PTY suite).
- Vitest: the block state machine over a synthetic marker stream — nested,
  unmatched `D`, a marker split across batches, a block truncated by the output
  bound.
- Vitest: the path linkifier, including a path that escapes the workspace.
- Manual: zsh with powerlevel10k, bash, and a shell with no integration.

## Commands to run

`pnpm check`, `pnpm dev`.

## Completion report

_Not started._
