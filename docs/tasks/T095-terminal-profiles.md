# Task T095 — Terminal shell profiles and rooting

- **Phase:** 0 / 2
- **Status:** done
- **Depends on:** T090

## Objective

Let the user choose which shell a terminal starts, the way VS Code's terminal
profile dropdown does, instead of always inheriting `$SHELL`. Report the shells
that actually exist on the machine rather than a hardcoded list, and remember
the choice across restarts.

## Motivation

T090 always starts the user's login shell. That is the right default, but it is
the *only* option: a user whose login shell is heavily configured (a slow
prompt, an interactive-startup surprise) has no way to reach a plain `bash` or
`sh` from inside Gritty, and no way to try another shell without editing their
account. `terminal.spawn` already accepts an absolute `shell`; nothing exposed
it.

## Dependencies

T090 (session registry, `PtyHost`, `terminal.spawn`'s `shell` argument), T060
(the persisted layout blob, for remembering the choice).

## Required reading

- `docs/tasks/T090-terminal-pty.md` (its "profiles UI" non-goal and follow-ups)
- Addendum §14 environment/processes; ADR-015; `docs/architecture/terminal.md`

## Allowed scope

- `crates/terminal/src/shell.rs` (discovery), `host.rs` (a `PtyHost` method),
  `system.rs`, `testing.rs`
- `crates/gritty-core/src/terminal/**` (`TerminalProfile`, `terminal.profiles`)
- `packages/protocol` (command, type, client method, version bump)
- `apps/desktop/src-tauri` (one adapter)
- `apps/desktop/src/features/terminal/**`, `features/session/persisted-layout.ts`
- the terminal's default font stack (the glyph defect below)
- a terminal settings popover (added mid-task, see "Scope change" below)

## Scope changes

Both recorded rather than hidden; both were asked for during review.

**1. A terminal no longer needs a workspace.** The reviewer asked to be able to
open a terminal without opening a folder first, defaulting to the home
directory. `workspaceId` and `cwd` are now optional on `SpawnTerminalRequest`
and `Terminal`; a `cwd` without a `workspaceId` is `InvalidInput`. ADR-010 is
not weakened — the renderer names a workspace or names nothing, and the core
resolves the directory — but making two required fields optional is a breaking
shape change, so `PROTOCOL_VERSION` goes 1.6.0 → **2.0.0**. ADR-015 carries the
amendment. Phase 0's freeze has not happened, which is the moment for this.

**2. Discovery lists only what the system registers.** The task first shipped
discovery that also scanned
`/bin`, `/usr/bin`, `/usr/local/bin` and `/opt/homebrew/bin` for a fixed list of
shell *names*. On review the reviewer doubted the resulting list, and they were
right: `/usr/local/bin/nu` on that machine is a symlink to `@antfu/ni`'s Node
script, not nushell. They asked for two things — list only what is genuinely
available, and let the user configure the rest. Both are now required behavior
below, and the "no settings panel" non-goal is withdrawn.

## Explicit non-goals

- Named profiles (a label, arguments, an environment, an icon) — that needs a
  persisted profile document, not a list of paths.
- Shell arguments, per-profile cwd, per-profile environment.
- Windows shell discovery (no ConPTY support behind this interface yet).
- Terminal theme/colours; only family and size are configurable.

## Required behavior

1. `terminal.profiles` returns the shells the picker should offer, each with an
   absolute `path`, a short `name`, `isDefault` for the one `terminal.spawn`
   picks when no shell is named, `isCustom`, and `available`. The default is
   first.
2. Discovery reads **only** `$SHELL` and `/etc/shells` — the operating system's
   own register of login shells. It never infers that a program is a shell from
   its file name. It keeps only executable regular files and deduplicates by
   canonical path, so `/bin/bash` and `/usr/bin/bash` on a merged-`/usr` system
   are one entry.
3. Two shells sharing a file name are told apart in `name` by their directory.
4. `terminal.profiles` accepts `custom`: absolute paths the user configured.
   Each is checked like a discovered shell, but an unusable one is reported
   with `available: false` and a reason rather than dropped. The list is
   bounded.
5. The terminal bar has a profile picker over the *available* profiles; "New
   terminal" starts the selected one. The choice persists with the layout.
6. A terminal settings popover adds and removes custom shells (showing why a
   broken one does not work) and sets the font family and size. Everything it
   owns persists with the layout and is restored.
7. A profile that has been removed from the machine since it was persisted, or
   that is listed but unavailable, falls back to the default rather than
   failing to spawn.
8. Discovery never spawns anything and never blocks the renderer.
9. A font change applies to a running terminal without losing its scrollback,
   and the PTY is told the new grid.
10. `terminal.spawn` without a `workspaceId` starts the shell in the user's home
    directory; "New terminal" is never disabled. A `cwd` without a
    `workspaceId` is `InvalidInput`. `Terminal.workspaceId` and `Terminal.cwd`
    are absent for such a session.
11. Closing or replacing a workspace does not make the panel forget running
    sessions, because the core does not kill them.

## Failure behavior

- A machine with no readable `/etc/shells` still reports `$SHELL`; a machine
  where even that fails reports nothing, and the picker is hidden rather than
  empty — "New terminal" still starts the core's resolved default.
- A custom path that is relative, missing, not a regular file or not executable
  is listed as unavailable with the reason, and is never offered to spawn.
- Choosing a shell that is deleted between discovery and spawn is the existing
  `terminal.spawn` error (`NotFound`/`PermissionDenied`), shown in the panel's
  error envelope. It is never silently replaced by another shell.
- A layout blob carrying a relative or duplicate custom path is sanitized on
  read; the blob is a file on disk and anything could have edited it.
- An account with no usable home (`$HOME` unset or not a directory, and no
  password-database entry) fails the spawn with `InvalidInput`/`Io` rather than
  starting a shell somewhere arbitrary.

## Acceptance criteria

- The picker lists only shells the system registers, defaulted to the login
  shell. Nothing appears that cannot be started.
- Selecting `bash` and clicking "New terminal" starts bash; the tab is labelled
  with the shell and its title attribute reports the path and directory.
- Adding a path in the settings puts it in the picker without a restart; a bad
  path is listed with the reason and never offered.
- Restarting the app restores the selected profile, the custom shells and the
  font.
- Powerline/Nerd Font glyphs render when such a font is installed; changing the
  font family or size reflows a running terminal without clearing it.
- With no folder open, "New terminal" starts a shell in `~`; opening a folder
  afterwards leaves it running and visible.

## Tests required

- Rust: discovery order, deduplication, `$SHELL` marked default, non-executable
  and missing candidates skipped, an empty environment, and a regression test
  that an executable merely *named* like a shell is not offered.
- Rust: `check_shell` rejects relative, missing, non-file and non-executable
  paths.
- Rust: `terminal.profiles` through the manager against a fake host — labelling,
  disambiguation, a custom shell that runs, one that is gone, one the system
  already registers, and the bound.
- Vitest: the picker's fallback when the persisted path is gone or unavailable;
  adding a custom shell; the layout blob's sanitizing and its pre-T095 form.
- Rust: home resolution from `$HOME`, the fallback when it is relative or
  broken, and an environment with no `HOME` at all.
- Rust: spawn with no workspace starts somewhere absolute that is not the open
  workspace; a `cwd` without a workspace is refused and spawns nothing; a
  workspace-less session is listed like any other.
- Tauri: spawn with no workspace open, and the `cwd`-without-workspace refusal.
- Vitest: the reducer keeps sessions across a workspace change and close, and
  tracks a session with no workspace.
- Manual: pick each listed shell and confirm it starts; add a real shell by
  path; change the font on a running terminal; open a terminal with no folder
  open, then open a folder and confirm it survives.

## Commands to run

`pnpm check`, `pnpm dev`.

## Completion report

### Summary

Three things, all in the terminal: a terminal no longer needs a workspace,
discovery lists only shells the system registers, and the user can configure
the rest. `terminal.spawn` without a `workspaceId` starts in the home
directory; `terminal.profiles` reports the pickable shells; a settings popover
adds shells and sets the font. Protocol **2.0.0** (breaking, see below).
ADR-015 carries the amendment.

Two defects fixed on the way, neither of them the reported symptom:

- **The terminal was never broken.** The first report was "it opens
  blank/broken"; the screenshot showed powerlevel10k rendering correctly except
  that every Private Use Area glyph was an empty box. A *font*, not a shell.
- **Closing a workspace hid running shells.** The panel forgot its sessions
  while the core kept them running — invisible and unkillable until app exit.
  Found while making sessions outlive a workspace; the UI now keeps every
  session the core has.

### Files changed

- `crates/terminal/src/shell.rs`: `ShellProfile`, `discover_shells` (`$SHELL`
  and `/etc/shells` only), `system_shells`, `check_shell`, `home_directory`
  (`$HOME`, then the password database), eleven tests.
- `crates/terminal/src/error.rs`: `InvalidInput` counts as caller fault.
- `crates/terminal/src/host.rs`, `system.rs`, `testing.rs`: `PtyHost::shells`
  and `FakePtyHost::set_shells`.
- `Cargo.toml`: `nix` gains the `user` feature (for the passwd home).
- `crates/gritty-core/src/terminal/wire.rs`: `ListProfilesRequest`,
  `TerminalProfile`; `Terminal.workspace_id`/`cwd` and
  `SpawnTerminalRequest.workspace_id` become optional.
  `mod.rs`: two rootings in `spawn`, `profiles`, `MAX_CUSTOM_SHELLS`, ten new
  tests. `protocol/export.rs`, `PROTOCOL_VERSION` 1.5.0 → 2.0.0.
- `apps/desktop/src-tauri`: `terminal_profiles` adapter, registration, three
  tests.
- `packages/protocol`: `commands.ts`, `client.ts`, `types.ts`, four generated
  files; the client tests now derive their fake protocol version from
  `PROTOCOL_VERSION` instead of a frozen literal, so a MAJOR bump no longer
  breaks every unrelated test.
- `apps/desktop/src/features/terminal/`: `terminal-profiles.ts` and
  `terminal-settings.tsx` (new); `use-terminals.ts` (profiles refetched on
  change, `spawn` takes a nullable workspace, sessions adopted with or without
  one); `terminal-panel.tsx` (picker, settings, never-disabled button, tabs
  labelled by shell); `terminal-view.tsx` (font stack, live font changes);
  `terminal-reducer.ts` (sessions survive a workspace change).
- `apps/desktop/src/features/session/persisted-layout.ts`: a `TerminalLayout`
  section, `LAYOUT_VERSION` 1 → 3, sanitizing.
- `App.tsx`, `app.css`, `apps/desktop/src/core/health.test.ts`.
- Docs: ADR-015 amendment, `docs/architecture/terminal.md`, `docs/tasks/README.md`.

### Tests actually executed

| Command | Result |
|---|---|
| `cargo test --workspace` | 279 pass, 0 fail (27 new) |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo fmt --all --check` | clean |
| `pnpm exec vitest run` | 248 pass across 41 files (18 new) |
| `pnpm -r typecheck`, `biome check` | clean, 164 files |
| `pnpm check` | every gate green **except** the generated-types guard, which compares `git status` and so needs the five generated files committed |

Not executed: `pnpm dev`. The remaining acceptance criteria are visual and the
agent cannot see or click the WebView.

### Architectural decisions

- **The renderer names a workspace or names nothing.** That is the whole
  argument for why optional `workspaceId` does not weaken ADR-010: the core
  still owns the mapping from "no workspace" to a directory. Accepting an
  absolute `cwd` from the WebView would have been the easy version and the
  wrong one.
- **A breaking bump, taken now.** Making two required fields optional is a
  MAJOR change. Phase 0's freeze has not happened (ADR-001 is open), so this is
  the cheapest moment it will ever be.
- **Sessions are not partitioned by workspace.** Killing a workspace's shells
  when it closes was considered and rejected: a long-running build is user
  work, and destroying it because someone opened another folder is the wrong
  default. Keeping them listed also fixes the invisible-shell leak. If they
  should be killed, that is a deliberate decision with its own confirmation UX.
- **Only authoritative shell sources.** `/etc/shells` is the register `chsh`
  validates against. A name-based scan traded a false negative (an unregistered
  Homebrew `fish`) for a false positive (an npm helper called `nu`), and a
  picker that offers something unusable is worse than one that misses something
  addable. The false negative is now the settings' job.
- **Unavailable is reported, not hidden.** A custom entry comes back with
  `available: false` and a reason; the picker still refuses to offer it.
- **Discovery is a `PtyHost` method; validation and home are free functions.**
  The host is the OS seam, so a machine survey belongs on it and can be faked.
  Checking one path, or finding `$HOME`, is a filesystem fact — the core calls
  those directly and the tests use real temporary directories.
- **The default profile sends no `shell`,** so "default" keeps meaning default
  rather than a path frozen when the list was fetched.
- **Font changes are applied in place** — options set, grid refit, PTY told —
  because recreating the terminal would throw away the scrollback.
- **Test fixtures derive the protocol version.** Six test files pinned a
  literal that a MAJOR bump invalidated. They now read `PROTOCOL_VERSION`, so
  the compatibility tests keep testing compatibility and nothing else breaks.

### Dependencies added

None. `nix` gains the `user` feature; it was already a unix-only dependency.

### Known limitations

- **Manual checks are owed.** Picking each shell, adding one by path, the font
  change, and the no-workspace flow. All visual.
- **A shell installed but unregistered needs a manual path.** The deliberate
  trade; `brew install fish` prints the `/etc/shells` instruction.
- **Closing a workspace leaves its shells listed.** Correct today (they are
  running) but the strip will mix workspaces in a long session; there is no
  grouping and no "close all from that workspace".
- **No named profiles** — no per-profile arguments, environment or cwd.
- **Windows is not covered.** `/etc/shells` is a unix path and the passwd
  fallback is unix-only; ConPTY is unsupported anyway.
- **`/bin/csh` and `/bin/tcsh` both appear on macOS.** Separate files with
  identical content; `canonicalize` does not collapse hard links.
- **The font family field is a raw CSS font list.** A typo silently falls back
  rather than being rejected.
- **Discovery and home resolution are synchronous filesystem work** on the
  command runtime. Small, but a hung network mount under an `/etc/shells` entry
  would block that one command.

### Follow-up tasks

- Decide whether closing a workspace should terminate its terminals, and if so
  with what confirmation. Today they keep running and stay listed.
- Group or label sessions by workspace in the tab strip once they can outlive
  one.
- Named terminal profiles (label, program, arguments, environment, cwd).
- Terminal theme/colours in the settings; only family and size are there.
- Validate the font family against `document.fonts.check`.
- `terminal.close` to forget an ended session without killing it (from T090).

### Remaining risks

- **Protocol 2.0.0 is a hard break.** Any core and client that are not upgraded
  together now refuse each other by design. Nothing outside this repository
  consumes the protocol, so the blast radius is a stale `pnpm dev` process.
- The picker's contents depend on the machine, which is why no test asserts a
  real machine's list. On this one: zsh (default), bash, csh, dash, ksh, sh,
  tcsh — the seven `/etc/shells` entries, with the npm helper gone.
- `home_directory` is exercised against the account running the tests. An
  account with no home at all is handled by code but not by a test, because
  making one is not something a test suite should do.
