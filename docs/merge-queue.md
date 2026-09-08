# Merge queue

What is waiting to land on `main`, in what order, and why. Tracked, because the
ordering is shared knowledge — unlike `AGENT-LEASES.md`, which is per-checkout,
and `HANDOFF.md`, which is per-machine.

```
scripts/merge-queue.py        # the live queue, with the seams flagged
```

That script reads GitHub. **This file holds only what GitHub cannot derive**:
which PR has to land before which, and what the second one will have to fix by
hand. Do not copy CI status or mergeability into it — those go stale in
minutes, and a stale status is worse than none.

## Why this exists

Two merges have gone wrong here, and both were predictable.

**PRs #16–#18 were stacked and merged bottom-up.** Each landed *in its parent
branch* instead of on `main`, while `gh pr list` cheerfully said MERGED. GitHub
retargets a stacked PR only when its base branch is **deleted** on merge.

**#20 and #21 were cut from the same base and collided on four seams.** Both
bumped `PROTOCOL_VERSION`, both added a `CoreEvent` variant, both edited
`docs/tasks/README.md`, `docs/decisions/README.md` and `docs/plans/roadmap.md`.
The second to land paid for all of it. That one was survivable because #21 had
deliberately skipped the versions #20 was going to spend; had they both picked
`2.8.0`, git would have merged them **cleanly and wrongly**.

That last failure mode is the reason this file is not just "read the PR list".
A textual conflict is the lucky outcome. Two branches agreeing on a number is
the unlucky one.

## The standing rules

1. **Cut from `main`, not from another branch.** If you must stack, say so
   here, merge top-down, and verify with
   `git rev-list --count origin/main..origin/<branch>` rather than trusting the
   PR state.
2. **Claim a version before you spend it.** `PROTOCOL_VERSION` and
   `SCHEMA_VERSION` are single lines that two branches will both want. Take the
   next free value, write it in the table below, and leave gaps rather than
   reusing a number — a gap costs nothing (`assertCompatible` compares majors
   only) and a collision costs a migration.
3. **Migration numbers are claimed the same way.** `0003_ownership.sql` and
   `0003_agent_links.sql` cannot both exist.
3b. **So are ADR numbers**, added after two branches in one wave both wrote an
   `ADR-024` (filesystem watching, and `target/` retention). This is worse than
   the `PROTOCOL_VERSION` case rather than better: the filenames differ, so git
   merges both **cleanly**, and the repository ends up with two ADRs carrying
   the same number and a `README.md` index that lists whichever row survived.
   Nothing asserts against it. Claim the number here before writing the file.
4. **Never hand-merge `packages/protocol/src/generated/`.** Resolve the Rust,
   then `cargo test -p gritty-core` and take what it writes.
5. **The second PR to land re-runs `pnpm check`** after resolving, not before.
   The generated-types freshness check is the last thing in the gate and the
   first thing a merge invalidates.
6. **A doc index is a seam too.** `docs/tasks/README.md`, `docs/decisions/README.md`
   and `docs/plans/roadmap.md` conflict as readily as code, and a merge that
   drops a row is silent.

## The queue

Hand-maintained: the order and the reason. Run the script for live status.

Smallest first, biggest last: the large PR then resolves **once** against
everything else, instead of each small one resolving against it.

