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
| [#24](https://github.com/JustGritt/GrittyIDE/pull/24) | the ticket board | 1 | Claims no version constant and no migration number. Its seam is `docs/tasks/README.md`, whose table it makes **generated**: anything else editing that table conflicts, and the resolution is to take this branch's file and re-run `agent-board.py render --index` rather than hand-merge rows. Also rewrites `AGENTS.md` step 1/6 and this file. |

**#22, #23 and #21 landed in that order on 2026-09-06**, which is the order this
table gave them, and the one collision it predicted is the one that happened:
#23's sentence in `docs/tasks/README.md` step 3 met a later branch editing the
same step. Their rows are deleted per the rule below; what they did is in git.

The script found the #23/#21 row itself, from the file lists, before anyone
wrote it down here. That is the intended division of labour: it notices, this
file decides.

### Claimed, so nobody else takes them

| Thing | Value | Branch |
|---|---|---|
| migration `0004` | `0004_agent_links.sql` | T223, specified only |

`PROTOCOL_VERSION` **2.10.0**, `SCHEMA_VERSION` **3** and migration `0003` were
#21's and are now on `main`; the next free values are 2.11.0, 4 and `0004`.

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
