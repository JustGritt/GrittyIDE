# Task T102-R — `gritty-git-write`: the subprocess adapter, the probe, and the failure map

- **Phase:** 2
- **Status:** review
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

**Summary.** `crates/git-write` (`gritty-git-write`): the argv, the probe, the
process group, and the failure map. No new third-party dependency — `nix` and
`tokio` were already workspace dependencies, and `members = ["crates/*"]` is a
glob so the member needed no manifest edit.

**Files changed.** `crates/git-write/{Cargo.toml,src/{lib,argv,error,probe,run}.rs,tests/writes.rs}`,
root `Cargo.toml` (one `[workspace.dependencies]` line).

**Tests actually executed.** `cargo test -p gritty-git-write` — 19 unit + 10
integration, all passing. `cargo clippy -p gritty-git-write --all-targets -D
warnings` clean.

The integration tests assert ADR-029's claims rather than restating them: a
clean filter runs on stage (the corruption the ADR exists to prevent), a
passing hook runs, a refusing hook stops the commit with HEAD unmoved and its
stderr reaching the caller, `--no-verify` skips it, a message with a quote, a
`$(…)` and a `#` line survives byte for byte, a discard is recoverable from the
stash, and a held `index.lock` is named rather than retried.

**The force-kill test earned its place by failing** (rule 12). It exposed two
bugs that reading the code had not:

1. `terminate()` returned as soon as the *child* was reaped — copied from
   `ClaudeProcess`, where the analogy does not hold. For an agent run the child
   is the work; for a commit, `git` exits on SIGTERM while a hook that traps it
   keeps running, so SIGKILL never fired and the hook survived. It now waits for
   the process *group*, reaping the child as it goes so a zombie cannot hold the
   group open.
2. `wait()` read stderr to EOF, which hangs whenever a hook backgrounds a
   process that inherits the pipe. The suite took 303s; it now takes 2s.

**Architectural decisions.**

- *Unstage is `git reset`, not `git restore --staged`* — the latter fails with
  "could not resolve HEAD" on an unborn branch. Verified in a scratch repository
  before choosing.
- *Pathspecs are `:(literal)`-prefixed.* `--` separates revisions from paths but
  does not turn pathspec *magic* off, so a file named `:(icase)notes.md` would
  otherwise be read as a pattern.
- *`--cleanup=verbatim`.* Git's default strips lines beginning with `#`, which
  silently eats a message citing an issue as `#42`.
- *The environment is scrubbed* of `GIT_DIR`, `GIT_WORK_TREE` and five
  siblings: Gritty may itself be launched from inside a repository.
- *No serde or ts-rs.* The wire shapes belong to `gritty-core::git`, not here.

**Known limitations.**

- **§6's four named outcomes are two.** Only structural facts are classified: a
  held lock is a file that exists, everything else is a refusal carrying git's
  own stderr. Distinguishing a hook refusal from an unmerged path from "nothing
  to commit" means pattern-matching git's localised, version-dependent prose —
  the parser ADR-023 declined to maintain. `error.rs` documents the choice.
- **`index_is_locked` looks in `<root>/.git`**, which is a *file* for a linked
  worktree or submodule; those layouts report a plain refusal instead. Revisit
  with T400, which makes that layout ordinary.
- **The non-unix path is untested.** `killpg` is unix-only; elsewhere only the
  direct child is reached and a forking hook would survive. Named in the module
  header rather than claimed (rule 11).

**Follow-up tasks.** None blocking. T400 should revisit the git-dir resolution.

**Remaining risks.** The `git` floor is 2.13 (for `stash push -- <pathspec>`);
older binaries disable the write half with that reason rather than failing.
