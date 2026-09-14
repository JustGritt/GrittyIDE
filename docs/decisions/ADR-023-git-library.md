# ADR-023: Git is read in-process through `gix`, with no `git` process and no configuration that can run one

- **Status:** Accepted (implemented by T100), amended by ADR-029
- **Date:** 2026-09-06
- **Accepted:** 2026-09-14 by Alexis, **as amended by ADR-029**, which was
  accepted first (2026-09-09) and replaces decision 4. Read the title and
  decision 1's "no `git` subprocess" as applying to *reads*: status, diff, log
  and show go through `gix` in `crates/git`, while writes run the `git` binary
  inside `crates/git-write` under ADR-029's rules. Decision 2 holds as written
  and is asserted by `no_program_named_by_the_repository_can_run`
  (`crates/git/src/open.rs:245`); decision 3 holds, since `gritty-core` names
  `gix` only in comments.

## Context

`docs/Gritty IDE Project Plan.md:797` leaves the Git backend open: shell out to
`git`, or use an in-process library (`git2`/libgit2, or `gix`). T100 needs
`git.status` and `git.diff` and recommends `gix`, deferring the choice to an
ADR "before implementation" (`docs/tasks/completed/T100-git-baseline.md:100-116`).
ADR-007 already decided that Git is how Gritty isolates work, so this is the
floor everything from T400 up stands on, and replacing it later is expensive.

Three candidates, against three constraints this repository has already written
down.

**Hard rule 5 — raw `git diff` is never the ChangeSet domain model.** Shelling
out means parsing porcelain, and the rule generalizes to status: `--porcelain=v2`
is a stable format, but a stable format is still a text format that has to be
reparsed into types, with a parser nobody here maintains and failure modes that
show up as a wrong badge rather than an error. Both libraries return structured
data. This eliminates the shell-out on its own.

**Hard rule 13 — repository content is untrusted input.** Sharpest here,
because Git is the one subsystem whose *inputs are the attacker's file format*:
pack files, tree objects, refs, and a `.git/config` that came with the clone.
`gix` is pure Rust and memory-safe end to end. `git2` is a binding around
libgit2, a mature and much-attacked C library — safer than it sounds through a
decade of hardening, but still a C parser with a larger memory-safety surface
for exactly the data this rule is about.

**Ambient authority is the concern nobody names.** Shelling out to `git`
inherits the user's whole configuration as authority: aliases, `core.pager`,
credential helpers, `include.path` directives — a repository can influence
several of these. It is the worst of the three on a constraint that matters more
than performance. But the libraries are not automatically clean either: both
`gix` and `git2` will honour configuration that names an external program —
clean/smudge filters, `diff.<driver>.textconv` — if asked to. A library is not
"no process" by construction; it is "no process if configured that way", and
that has to be a test rather than an assumption.

**Cancellation.** Hard rule 10. `gix`'s status platform takes an interruption
flag alongside its progress reporting and thread limit
(`Repository::index_worktree_status`), which composes directly with
`OperationContext`'s token — the same shape T100 §6 needs for a status that
outlives its budget. libgit2's status and diff are largely synchronous walks
with no general cancellation hook, so the equivalent would be checking the token
in a callback where one happens to exist, and not at all where one does not. A
spawned process cancels by dying, which is the problem ADR-015 already solved
once and does not want to solve twice.

**Cost of each.** `gix` 0.87.1, MSRV 1.85, `MIT OR Apache-2.0`, pure Rust, one
`[workspace.dependencies]` entry (its own sub-crates come as its dependencies,
not as separate entries here). `git2` 0.21.0, `MIT OR Apache-2.0`, pulls
`libgit2-sys` and builds C — a C toolchain in every build environment, including
the Linux CI that does not exist yet (`CI-LINUX-DARK`) and any future
cross-compile. `gix` is also the larger dependency tree by crate count, which is
the fair counterweight.

## Decision

**1. `gix`, in-process. No `git` subprocess, and no `git2`.** One exact-pinned
external `[workspace.dependencies]` entry with its justification, consumed only
inside the new `crates/git` (`gritty-git`) adapter. It overlaps nothing in the
tree.

**2. Nothing repository-influenced may cause a program to run.** The adapter
opens repositories with external filters, `textconv` and any other
config-named executable disabled, and **a test asserts it** against a fixture
whose `.git/config` and `.gitattributes` ask for one. Rule 12 applies: this is
not claimed from code shape. This is the requirement that makes "in-process"
mean what it says, and it binds whichever library is used.

**3. `gritty-git` is the boundary, not an abstraction over backends.** No trait
with one implementation, no feature-flagged second backend — hard rule 9. What
makes this reversible is the same adapter/domain split ADR-005 asked for and
that `gritty-terminal`/`terminal`, `gritty-agents`/`agent_session` and
`gritty-agent-fake`/`agent_run` already use three times over: `gix` types never
cross into `gritty_core::git`, so replacing the library is a rewrite of one
crate, with the domain types and their tests unchanged.

**4. Read-only.** `git.status` and `git.diff` only. Nothing writes, stages,
commits, fetches or authenticates, so no credential helper, transport or
network surface is reachable in the first place. T400 will want writes; that is
a later decision and a later risk review.

## Consequences

- `gix` is pre-1.0 at 0.87.1, and its release history is a long run of breaking
  minors. Upgrades are deliberate tasks with a compile error to guide them, per
  the dependency policy — not automatic. This is the real cost of the choice
  and it is paid on a schedule we pick.
- `gix`'s status is younger than libgit2's, which is two decades old. The
  edges most likely to differ are submodules, sparse checkout, and conflicted
  entries. T100's fixtures must cover a repository in each interesting state
  (merging, rebasing, detached, conflicted, no upstream), and any behaviour
  Gritty cannot reproduce is reported as unknown rather than guessed —
  `RepositoryStatus` already carries absent-able fields for this.
- No C toolchain is added to the build, so Linux CI and any future
  cross-compile inherit no new requirement. This is worth more than it looks
  while the platform matrix has `–` rows in it.
- Nothing in Gritty can be affected by the user's `git` aliases, pager or
  credential configuration, because no `git` runs. The corollary is that Gritty
  will occasionally disagree with what the user's own `git status` prints, when
  their configuration changes what that command does. Disagreeing visibly is the
  right failure; inheriting their configuration silently is not.
- If `gix` turns out to be insufficient, the replacement is `git2` behind the
  same adapter, and decision 3 is what keeps that a contained change. It would
  supersede this ADR rather than extend it.

## Revisit when

`gix` cannot produce a status or diff Gritty needs and the gap is not
closing upstream; or Gritty needs to *write* through Git (T400's worktrees,
branch creation, commits), which brings transports, credentials and a risk
surface this ADR deliberately does not cover; or `gix` reaches 1.0 and the
upgrade cost stops being a standing consequence.
