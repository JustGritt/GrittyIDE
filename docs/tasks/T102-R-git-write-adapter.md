# Task T102-R — `gritty-git-write`: the subprocess adapter, the probe, and the failure map

- **Phase:** 2
- **Status:** blocked (behind T102-C; ADR-029 is decided)
- **Depends on:** ADR-029, T102-C

## Objective

The crate behind the four writes: a small, auditable adapter that invokes the
user's `git` binary, maps its exit status onto the error envelope, probes once
for the binary's presence, and runs a commit as a cancellable `Operation` whose
cancellation kills the hook's process group.

## Motivation

ADR-029's argument in one line: `gritty-git` opens repositories with clean
filters disabled so nothing a repository names can execute, which is
conservative for a read and destructive for a write — staging with filters off
puts un-cleaned content in the index. Plus signing and hooks, which an
in-process commit cannot do at all.

## Required reading

- `docs/decisions/ADR-029-git-write-backend.md` — all six decisions
- `crates/git/src/lib.rs:17-35` — the invariants this crate must *not* break
- `crates/git/src/open.rs` — why `gritty-git` opens repositories the way it does
- `crates/agent-claude/src/process.rs:135-145`, `:310-326` — the process-group
  spawn and the SIGTERM → 500ms → SIGKILL ladder to reuse
- `crates/gritty-core/src/git/mod.rs` — the budget hand-over to mirror
- `crates/terminal/src/system.rs:692-735` — the other kill ladder, for reference

## Allowed scope

- `crates/git-write/**` (new)
- `crates/gritty-core/src/git/**`
- `apps/desktop/src-tauri/**`
- root `Cargo.toml` (one workspace member; no new external dependency expected)

## Explicit non-goals

- **Nothing goes in `crates/git`.** Its header promises "nothing writes" and "no
  external program runs", and `open.rs`'s
  `no_program_named_by_the_repository_can_run` enforces it. ADR-029 decision 2
  keeps the two crates apart on purpose; they must not depend on each other.
- No network subcommand: no `fetch`, `pull`, `push`, `clone`, `remote`, `ls-remote`.
- No `checkout`, `branch`, `switch`, `worktree`, `rebase`, `merge`, `cherry-pick`.
- No parsing of porcelain into a domain type — the reads are `gix`'s and stay so.
  A write reads an exit code, not an output format.

## Required behavior

1. **Every argv is explicit and closed.** Pathspecs always after `--`; never
   `git add -A`, never `git commit -a`. A test asserts that a path which looks
   like a flag (`--upload-pack=…`) lands as an argument.
2. **No pager, editor or prompt.** `GIT_PAGER=cat`, `GIT_EDITOR=true`,
   `GIT_TERMINAL_PROMPT=0`. The commit message is delivered with `-F` from a
   file, never interpolated into a command line — a test asserts a message
   containing a newline, a quote and a `$(…)` survives byte for byte.
3. **The probe.** `git --version` once per workspace, cached; absence or a
   version below the floor disables the write half with a stated reason.
4. **The commit is an `Operation`.** `registry.start` + `ctx.cancelled()` +
   an `agent-claude`-shaped `terminate()`. Note in the code that **no operation
   in the tree owns a child process today** — this is the first, and it composes
   two proven pieces rather than inheriting a precedent.
5. **Discard is recoverable** (ADR-029, and the priority order's second place):
   tracked modifications go through `git stash push -- <paths>` before the
   restore, and the response says so, so the UI can tell the user where their
   work went. Untracked files are **not** stashed — they go through
   `files.delete`'s existing `Trash` disposition.
6. **The failure map.** `index.lock` contention, a hook's non-zero exit, an
   unmerged path, "nothing to commit" — each a distinct, named outcome. A hook's
   stderr reaches the caller; nothing is swallowed and nothing is logged that
   came out of the repository (rule 13).

## Failure behavior

- `git` missing → the probe's verdict, surfaced once, not four failing commands.
- A hook that exits non-zero → the commit did not happen, and the caller is told
  what the hook said. Never reported as success.
- Cancellation mid-hook → the process **group** dies, not just the child, and
  nothing partial is claimed. Rule 12: this needs a force-kill test, not an
  assertion from code shape.
- A repository that is not a repository → the `NotARepository` value, as the
  reads already do.
- A path outside the root → refused before any process is spawned.

## Acceptance criteria

- The four writes work against a real repository, with hooks and signing
  honoured — verified against a fixture repository that installs a `pre-commit`
  hook and one that refuses.
- Cancelling a commit whose hook sleeps kills the hook.
- `pnpm check` green.

## Tests required

- Argv construction, as pure functions over the request — the cheapest place to
  prove decisions 1 and 2.
- A fixture repository with a `pre-commit` hook that (a) passes, (b) refuses,
  (c) sleeps long enough to be cancelled.
- A fixture with a `filter.*.clean` driver, asserting the staged blob is the
  *cleaned* one — the corruption ADR-029 exists to prevent, proven rather than
  claimed.
- Force-kill: cancel mid-hook and assert the process group is gone.

## Commands to run

```
cargo test -p gritty-git-write
cargo test -p gritty-core --lib git
cargo test -p gritty-desktop --test commands
pnpm check
```

## Completion report

_Blocked; not started._