| PR | What | Order | Why / what the loser fixes |
|---|---|---|---|
| [#37](https://github.com/JustGritt/GrittyIDE/pull/37) | `JSDOM-VITEST` | 1 | Adds `docs/tasks/JSDOM-VITEST-component-tests.md`, so the index row gains a link. |
| `PLAN-FANOUT` | Waves, lanes and the ticket splits | 2 | **Regenerates `docs/tasks/README.md` with 23 new rows**, and drops #37's link because the task file is not in this tree yet. Whichever lands second re-runs `scripts/agent-board.py render --index` and commits what it writes — never hand-merge that table. No code, no version, no migration. |

**#27 landed on 2026-09-06**, spending `PROTOCOL_VERSION` 2.11.0,
`SCHEMA_VERSION` 4 and migration `0004_agent_links.sql`; #31–#36 landed
after it, and `t335-agent-permissions` spent 2.12.0. Their rows are deleted
per the rule below. The next free values are **2.13.0**, **5** and
**`0005`**.

**#25 and #26 landed on 2026-09-06**, in the order this table gave them, and
the collision it predicted is the one that happened: #25 and #27 both edited
this file's claim paragraph. The resolution was to keep both halves, which is
why the seams list below survives. Their rows are deleted per the rule below.

**#22, #23, #21 and #24 landed on 2026-09-06** in the order this table gave
them, and the one collision it predicted is the one that happened: #23's
sentence in `docs/tasks/README.md` step 3 met a later branch editing the same
step. Their rows are deleted per the rule below; what they did is in git.

The script found the #23/#21 row itself, from the file lists, before anyone
wrote it down here. That is the intended division of labour: it notices, this
file decides.

### Claimed, so nobody else takes them

| Thing | Value | Branch |
|---|---|---|
| `PROTOCOL_VERSION` | ~~2.16.0~~ | `T300-CMD` — landed on `main` |
| `PROTOCOL_VERSION` | ~~2.17.0~~ | `PROTO-W2-GIT` — landed on `main` (`f261c71`) |
| `PROTOCOL_VERSION` | **2.18.0** | `PROTO-W3` — the `search.query` command and its `CoreEvent` variant |
| `PROTOCOL_VERSION` | **2.19.0** | `T310-B` — `runs.start` / `runs.prompt`, the `CLAUDE_CODE` variant |

**`PROTO-W2-GIT` claims 2.17.0 and nothing else.** It spends no migration and
no `SCHEMA_VERSION`: `git.status` and `git.diff` are reads, and nothing about a
repository is persisted. It adds no `CoreEvent` variant either — a read that
outlives its budget becomes an `Operation`, and the existing `Operation*` events
carry it, which is what `GitStatusResponse::Pending` is for.

Its seams, for whoever lands beside it: `crates/gritty-core/src/protocol/`
(`mod.rs` for the version, `export.rs` for six new root types),
`packages/protocol/src/{client,commands}.ts`, the generated directory, and
`apps/desktop/src-tauri/src/{commands,core,lib}.rs`.

**`PROTO-W2` shrank to almost nothing and should be re-scoped rather than
taken as written.** It was "watcher, git, terminal blocks, diagnostics,
violations, in one PR". T070 and T326 shipped their own wire, this ticket takes
git, and what is left is terminal blocks (behind ADR-017, undecided) and
diagnostics (T110, unbuilt) — two things that will not travel together.

**Only T300-CMD claims anything in the Gate D wave.** T300-CORE is core domain
behind the contract already on `main`, and T300-FAKE is a new crate; neither
adds a command, an event or a type, so neither spends a minor. T300-C already
generated the run types and both `CoreEvent` variants at 2.15.0 — what is left
for 2.16.0 is the four `runs.*` request/response shapes and their client
methods. No migration: nothing about a run is persisted, deliberately, until
ADR-021 says what a real provider's runs should keep.

**The six-branch wave landed on `main` on 2026-09-06** (`8fb69d5`): T300-C,
T326, T100, T070, PTY-FLAKE and CI-RUNNER-GC. Their rows are deleted per the
rule below. `PROTOCOL_VERSION` **2.15.0** is on `main`; 2.13.0 and 2.14.0 were
spent on the way and the resolved line took the highest. `SCHEMA_VERSION` is
still **4** and migration **`0005` is free** — T326 needed neither.

**The next free values are `PROTOCOL_VERSION` 2.20.0, `SCHEMA_VERSION` 5,
migration `0005`, and ADR-027.**

**Claimed by the 2026-09-08 wave.** `PROTO-W3` takes **2.18.0** for `search.query`
and the one `CoreEvent` variant T080 adds; it spends no `SCHEMA_VERSION` and no
migration, because a search is a read and nothing about one is persisted. Its
seams are the same four `PROTO-W2-GIT` names above, so the two must not be in
flight together — `PROTO-W2-GIT` is already on `main`, which is what makes this
safe. **2.19.0 is reserved for `T310-B`**, recorded now rather than when that
ticket starts: the last handoff said 2.18.0 was "spoken for" by it and never
wrote it down, which is rule 2's failure mode exactly. `T310-A` claims nothing —
it is a new crate with no command, no event and no type on the wire, and
`T100-U` claims nothing either, because `PROTO-W2-GIT` already shipped the wire
it consumes.

**2.18.0 is now spent, not just claimed.** `PROTO-W3` built it together with
T080 in one session (`crates/gritty-core/src/protocol/mod.rs`), and spent
nothing else: no `SCHEMA_VERSION`, no migration, no ADR number. Its seams, for
whoever lands beside it, are `PROTO-W2-GIT`'s four plus two the git wire did not
touch — `crates/gritty-core/src/event/mod.rs` (one `CoreEvent` variant,
`SearchMatchesFound`, plus its `name()` and `operation_id()` arms) and
`crates/gritty-core/src/session/tap.rs` (the exhaustive match that decides what
is persisted). A branch that also adds a `CoreEvent` variant will conflict on
both, and the conflict is the *lucky* outcome: two variants added in different
places merge cleanly and leave `tap.rs` non-exhaustive, which the compiler
catches, while `export.rs` and the generated directory would merge cleanly and
wrongly. Resolve the Rust and re-run `cargo test -p gritty-core` (rule 4).

**`SCHEMA_VERSION` 5 and migration `0005` are still free, and ADR-021 names
both.** `docs/decisions/ADR-021-agent-run-persistence.md` (written 2026-09-08,
`Proposed`) says a persisted run needs `0005_agent_runs.sql` and
`SCHEMA_VERSION` 5. That is a **name, not a claim**: an ADR at `Proposed` has
not been decided, so it must not hold a number a decided ticket may need first.
Whoever writes the migration claims it here, and if that turns out to be some
other ticket, ADR-021's §8 follows the number rather than the other way round.

**ADR-021 fills the gap at 021**, which had been reserved by name in six
documents since T300-C and never written. The next free ADR number is unchanged
at **ADR-027**. While writing it, two rows in `docs/decisions/README.md` were
found duplicated — ADR-024 and ADR-025 each appeared twice, once with a stale
title and a wrong `Accepted` status — which is rule 3b's failure mode surviving
in the index rather than in the filenames. The index now matches the files;
nothing asserts against this, so it is worth looking for again after a wave.

**ADR-026 is taken** by `T233-C` (`docs/decisions/ADR-026-agent-console-tab.md`,
in the working tree at the time of writing). Recorded here after the fact rather
than before, by a different session, which is exactly the hole rule 3b names: the
file was written without the number being claimed, and nothing would have stopped
a second branch writing its own ADR-026 under a different filename and merging
cleanly. Claim the number here first.

What that wave proved, for the next one: claiming three protocol minors by name
before launching turned the only collision that mattered into a one-line
resolution. The one thing not on the claim list — ADR numbers — collided
immediately, which is why rule 3b now exists.

T335 is spending 2.12.0 **and nothing else**. It claimed `SCHEMA_VERSION` 5 and
migration `0005` while it was being planned and has **released both**: permission
grants turned out to belong in memory rather than in the database — a standing
"yes" that outlived the process that took it would be authority nobody remembers
granting — so nothing here is persisted. **`SCHEMA_VERSION` 5 and migration
`0005` are free.** That matters more than a released protocol minor would: a
protocol gap costs nothing, and a migration gap breaks the build.

Its other seams, for whoever lands beside it:
`crates/agents/src/hook.rs` (the state-file format, version 1 → 2),
`crates/gritty-core/src/protocol/export.rs`,
`packages/protocol/src/{client,commands,types}.ts`,
`crates/agents/src/integration.rs` (the installed command line gains
`--ask-timeout`, and the `PermissionRequest` entry gains a `timeout`) and
`docs/architecture/agent-sessions.md`. It adds **no** `CoreEvent` variant — its
commands return a fresh roster, the way T223's do — and touches
`crates/persistence` not at all.

T223's other seams, for whoever lands beside it:
`docs/architecture/{agent-sessions,workstreams,persistence}.md`,
`crates/gritty-core/src/protocol/export.rs`, `crates/persistence/src/store.rs`
and `packages/protocol/src/{client,commands,types}.ts`. It adds **no**
`CoreEvent` variant — its commands return their own fresh snapshot instead — so
the enum #20 and #21 collided on is not one of them.

When a PR merges, delete its rows. This file describes what is *in flight*; git
history describes what happened.

One correction to rule 3 above, from
`crates/persistence/src/migrations.rs`. Migration numbers are claimed like a
version but they are **not** spent like one: a `PROTOCOL_VERSION` gap costs
nothing, and a migration gap breaks the build, because
`migrations_are_numbered_contiguously_from_one` asserts `version == index + 1`.
So a claimed migration number must actually be spent, in order — and whoever
loses a race for one renumbers rather than leaving the hole.

## What the script checks

- open PRs whose base is not `main` — a stack, with the #16–#18 warning;
- two open PRs moving `PROTOCOL_VERSION` or `SCHEMA_VERSION`, and separately
  the worse case of two claiming the **same** value;
- pairs of open PRs touching the same seam, from the actual file lists rather
  than from anything written down here.

It is read-only, needs `gh`, and says so and exits 0 when `gh` is missing or
unauthenticated. It deliberately does not merge anything: ordering is a
judgement, and the judgement is what this file is for.
