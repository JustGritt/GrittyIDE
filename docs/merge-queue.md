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
| [#27](https://github.com/JustGritt/GrittyIDE/pull/27) | T223 attention + workstream link | 1 | **Spends `PROTOCOL_VERSION` 2.11.0, `SCHEMA_VERSION` 4 and migration `0004_agent_links.sql`.** Rebased onto #25 and #26; the one conflict this table predicted — this file — is resolved, and `docs/tasks/README.md` came through clean. Alone in the queue now. |

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
| `PROTOCOL_VERSION` | **2.11.0** | `t223-agent-attention` |
| `SCHEMA_VERSION` | **4** | `t223-agent-attention` |
| migration `0004` | `0004_agent_links.sql` | `t223-agent-attention` |
| `PROTOCOL_VERSION` | **2.12.0** | `t335-agent-permissions` |

`PROTOCOL_VERSION` **2.10.0**, `SCHEMA_VERSION` **3** and migration `0003` were
#21's and are now on `main`. `t223-agent-attention` is spending 2.11.0, 4 and
`0004`, so the next free values after it are **2.12.0**, **5** and **`0005`**.
The per-lane minors in `execution-board.md` are a planning hint and are already
wrong: 2.11 was reserved there for T070 and T223 took it.

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
