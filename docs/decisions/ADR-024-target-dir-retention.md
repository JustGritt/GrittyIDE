# ADR-024: One `target/` per checkout, swept by budget and age — not a shared `CARGO_TARGET_DIR`

- **Status:** Accepted
- **Date:** 2026-09-06
- **Motivated by:** CI-RUNNER-GC (`docs/tasks/CI-RUNNER-GC-disk-retention.md`),
  itself downstream of CI-COST (self-hosted `macos` runner is the dev Mac) and
  CI-LINUX-DARK (no fallback while it's blocked)

## Context

The self-hosted macOS runner is Alexis' dev Mac, and since CI-COST it is the
only place `macos` CI jobs run. `.github/workflows/ci.yml`'s `macos` job uses
`actions/checkout@v7` with `clean: false`, deliberately, so `target/` survives
between runs and every job isn't a cold Rust build. Nothing has ever pruned it.

`docs/plans/execution-board.md` ("Why lanes should run in separate
worktrees") independently settled on one `git worktree` per active agent
lane, because two sessions running `cargo test -p gritty-core` in one
checkout write `packages/protocol/src/generated/` out from under each other —
"this is not an edit, it is a build," which file leases cannot fix. Cargo's
default (`CARGO_TARGET_DIR` unset) means each worktree gets its **own**
`target/`. This ADR is the answer to the question CI-RUNNER-GC's spec posed
but left open: given that the machine can barely afford *one* `target/`,
should `CARGO_TARGET_DIR` be pointed at one shared directory instead of
letting it multiply per worktree?

## What was measured

All on this machine, 2026-09-06, `worktree-agent-ae118de9a06ec662a`:

- **A `cargo clean` immediately before this task started removed 261,844
  files, 95.9 GiB**, taking free space from 4.0 GiB to 87 GiB. Nothing but a
  human noticing had ever reclaimed that — the ticket's premise, confirmed.
- With that `target/` cold, `git worktree list` showed **six** live
  worktrees. Of the five besides this one, only one (`pty-flake`) had built
  anything (98 MiB); the rest had no `target/` yet. The main checkout's
  `target/` was 2.8 GiB, already regrown from other agents' concurrent work
  in the few minutes since the clean.
- A from-scratch `pnpm check` in this worktree (fmt, clippy
  `--all-targets`, `cargo test --workspace` including doctests) left
  `target/debug/` at 4.5 GiB: `deps` 3.1G, `incremental` 922M, `build` 360M,
  `.fingerprint` 13M.
- `pnpm tauri build --bundles app --ci` (the `macos` job's own release step),
  run immediately after, in the same worktree: see the task file's completion
  report for the final `target/release` figure — gathered the same day, same
  method, real command, not estimated.

Six worktrees each running the measurement above, uncapped, would
extrapolate to several times the disk this machine has. That is the argument
this ADR has to resolve, and it is also the argument for CI-RUNNER-GC's
retention policy existing at all — a GC that keeps each `target/` small is
what makes "one per lane" affordable; it is not, itself, a reason to merge
the directories.

## Decision

**Keep Cargo's default: one `target/` per checkout** — the main tree, and one
per active `.claude/worktrees/*` lane. Do not set `CARGO_TARGET_DIR` to a
directory shared across worktrees or between worktrees and CI. Instead,
`scripts/runner-gc.py` discovers every `target/`-shaped directory this
machine has (via `git worktree list`, validated by Cargo's own
`CACHEDIR.TAG` + `.rustc_info.json`/`.cargo-lock` markers) and sweeps each of
them independently, under its own budget.

## Consequences

- **Disk cost is paid per lane, not once**, and CI-RUNNER-GC's retention
  policy is the thing that makes that affordable: budget- and age-based
  eviction keeps each `target/` bounded (see `scripts/runner-gc.py
  --cache-budget-gb`, default reasoned from the `pnpm check` measurement
  above) instead of letting six lanes each grow toward the 84G+ steady state
  this machine hit before its first-ever `cargo clean`.
- **No cross-lane build serialization.** Cargo takes its own advisory lock
  (`.cargo-lock`) on a target directory for the duration of a build or test
  invocation. A shared `CARGO_TARGET_DIR` would mean one lane's `cargo test`
  blocks, silently, for the full duration of another lane's build — for this
  workspace (Tauri plus its dependency graph), minutes, not seconds. That is
  a responsiveness *and* inspectability failure (a hung-looking build with no
  visible cause) against `AGENTS.md`'s own priority order
  (`correctness → recoverability → inspectability → responsiveness`), on top
  of being the exact `packages/protocol/src/generated/` corruption hazard
  `execution-board.md` created worktrees to avoid in the first place. Sharing
  the target directory would reintroduce, at the build level, the identical
  problem worktrees were adopted to solve at the file level.
- **The GC's job is enumeration, not consolidation.** It must discover every
  `target/` this machine has (main checkout plus every currently-linked
  worktree) rather than assume there is one, and skip (never force through)
  any target directory a live build appears to hold — detected via the same
  `.cargo-lock` flock Cargo itself takes, plus a best-effort process-table
  scan as a backstop.
- **A worktree's `target/` is disposable exactly like the main checkout's.**
  Removing a worktree (`git worktree remove`) already removes its `target/`
  with it; nothing in this decision changes that. The GC only ever prunes
  *inside* a target directory that still exists, never removes a worktree.
- **This machine's practical lane budget is now a disk-arithmetic question,
  not an unbounded one.** `cache_budget_gb × 2 profiles × active worktrees`,
  plus whatever `incremental` and `bundle` add between sweeps, against
  whatever `df` reports free. That arithmetic is exactly why CI-RUNNER-GC's
  floor and budget defaults carry dated measurements instead of guesses —
  see its task file's completion report for the numbers and how they were
  produced.

## Alternatives rejected

**A single shared `CARGO_TARGET_DIR` for every worktree.** Rejected for the
build-lock and generated-output-corruption reasons above. It also does not
actually solve the disk problem — the shared directory still grows without a
retention policy, so it would need the same GC this ADR already requires per
directory, while adding a new failure mode (cross-lane build blocking) that
today's per-worktree default does not have.

**A single shared `CARGO_TARGET_DIR` for CI only, worktrees kept separate.**
Narrower version of the same trade, same lock hazard the moment two `macos`
CI runs or a CI run and a local build overlap on the same directory — see the
task file's "open question" about whether more than one runner process can
be scheduled on this machine, which this ADR does not resolve and which
remains a real risk if it can.

**Do nothing (status quo).** The measured 95.9 GiB in roughly a week, on a
machine with no fallback CI while GitHub-hosted Linux is dark
(CI-LINUX-DARK), is a single point of failure for all CI and all local
development at once. Not tenable once measured; see the task file's
Motivation.
