# ADR-029: Git writes run through the `git` binary, while reads stay in-process on `gix`

- **Status:** Proposed
- **Date:** 2026-09-08
- **Amends:** ADR-023 decision 4 ("Read-only"), which named this decision in its
  own *Revisit when* and deliberately did not cover it

## Context

ADR-023 chose `gix` in-process for `git.status` and `git.diff`, and closed with
decision 4: *"Read-only. … Nothing writes, stages, commits, fetches or
authenticates. … T400 will want writes; that is a later decision and a later
risk review."* Its *Revisit when* names the trigger precisely: *"or Gritty needs
to **write** through Git."*

That point has arrived, and not only for T400. The Changes panel (`T100-U`) can
show that a file is staged and cannot stage one; `git-reducer.ts:22` records the
reason in as many words — *"There is no stage, commit, checkout or fetch on the
wire to call."* Source control as a user-facing feature needs `git.stage`,
`git.unstage`, `git.discard` and `git.commit`.

The obvious answer is "extend `gritty-git`; it already has the repository open."
That answer is wrong, and the reasons are specific to decisions this repository
has already made.

### ADR-023's own safety configuration is unsafe for a write

ADR-023 decision 2 requires that *"nothing repository-influenced may cause a
program to run"*, and `crates/git/src/open.rs` implements it by opening every
repository with clean/smudge filters and `diff.<driver>.textconv` disabled —
with `no_program_named_by_the_repository_can_run` asserting it.

For a read that is conservative and correct: a filter that does not run means
raw bytes are shown instead of a rendering, and the failure is visible and
harmless.

For a **write** the same configuration is destructive. Staging a file means
converting worktree content into an index blob, and the *clean* filter is that
conversion. With filters disabled, `git add` through `gix` would write
**un-cleaned** content: in a Git LFS repository it stores the whole binary where
a pointer belongs; with any `filter.*.clean` driver it stores content the
repository's own rules say must never be committed. Nothing reports an error.
The repository is quietly corrupted, and the corruption is discovered by someone
else, later, on a clone.

Re-enabling filters for writes would mean reversing decision 2 for the one code
path where an attacker-influenced config gets to name an executable — trading a
silent-corruption bug for the exact ambient-authority hole that decision was
written to close.

### Two more divergences, both silent

**Signing.** On the machine this was written on, `commit.gpgsign` is `true` with
a signing key set. `gix` does not sign commits. Every commit Gritty made in this
repository would be unsigned while every commit beside it is signed — a
divergence nobody sees until a branch protection rule or a reviewer catches it.

**Hooks.** `pre-commit` and `commit-msg` are how a project enforces its own gate.
An in-process commit runs neither, so a Gritty commit is one the project's gate
never saw — in a repository whose `AGENTS.md` makes `pnpm check` the definition
of "checks pass", that is precisely the wrong thing to bypass by accident.

### The argument that banned the subprocess for reads does not transfer

ADR-023 eliminated shelling out on hard rule 5: *raw `git diff` is never the
ChangeSet domain model*, and porcelain is a text format needing a parser nobody
here maintains. That reasoning is about **reading**. A write has no output format
to parse — explicit pathspecs after `--`, and an exit code. There is no porcelain
in `git add -- src/a.ts`.

And the ambient-authority argument **inverts**. For a read, inheriting the user's
configuration is a liability: aliases, pager, credential helpers, `include.path`.
For a commit it is the definition of correctness. A commit should carry the
user's identity, their signing key, their `commit.template`, and should be
stopped by their hooks. A Gritty commit ought to be indistinguishable from one
the user made by hand — and the only way to guarantee that is to let their `git`
make it.

## Decision

**1. Writes shell out to `git`; reads stay on `gix`.** `git.stage`,
`git.unstage`, `git.discard` and `git.commit` invoke the user's `git` binary.
`git.status`, `git.diff`, `git.log` and `git.show` are unchanged and remain
in-process. ADR-023 decisions 1–3 stand for every read.

**2. The subprocess lives in its own crate, `crates/git-write`
(`gritty-git-write`).** Not in `crates/git`, whose header promises "nothing
writes" and "no external program runs" and whose `open.rs` test enforces it.
Keeping the two apart means that invariant stays literally true and testable,
and it puts the trust boundary in the dependency graph where a reviewer can see
it. `gritty-git` and `gritty-git-write` never depend on each other;
`gritty-core::git` composes both.

**3. Every invocation is explicit, and none of it is repository-influenced
argv.** Concretely, and each of these is a test:

- **Pathspecs are always explicit and always after `--`.** Never `git add -A`,
  never `git commit -a`. Gritty ships a terminal, so a user's own `git` may be
  running concurrently; a wildcard write would sweep up whatever another process
  changed a moment ago.
- **`-c` overrides carry no repository input**, and no path from a repository
  reaches an option position — a file named `--upload-pack=…` is an argument,
  never a flag, which is what `--` guarantees.
- **No pager, no editor, no prompt.** `GIT_PAGER=cat`, `GIT_EDITOR=true`,
  `GIT_TERMINAL_PROMPT=0`, and the commit message is delivered by file
  (`-F`), never interpolated into a command line.
- **Nothing in this crate reaches the network.** No `fetch`, `pull`, `push`,
  `clone` or `remote`; credentials and transports stay out of reach, exactly as
  ADR-023 arranged for reads.

**4. Hooks run, deliberately, and that is disclosed.** A write invokes the
repository's hooks because that is what makes a Gritty commit a real commit.
This is repository-authored code executing — hard rule 13's subject — so it is
stated in the UI rather than hidden, `--no-verify` exists as a visible choice,
and it is never the default. The honest framing: Gritty running the hooks is no
more authority than the terminal Gritty already ships, and *not* running them is
the more surprising behaviour.

**5. `git` is probed once per workspace, and its absence disables the write half
with a stated reason.** A missing or too-old binary means the staging and commit
affordances are shown disabled with the reason attached — never a button that
silently does nothing, and never a read that breaks. The read half does not
depend on the probe.

**6. `git.commit` is an `Operation`.** A `pre-commit` hook can run a test suite,
so a commit can outlive any budget; hard rule 10 makes it a cancellable
operation, and cancelling it kills the hook's process group the way
`ClaudeProcess::terminate` already does. The three other writes are fast and
answer directly, with the same budget-then-operation escape the reads use.

## Consequences

- **A shipped Gritty now requires a `git` binary for the write half.** This is
  new in production, though not in the repository — `crates/git/src/fixtures.rs`
  already spawns `git` to build test repositories, behind a test-only feature
  compiled out of release builds. On macOS `git` implies the Xcode command line
  tools. Decision 5 is what keeps a missing binary a degraded feature instead of
  a broken one.
- **Gritty inherits the user's Git configuration on writes and not on reads.**
  That asymmetry is deliberate and worth stating plainly, because it is the one
  surprising thing here: a status Gritty shows may differ from the user's own
  `git status` under an exotic config, while a commit Gritty makes is exactly
  theirs. Both directions are the safe one for their side.
- **Two Git mechanisms now exist in the tree.** The cost is real. What bounds it
  is decision 2: the split is by *operation*, not by a trait with two
  implementations, so there is no abstraction pretending they are the same thing
  — which is hard rule 9's concern, and the same shape ADR-023 decision 3 chose.
- **The write surface is auditable in one place.** Every argv Gritty can
  construct lives in one small crate with no network code, which is a better
  review surface than write paths spread through a large in-process library.
- **`gix` remains free to grow into this.** If its write paths later gain filter,
  signing and hook support, decision 1 is the thing to revisit, and the domain
  types and their tests do not move.

## Alternatives rejected

- **`gix` for writes.** Rejected on filters (silent index corruption), signing
  and hooks. Any of the three alone is a correctness bug; together they mean a
  Gritty commit is not the commit the project asked for.
- **`git2`/libgit2 for writes.** Mature index and commit support, but it still
  does not run hooks, adds a C toolchain to every build environment including a
  Linux CI that does not yet exist, and puts a second Git library in the tree to
  solve a problem the user's own `git` already solves.
- **`gix` for the index and `git` for the commit.** Splits one feature across two
  mechanisms on a line users cannot see, and the filter problem lives on the
  `gix` side of that line — which is the half that matters.

## Revisit when

`gix` can apply clean filters, produce signed commits and run hooks — at which
point the subprocess buys nothing and decision 1 should be reversed; or Gritty
needs a *networked* write (fetch, pull, push), which brings credentials and
transports and is a further ADR this one deliberately does not cover; or the
`git` probe proves unreliable enough in the field that the runtime dependency
costs more than the correctness it buys.
